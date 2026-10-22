//! Reproducible build-only benchmark, with CSV on stdout and diagnostics on stderr.
//! Example: nsys profile <binary> --case zipf1.5 --n 100000000 --levels 3 --repeat 5
//! CPU generation, input uploads, output downloads and checksums are outside both
//! timers. Wall time includes build submission and waiting for its completion;
//! CUDA time spans the build on its stream (including any host submission gaps).
//! Each measured build reads every public output after timing, one column at a
//! time, to bound host memory. Checksums consume outputs, not a CPU sort oracle.
//! Warmups use the same pool and inputs but are not emitted as measured samples.
use cuda_core::{CudaContext, CudaStream};
use gpu_blocked_run::{read_stats, Buffer, MemoryResource, Run};
use rccl_memory::{ContiguousDeviceRange, CudaMemoryResource, PoolMemoryResource};
use std::{error::Error, io, time::Instant};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
const CASES: [&str; 6] = [
    "sorted",
    "reverse",
    "uniform",
    "extreme-skew",
    "zipf1",
    "zipf1.5",
];

struct Options {
    sizes: Vec<usize>,
    cases: Vec<String>,
    levels: Vec<usize>,
    repeat: usize,
    warmup: usize,
    seed: u64,
    domain: usize,
}

fn invalid(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(io::Error::new(io::ErrorKind::InvalidInput, message.into()))
}

fn parse_list<T: std::str::FromStr>(value: &str) -> Result<Vec<T>> {
    value
        .split(',')
        .map(|part| {
            part.parse()
                .map_err(|_| invalid(format!("invalid number: {part}")))
        })
        .collect()
}

fn options() -> Result<Option<Options>> {
    let mut options = Options {
        sizes: vec![0, 1, 31, 257, 1_000_000, 10_000_000, 100_000_000],
        cases: CASES.iter().map(|s| (*s).to_owned()).collect(),
        levels: vec![1, 2, 3],
        repeat: 3,
        warmup: 1,
        seed: 0x0123_4567_89ab_cdef,
        domain: 1_000_000,
    };
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        if flag == "--help" || flag == "-h" {
            println!("sort-bench [--case sorted|reverse|uniform|extreme-skew|zipf1|zipf1.5|all] [--n N[,N...]] [--levels L[,L...]] [--repeat N] [--warmup N] [--seed U64] [--domain N]\nDefaults: all cases; n=0,1,31,257,1000000,10000000,100000000; levels=1,2,3; repeat=3; warmup=1; domain=1000000. Seed is decimal. CSV contains individual measured samples, no estimated timings.");
            return Ok(None);
        }
        let value = args
            .next()
            .ok_or_else(|| invalid(format!("missing value for {flag}")))?;
        match flag.as_str() {
            "--case" => {
                options.cases = if value == "all" {
                    CASES.iter().map(|s| (*s).to_owned()).collect()
                } else {
                    value.split(',').map(str::to_owned).collect()
                }
            }
            "--n" => options.sizes = parse_list(&value)?,
            "--levels" => options.levels = parse_list(&value)?,
            "--repeat" => options.repeat = value.parse()?,
            "--warmup" => options.warmup = value.parse()?,
            "--seed" => options.seed = value.parse()?,
            "--domain" => options.domain = value.parse()?,
            _ => return Err(invalid(format!("unknown option: {flag}"))),
        }
    }
    if options
        .cases
        .iter()
        .any(|case| !CASES.contains(&case.as_str()))
    {
        return Err(invalid("unknown case; use --help"));
    }
    if options.sizes.iter().any(|&n| n > u32::MAX as usize)
        || options.levels.contains(&0)
        || options.repeat == 0
        || options.domain == 0
        || options.domain > u32::MAX as usize
    {
        return Err(invalid(
            "n must fit u32; levels and repeat must be positive; domain must be in 1..=u32::MAX",
        ));
    }
    Ok(Some(options))
}

// SplitMix64 is fixed here rather than relying on a dependency's RNG version.
fn random(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

// Walker alias sampling keeps 100M Zipf generation O(n), not O(n log domain).
struct Zipf {
    probability: Vec<f64>,
    alias: Vec<usize>,
}
impl Zipf {
    fn new(domain: usize, exponent: f64) -> Self {
        let mut probability: Vec<f64> = (1..=domain)
            .map(|rank| (rank as f64).powf(-exponent))
            .collect();
        let total: f64 = probability.iter().sum();
        for p in &mut probability {
            *p *= domain as f64 / total;
        }
        let mut small = Vec::new();
        let mut large = Vec::new();
        for (index, &p) in probability.iter().enumerate() {
            if p < 1.0 {
                small.push(index);
            } else {
                large.push(index);
            }
        }
        let mut alias: Vec<usize> = (0..domain).collect();
        while !small.is_empty() && !large.is_empty() {
            let low = small.pop().unwrap();
            let high = large.pop().unwrap();
            alias[low] = high;
            probability[high] = probability[high] + probability[low] - 1.0;
            if probability[high] < 1.0 {
                small.push(high);
            } else {
                large.push(high);
            }
        }
        for index in small.into_iter().chain(large) {
            probability[index] = 1.0;
        }
        Self { probability, alias }
    }
    fn sample(&self, state: &mut u64) -> usize {
        let column = (random(state) % self.probability.len() as u64) as usize;
        let coin = (random(state) >> 11) as f64 / (1u64 << 53) as f64;
        if coin < self.probability[column] {
            column
        } else {
            self.alias[column]
        }
    }
}

fn column(n: usize, level: usize, case: &str, options: &Options, zipf: Option<&Zipf>) -> Vec<i64> {
    let mut state = options
        .seed
        .wrapping_add((level as u64).wrapping_mul(0xd1b5_4a32_d192_ed03));
    (0..n)
        .map(|row| {
            let key = match case {
                // Every column is monotone, so the whole tuple has the named order.
                "sorted" => row as u64 * options.domain as u64 / n as u64,
                "reverse" => (n - 1 - row) as u64 * options.domain as u64 / n as u64,
                "uniform" => random(&mut state) % options.domain as u64,
                // 99.99% in key zero, with the remainder spread over the domain.
                "extreme-skew" => {
                    if random(&mut state) % 10_000 == 0 {
                        random(&mut state) % options.domain as u64
                    } else {
                        0
                    }
                }
                "zipf1" | "zipf1.5" => zipf.unwrap().sample(&mut state) as u64,
                _ => unreachable!(),
            };
            key as i64
        })
        .collect()
}

fn mix(checksum: &mut u64, value: u64) {
    *checksum = checksum.rotate_left(7).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ value;
}

fn consume(run: &Run<'_>) -> Result<u64> {
    let mut checksum = 0u64;
    for value in run.keys.to_host_vec()? {
        mix(&mut checksum, value as u64);
    }
    for payload in &run.payloads {
        for value in payload.to_host_vec()? {
            mix(&mut checksum, value as u64);
        }
    }
    for value in run.offsets.to_host_vec()? {
        mix(&mut checksum, value as u64);
    }
    for value in run.unique_keys.to_host_vec()? {
        mix(&mut checksum, value as u64);
    }
    for value in run.fences.to_host_vec()? {
        mix(&mut checksum, value as u64);
    }
    for value in run.bits.to_host_vec()? {
        mix(&mut checksum, value);
    }
    for value in run.prefix.to_host_vec()? {
        mix(&mut checksum, value as u64);
    }
    Ok(std::hint::black_box(checksum))
}

fn bench(
    ctx: &CudaContext,
    stream: &CudaStream,
    memory: &MemoryResource,
    options: &Options,
    n: usize,
    case: &str,
    levels: usize,
) -> Result<()> {
    eprintln!("generating case={case} n={n} levels={levels}");
    let zipf = match case {
        "zipf1" => Some(Zipf::new(options.domain, 1.0)),
        "zipf1.5" => Some(Zipf::new(options.domain, 1.5)),
        _ => None,
    };
    // Upload and release one host column at a time, including at 100M rows.
    let mut keys = Vec::with_capacity(levels);
    for level in 0..levels {
        let host = column(n, level, case, options, zipf.as_ref());
        keys.push(Buffer::from_host_in(memory, stream, &host)?);
        stream.synchronize()?;
    }
    drop(zipf);
    let host_rows: Vec<i64> = (0..n).map(|row| row as i64).collect();
    let rows = Buffer::from_host_in(memory, stream, &host_rows)?;
    stream.synchronize()?;
    drop(host_rows);
    let key_refs: Vec<&dyn ContiguousDeviceRange<i64>> = keys
        .iter()
        .map(|key| key as &dyn ContiguousDeviceRange<i64>)
        .collect();
    let payloads: [&dyn ContiguousDeviceRange<i64>; 1] = [&rows];
    let domains = vec![(0, options.domain as i64 - 1); levels];
    let mut expected_checksum = None;
    for iteration in 0..options.warmup.saturating_add(options.repeat) {
        stream.synchronize()?;
        let start =
            stream.record_event(Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?;
        let wall_start = Instant::now();
        let run = if levels == 1 {
            Run::build(
                ctx,
                stream,
                memory,
                key_refs[0],
                &payloads,
                domains[0],
                None,
            )?
        } else {
            Run::build_prefix(ctx, stream, memory, &key_refs, &payloads, &domains, None)?
        };
        let end = stream.record_event(Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))?;
        end.synchronize()?;
        let wall_ms = wall_start.elapsed().as_secs_f64() * 1000.0;
        let cuda_ms = start.elapsed_ms(&end)?;
        // No host metadata/output readback is permitted before the end event.
        let stats = read_stats(memory, stream, &[&run])?[0];
        if stats.rows as usize != n {
            return Err(invalid("output row count differs from input"));
        }
        let checksum = consume(&run)?;
        if expected_checksum.is_some_and(|expected| expected != checksum) {
            return Err(invalid("output checksum changed between identical builds"));
        }
        expected_checksum = Some(checksum);
        if iteration >= options.warmup {
            println!(
                "{case},{n},{levels},{},{},{},{},{cuda_ms:.6},{wall_ms:.6},{},{},{},{},{checksum}",
                options.domain,
                options.seed,
                options.warmup,
                iteration - options.warmup,
                stats.rows,
                stats.unique,
                stats.max_count,
                stats.invalid
            );
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let Some(options) = options()? else {
        return Ok(());
    };
    let context = CudaContext::new(0)?;
    let stream = context.new_stream()?;
    gpu_blocked_run::prepare_kernels(&context)?;
    println!("case,n,levels,domain,seed,warmup,sample,cuda_ms,wall_ms,rows,unique,max_count,invalid,checksum");
    for &n in &options.sizes {
        for case in &options.cases {
            for &levels in &options.levels {
                // Reuse allocations within a case, but release cached GPU storage
                // between cases so the large sweep does not accumulate pool bins.
                let memory = MemoryResource::new(PoolMemoryResource::new(
                    CudaMemoryResource::new(context.clone()),
                    32 << 20,
                    0,
                )?);
                bench(&context, &stream, &memory, &options, n, case, levels)?;
                stream.synchronize()?;
            }
        }
    }
    Ok(())
}
