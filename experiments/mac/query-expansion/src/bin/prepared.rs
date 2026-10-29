//! Matched warm CPU reads of actual GPU/CPU MacRun, GPU/native RadixRun and hash.
//! Construction, ingestion, validation and retained-cursor allocation are untimed.
use cq_mac::{IndexSpec, MacContext, Payload};
use cq_storage::{Build, ExactMembership, HashIndex, IndexRead, RadixRun};
use serde_json::{Value, json};
use std::{error::Error, hint::black_box, path::PathBuf, time::Instant};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
const PROVIDERS: [&str; 5] = [
    "mac_gpu",
    "mac_cpu_radix",
    "native_radix",
    "runtime_hash",
    "gpu_radixrun",
];
const MODES: [&str; 3] = ["membership", "immediate", "retained"];
const SPEC: IndexSpec = IndexSpec {
    key_column: 0,
    payload: Payload::Column(1),
};

struct Options {
    keys: Vec<usize>,
    probes: usize,
    runs: usize,
    out: Option<PathBuf>,
}
impl Options {
    fn parse() -> Result<Self> {
        let mut options = Self {
            keys: vec![16_384, 262_144],
            probes: 65_536,
            runs: 7,
            out: None,
        };
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| format!("missing value for {flag}"))?;
            match flag.as_str() {
                "--keys" => {
                    options.keys = value
                        .split(',')
                        .map(str::parse)
                        .collect::<std::result::Result<_, _>>()?
                }
                "--probes" => options.probes = value.parse()?,
                "--runs" => options.runs = value.parse()?,
                "--out" => options.out = Some(value.into()),
                _ => {
                    return Err(format!(
                        "unknown option {flag}; use --keys N,N --probes N --runs ODD --out FILE"
                    )
                    .into());
                }
            }
        }
        if options.keys.is_empty()
            || options
                .keys
                .iter()
                .any(|n| !n.is_power_of_two() || *n > 262_144 || *n < 8)
        {
            return Err("keys must be powers of two in 8..=262144".into());
        }
        if options.probes == 0 || options.runs < 3 || options.runs.is_multiple_of(2) {
            return Err("probes must be positive; runs must be odd and >=3".into());
        }
        Ok(options)
    }
}

fn mix(mut x: u64) -> u64 {
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58476d1ce4e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}
fn key(rank: u32, n: usize, distribution: &str) -> u32 {
    match distribution {
        "dense" => rank * 2,
        "skewed" if (rank as usize) < n * 7 / 8 => rank * 2,
        "skewed" => 0x8000_0000 | ((rank.wrapping_mul(2_654_435_761) & 0x3fff_ffff) << 1),
        _ => rank.wrapping_mul(2_654_435_761) << 1,
    }
}
fn row(i: usize, n: usize, distribution: &str, fanout: usize) -> [u32; 2] {
    let rank = (i as u32).wrapping_mul(2_654_435_761) & (n as u32 - 1);
    let occurrence = i / n;
    debug_assert!(occurrence < fanout);
    // Nonmonotonic values, with repeated identical occurrences per key.
    [
        key(rank, n, distribution),
        mix(u64::from(rank) ^ ((occurrence / 2) as u64).wrapping_mul(104_729)) as u32,
    ]
}

struct Oracle {
    rows: Vec<(u32, u32)>,
    keys: Vec<u32>,
    offsets: Vec<usize>,
}
impl Oracle {
    fn new(mut rows: Vec<(u32, u32)>) -> Self {
        rows.sort_by_key(|r| r.0);
        let mut keys = Vec::new();
        let mut offsets = Vec::new();
        for (i, &(key, _)) in rows.iter().enumerate() {
            if keys.last() != Some(&key) {
                keys.push(key);
                offsets.push(i);
            }
        }
        offsets.push(rows.len());
        Self {
            rows,
            keys,
            offsets,
        }
    }
    fn values(&self, key: u32) -> &[(u32, u32)] {
        self.keys
            .binary_search(&key)
            .map_or(&[], |i| &self.rows[self.offsets[i]..self.offsets[i + 1]])
    }
    fn check<'a, I>(&self, index: &'a I)
    where
        I: ExactMembership<u32>,
        &'a I: IndexRead<Key = u32, Item = u32>,
    {
        // Every key and every occurrence is checked, not just an aggregate checksum.
        for key in self
            .keys
            .iter()
            .copied()
            .chain(self.keys.iter().map(|k| k | 1))
            .chain([0, 1, u32::MAX - 1, u32::MAX])
        {
            let expected = self.values(key);
            assert_eq!(
                index.contains(&key),
                !expected.is_empty(),
                "membership {key}"
            );
            assert!(
                index.lookup(&key).eq(expected.iter().map(|r| r.1)),
                "payload order/multiplicity {key}"
            );
        }
    }
    fn queries(&self, n: usize, hit_percent: usize) -> Vec<u32> {
        let mut queries = (0..n)
            .map(|i| {
                let rank = mix(i as u64 + 42) as usize % self.keys.len();
                self.keys[rank] | u32::from(i % 100 >= hit_percent)
            })
            .collect::<Vec<_>>();
        for i in (1..n).rev() {
            queries.swap(i, mix(i as u64 + 701) as usize % (i + 1));
        }
        queries
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Answer {
    count: u64,
    sum: u64,
}
impl Answer {
    fn add(&mut self, value: u32) {
        self.count += 1;
        self.sum = self.sum.wrapping_add(u64::from(value));
    }
}

fn sample<'a, I>(index: &'a I, queries: &[u32], mode: usize) -> (f64, Answer)
where
    I: ExactMembership<u32>,
    &'a I: IndexRead<Key = u32, Item = u32>,
{
    // Cursor storage is reserved outside timing; locating/storing/draining is timed.
    let mut cursors = Vec::with_capacity(if mode == 2 { queries.len() } else { 0 });
    let index = black_box(index);
    let queries = black_box(queries);
    let start = Instant::now();
    let mut answer = Answer::default();
    match mode {
        0 => answer.count = queries.iter().filter(|k| index.contains(k)).count() as u64,
        1 => {
            for key in queries {
                for value in index.lookup(key) {
                    answer.add(value);
                }
            }
        }
        2 => {
            cursors.extend(queries.iter().map(|key| index.lookup(key)));
            for cursor in cursors.drain(..) {
                for value in cursor {
                    answer.add(value);
                }
            }
        }
        _ => unreachable!(),
    }
    let answer = black_box(answer);
    (start.elapsed().as_secs_f64() * 1000.0, answer)
}
fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[sorted.len() / 2]
}

fn boundaries(context: &MacContext) -> Result<()> {
    let rows = [
        [u32::MAX, 7],
        [0, 9],
        [u32::MAX, 7],
        [1, 2],
        [u32::MAX, 3],
        [0, 1],
    ];
    for count in [0, 1, rows.len()] {
        let source = context.relation_u32(count, |i| rows[i])?;
        let oracle = Oracle::new(rows[..count].iter().map(|r| (r[0], r[1])).collect());
        let gpu = context.build(&source, SPEC, None)?;
        let cpu = context.build_on_cpu_radix(&source, SPEC, None)?;
        let gpu_radix = context.build_radix(&source, SPEC, None)?;
        let radix = RadixRun::from_pairs(rows[..count].iter().map(|r| (r[0], r[1])));
        let hash = HashIndex::build(rows[..count].iter().map(|r| (r[0], r[1])));
        oracle.check(&gpu);
        oracle.check(&cpu);
        oracle.check(&radix);
        oracle.check(&hash);
        oracle.check(&gpu_radix);
    }
    Ok(())
}

fn main() -> Result<()> {
    let options = Options::parse()?;
    let start = Instant::now();
    let context = MacContext::new()?;
    let setup_ms = start.elapsed().as_secs_f64() * 1000.0;
    boundaries(&context)?;
    let mut records: Vec<Value> = Vec::new();
    println!("distribution\tkeys\trows\thit_percent\tmode\tprovider\tmedian_ms\tns_per_probe");
    for &n in &options.keys {
        for distribution in ["dense", "sparse", "skewed", "duplicate_heavy"] {
            let fanout = if distribution == "duplicate_heavy" {
                32
            } else {
                4
            };
            let start = Instant::now();
            let source = context.relation_u32(n * fanout, |i| row(i, n, distribution, fanout))?;
            let ingestion_ms = start.elapsed().as_secs_f64() * 1000.0;
            let pairs = || {
                source
                    .u32_values()
                    .unwrap()
                    .chunks_exact(2)
                    .map(|r| (r[0], r[1]))
            };
            let oracle = Oracle::new(pairs().collect());
            assert_eq!(oracle.keys.len(), n);
            let gpu = context.build(&source, SPEC, None)?;
            let cpu = context.build_on_cpu_radix(&source, SPEC, None)?;
            let gpu_radix = context.build_radix(&source, SPEC, None)?;
            let radix = RadixRun::from_pairs(pairs());
            let hash = HashIndex::build(pairs());
            assert_eq!(hash.len(), n); // Force lazy construction before validation/timing.
            oracle.check(&gpu);
            oracle.check(&cpu);
            oracle.check(&radix);
            oracle.check(&hash); // Also initializes native lazy membership before measurement.
            oracle.check(&gpu_radix);
            let mut occupancy = [0usize; 4096];
            for &(key, _) in &oracle.rows {
                occupancy[(key >> 20) as usize] += 1;
            }
            for hit_percent in [10, 50, 90] {
                let queries = oracle.queries(options.probes, hit_percent);
                let mut expected = Answer::default();
                let mut hits = 0;
                for &key in &queries {
                    let values = oracle.values(key);
                    hits += u64::from(!values.is_empty());
                    for &(_, value) in values {
                        expected.add(value);
                    }
                }
                for (mode, mode_name) in MODES.iter().enumerate() {
                    let wanted = if mode == 0 {
                        Answer {
                            count: hits,
                            sum: 0,
                        }
                    } else {
                        expected
                    };
                    let mut samples: [Vec<f64>; PROVIDERS.len()] =
                        std::array::from_fn(|_| Vec::with_capacity(options.runs));
                    let mut execution_order = Vec::new();
                    for round in 0..options.runs + 2 {
                        let mut order = Vec::new();
                        for offset in 0..PROVIDERS.len() {
                            let provider = (offset + round) % PROVIDERS.len();
                            let (ms, actual) = match provider {
                                0 => sample(&gpu, &queries, mode),
                                1 => sample(&cpu, &queries, mode),
                                2 => sample(&radix, &queries, mode),
                                3 => sample(&hash, &queries, mode),
                                4 => sample(&gpu_radix, &queries, mode),
                                _ => unreachable!(),
                            };
                            assert_eq!(
                                actual, wanted,
                                "{distribution}/{mode_name}/{}",
                                PROVIDERS[provider]
                            );
                            if round >= 2 {
                                samples[provider].push(ms);
                            }
                            order.push(PROVIDERS[provider]);
                        }
                        if round >= 2 {
                            execution_order.push(order);
                        }
                    }
                    for (provider, values) in samples.iter().enumerate() {
                        let ms = median(values);
                        println!(
                            "{distribution}\t{n}\t{}\t{hit_percent}\t{mode_name}\t{}\t{ms:.6}\t{:.3}",
                            n * fanout,
                            PROVIDERS[provider],
                            ms * 1e6 / queries.len() as f64
                        );
                        records.push(json!({"distribution": distribution, "distinct_keys": n,
                            "rows": n * fanout, "fanout": fanout, "requested_hit_percent": hit_percent,
                            "probes": queries.len(), "hits": hits, "mode": mode_name,
                            "provider": PROVIDERS[provider], "samples_ms": values, "median_ms": ms,
                            "ns_per_probe": ms * 1e6 / queries.len() as f64,
                            "result_count": wanted.count, "result_checksum": wanted.sum,
                            "occupied_prefix_buckets": occupancy.iter().filter(|&&n| n > 0).count(),
                            "max_prefix_bucket_rows": occupancy.iter().max().unwrap(),
                            "source_ingestion_ms_excluded": ingestion_ms,
                            "gpu_spline_points": gpu_radix.spline_point_count(),
                            "execution_order": execution_order}));
                    }
                }
            }
        }
    }
    let report = json!({"schema": "mccl.prepared-readers.v2", "architecture": std::env::consts::ARCH,
        "context_setup_ms_excluded": setup_ms, "runs": options.runs, "warmups": 2,
        "args": std::env::args().collect::<Vec<_>>(),
        "timing": "single-thread CPU warm prepared reads; ingestion, all construction, full exact validation, lazy membership initialization and retained cursor reservation excluded; retained lookup/store/drain included; checksum enumeration, not output materialization",
        "source": "same immutable shared u32 key/payload relation; GPU grouped RadixRun uses the native CPU read implementation; packed GPU/CPU MacRun controls retained",
        "validation": "empty/singleton/u32-extrema boundary cases, complete stable occurrence sequences for every distinct key, misses and every timed count/checksum",
        "limits": "synthetic power-of-two key counts; randomized runtime hash seed; all five indexes retained; no CPU/GPU overlap; no cold latency, construction speedup or full-query claim",
        "results": records});
    if let Some(path) = options.out {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::io::BufWriter::new(std::fs::File::create(&path)?);
        serde_json::to_writer_pretty(file, &report)?;
        println!("Raw prepared-reader samples: {}", path.display());
    }
    println!("Exact prepared-reader validation passed for every provider and workload.");
    Ok(())
}
