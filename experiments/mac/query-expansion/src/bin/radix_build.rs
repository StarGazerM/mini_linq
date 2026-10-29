//! Completed GPU RadixRun construction and merge versus native construction.
use cq_mac::{IndexSpec, MacContext, MacRadixRun, Payload};
use cq_storage::RadixRun;
use serde_json::json;
use std::{error::Error, hint::black_box, path::PathBuf, time::Instant};
type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
const SPEC: IndexSpec = IndexSpec {
    key_column: 0,
    payload: Payload::Column(1),
};
fn permute(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^ (x >> 16)
}
fn row(i: usize, n: usize, distribution: &str) -> [u32; 2] {
    let key = match distribution {
        "dense" => (i as u32).wrapping_mul(7919) % (n / 4).max(1) as u32,
        "duplicates" => permute(i as u32 / 32),
        _ => permute(i as u32),
    };
    [key, (i as u32).wrapping_mul(104729)]
}
fn median(samples: &[f64]) -> f64 {
    let mut values = samples.to_vec();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}
fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let result = f();
    (result, start.elapsed().as_secs_f64() * 1000.0)
}
fn check(run: &MacRadixRun, expected: &[(u32, u32)]) {
    assert_eq!(run.len(), expected.len());
    assert!(run.items().iter().copied().eq(expected.iter().map(|r| r.1)));
    let mut group = 0;
    for (i, &(key, _)) in expected.iter().enumerate() {
        if i == 0 || expected[i - 1].0 != key {
            assert_eq!(run.keys()[group], key);
            assert_eq!(run.offsets()[group], i);
            group += 1;
        }
    }
    assert_eq!(group, run.keys().len());
    assert_eq!(run.offsets()[group], expected.len());
    for &key in run.keys().iter().step_by((group / 4096).max(1)) {
        for query in [key, key.saturating_sub(1), key.saturating_add(1)] {
            let lo = expected.partition_point(|r| r.0 < query);
            let hi = expected.partition_point(|r| r.0 <= query);
            assert!(run.lookup(&query).eq(expected[lo..hi].iter().map(|r| r.1)));
        }
    }
}
fn main() -> Result<()> {
    let mut sizes = vec![65_536usize, 1_048_576, 16_777_216];
    let mut runs = 5usize;
    let mut out: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("missing option value")?;
        match flag.as_str() {
            "--sizes" => {
                sizes = value
                    .split(',')
                    .map(str::parse)
                    .collect::<std::result::Result<_, _>>()?
            }
            "--runs" => runs = value.parse()?,
            "--out" => out = Some(value.into()),
            _ => return Err(format!("unknown option {flag}").into()),
        }
    }
    if sizes.is_empty()
        || sizes.iter().any(|&n| n > cq_mac::backend::MAX_ROWS / 2)
        || runs < 3
        || runs.is_multiple_of(2)
    {
        return Err("invalid sizes or runs (odd >=3 required)".into());
    }
    let (context, setup_ms) = timed(MacContext::new);
    let context = context?;
    let mut records = Vec::new();
    println!(
        "rows\tdistribution\tnative_ms\tpacked_gpu_ms\tgrouped_gpu_ms\tmerge_gpu_ms\tspline_points"
    );
    for n in sizes {
        for distribution in ["dense", "uniform", "duplicates"] {
            let (source, ingestion_ms) =
                timed(|| context.relation_u32(n, |i| row(i, n, distribution)));
            let source = source?;
            let pairs = || {
                source
                    .u32_values()
                    .unwrap()
                    .chunks_exact(2)
                    .map(|r| (r[0], r[1]))
            };
            let mut expected = pairs().collect::<Vec<_>>();
            expected.sort_by_key(|r| r.0);
            let mut samples: [Vec<f64>; 3] = std::array::from_fn(|_| Vec::new());
            let mut first_ms = [0.0; 3];
            let mut points = 0;
            for round in 0..runs + 2 {
                for offset in 0..3 {
                    let provider = (round + offset) % 3;
                    let ms = match provider {
                        0 => {
                            let (run, ms) = timed(|| RadixRun::from_pairs(pairs()));
                            assert!(run.items().iter().copied().eq(expected.iter().map(|r| r.1)));
                            black_box(&run);
                            ms
                        }
                        1 => {
                            let (run, ms) = timed(|| context.build(&source, SPEC, None));
                            let run = run?;
                            assert!(run.rows().eq(expected.iter().copied()));
                            ms
                        }
                        _ => {
                            let (run, ms) = timed(|| context.build_radix(&source, SPEC, None));
                            let run = run?;
                            check(&run, &expected);
                            points = run.spline_point_count();
                            ms
                        }
                    };
                    if round == 0 {
                        first_ms[provider] = ms;
                    }
                    if round >= 2 {
                        samples[provider].push(ms);
                    }
                }
            }
            let base = context.build_radix(&source, SPEC, None)?;
            let delta_n = n / 10;
            let delta = context.relation_u32(delta_n, |i| row(i, n, distribution))?;
            let delta_run = context.build_radix(&delta, SPEC, None)?;
            let mut merged_expected = pairs()
                .chain(delta.u32_values()?.chunks_exact(2).map(|r| (r[0], r[1])))
                .collect::<Vec<_>>();
            merged_expected.sort_by_key(|r| r.0);
            let mut merge_samples = Vec::new();
            for round in 0..runs + 2 {
                let (run, ms) = timed(|| context.merge_radix(&base, &delta_run));
                check(&run?, &merged_expected);
                if round >= 2 {
                    merge_samples.push(ms);
                }
            }
            println!(
                "{n}\t{distribution}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{points}",
                median(&samples[0]),
                median(&samples[1]),
                median(&samples[2]),
                median(&merge_samples)
            );
            records.push(json!({"rows":n,"distribution":distribution,"ingestion_ms":ingestion_ms,
                "native_ms":median(&samples[0]),"packed_gpu_ms":median(&samples[1]),"grouped_gpu_ms":median(&samples[2]),
                "samples_ms":samples,"first_ms":first_ms,"spline_points":points,"distinct_keys":base.keys().len(),
                "merge_delta_rows":delta_n,"merge_ms":median(&merge_samples),"merge_samples_ms":merge_samples}));
        }
    }
    let report = json!({"schema":"mccl.radixrun-build.v1","args":std::env::args().collect::<Vec<_>>(),
        "context_setup_ms":setup_ms,"warmups":2,"runs":runs,
        "timing":"completed wall time including allocation, GPU projection/sort/group/spline/directory and wait; same preingested source; validation/drop excluded; rotating constructor order; GPU merge includes grouped reconstruction; not kernel-only bandwidth",
        "native":"native RadixRun<u32,u32> stable sort, grouping and spline from identical source pairs, single CPU thread",
        "validation":"all output payloads and group boundaries plus sampled exact hit/miss ranges; tiny/chunk-boundary/full key lookup cases in cq-mac regressions",
        "results":records});
    if let Some(path) = out {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        serde_json::to_writer_pretty(
            std::io::BufWriter::new(std::fs::File::create(path)?),
            &report,
        )?;
    }
    println!("Exact grouped GPU construction and stable merge validation passed.");
    Ok(())
}
