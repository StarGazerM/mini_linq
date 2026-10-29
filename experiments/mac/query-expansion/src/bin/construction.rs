//! Same H09 code, row-ID payloads, shared buffers, directory and CPU reads.
//! Only the constructor callback changes between timed executions.
use cq_mac::{IndexSpec, MacContext, MacError, MacRun, Relation};
use mac_query_expansion::{ManualResult, fixtures, h09, manual_h09};
use serde_json::{Value, json};
use std::{error::Error, fs, path::PathBuf, time::Instant};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
type Constructor<'a> = dyn FnMut(&MacContext, &Relation, IndexSpec, Option<&Relation>) -> Result<MacRun, MacError>
    + 'a;

struct Options {
    sizes: Vec<usize>,
    runs: usize,
    out: Option<PathBuf>,
    cpu_radix: bool,
}

impl Options {
    fn parse() -> Result<Self> {
        let mut result = Self {
            sizes: vec![16_387, 65_539, 262_147, 1_048_579, 4_194_307],
            runs: 9,
            out: None,
            cpu_radix: false,
        };
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| format!("missing value after {flag}"))?;
            match flag.as_str() {
                "--sizes" => {
                    result.sizes = value
                        .split(',')
                        .map(str::parse)
                        .collect::<std::result::Result<_, _>>()?
                }
                "--runs" => result.runs = value.parse()?,
                "--out" => result.out = Some(value.into()),
                "--cpu" => {
                    result.cpu_radix = match value.as_str() {
                        "radix" => true,
                        "comparison" => false,
                        _ => return Err("--cpu must be radix or comparison".into()),
                    };
                }
                _ => {
                    return Err(format!(
                        "unknown option {flag}; use --sizes N,N --runs ODD --cpu comparison|radix --out FILE"
                    )
                    .into());
                }
            }
        }
        if result.runs < 3 || result.runs % 2 == 0 {
            return Err("--runs must be odd and at least three".into());
        }
        if result.sizes.is_empty() || result.sizes.iter().any(|&n| n > cq_mac::backend::MAX_ROWS) {
            return Err("sizes must be a nonempty list within the explicit Mac row domain".into());
        }
        Ok(result)
    }
}

struct Sample {
    total_ms: f64,
    build_ms: f64,
    remaining_ms: f64,
}

impl Sample {
    fn json(&self, pair: usize, first: bool) -> Value {
        json!({"pair": pair, "first": first, "total_ms": self.total_ms,
            "build_ms": self.build_ms,
            "remaining_ms": self.remaining_ms})
    }
}

fn run(
    context: &MacContext,
    data: &h09::Data,
    prepared: &manual_h09::Prepared,
    constructor: &mut Constructor<'_>,
) -> Result<(Sample, ManualResult<h09::Answer>)> {
    let start = Instant::now();
    // Dynamic dispatch keeps one common, non-generic query execution body.
    let execution = manual_h09::execute(context, data, prepared, constructor)?;
    let total = start.elapsed();
    let build = execution.construction;
    let sample = Sample {
        total_ms: total.as_secs_f64() * 1000.0,
        build_ms: build.as_secs_f64() * 1000.0,
        remaining_ms: total.saturating_sub(build).as_secs_f64() * 1000.0,
    };
    Ok((sample, execution.result))
}

fn counts(result: &ManualResult<h09::Answer>) -> (usize, usize, usize, usize, usize) {
    (
        result.index_builds,
        result.indexed_rows,
        result.index_probes,
        result.payload_gathers,
        result.witnesses,
    )
}

fn check(reference: &ManualResult<h09::Answer>, actual: &ManualResult<h09::Answer>) {
    // Also check identical enumeration order and pre-canon multiplicity, not
    // only set cardinality. All checking and output destruction are untimed.
    assert_eq!(actual.answers, reference.answers);
    assert_eq!(counts(actual), counts(reference));
}

fn check_compiled(data: &h09::Data, actual: &ManualResult<h09::Answer>) {
    let mut expected = h09::compiled(data);
    expected.sort_unstable();
    let mut values = actual.answers.clone();
    values.sort_unstable();
    assert_eq!(values, expected, "complete authored H09 answers differ");
}

fn verify_boundaries(
    context: &MacContext,
    cpu: &mut Constructor<'_>,
    gpu: &mut Constructor<'_>,
) -> Result<()> {
    let boundary = fixtures::h09_boundaries();
    let mut empty = boundary.clone();
    empty.lineitem.clear();
    let mut no_green = boundary.clone();
    no_green
        .part
        .iter_mut()
        .for_each(|(_, name)| *name = "red".into());
    for (label, data) in [
        ("duplicates/extrema", &boundary),
        ("empty", &empty),
        ("no-green", &no_green),
    ] {
        let prepared = manual_h09::Prepared::new(context, data)?;
        let (_, cpu) = run(context, data, &prepared, cpu)?;
        let (_, gpu) = run(context, data, &prepared, gpu)?;
        check_compiled(data, &cpu);
        check(&cpu, &gpu);
        if label == "duplicates/extrema" {
            assert_eq!(cpu.witnesses, 68);
            assert_eq!(cpu.answers.len(), 4);
        }
        if label == "no-green" {
            assert_eq!(cpu.index_builds, 0);
            assert_eq!(gpu.index_builds, 0);
        }
    }
    println!("Boundary, empty and demand-driven construction checks passed for both constructors.");
    Ok(())
}

fn summary(samples: &[Sample], project: impl Fn(&Sample) -> f64) -> Value {
    let mut values = samples.iter().map(project).collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    json!({"min": values[0], "p25": values[values.len()/4],
        "median": values[values.len()/2], "p75": values[values.len()*3/4], "max": values[values.len()-1]})
}

fn median(samples: &[Sample], project: impl Fn(&Sample) -> f64) -> f64 {
    let mut values = samples.iter().map(project).collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn mode_summary(samples: &[Sample]) -> Value {
    json!({"total_ms": summary(samples, |s| s.total_ms),
        "build_ms": summary(samples, |s| s.build_ms),
        "remaining_ms": summary(samples, |s| s.remaining_ms)})
}

fn main() -> Result<()> {
    let options = Options::parse()?;
    let mut gpu = MacContext::build;
    let mut cpu =
        |context: &MacContext, source: &Relation, spec: IndexSpec, selection: Option<&Relation>| {
            if options.cpu_radix {
                context.build_on_cpu_radix(source, spec, selection)
            } else {
                context.build_on_cpu(source, spec, selection)
            }
        };
    let cpu_description = if options.cpu_radix {
        "single-thread stable byte radix, constant bytes skipped + radix12 directory"
    } else {
        "stable slice::sort_by_key comparison sort + radix12 directory"
    };
    let start = Instant::now();
    let context = MacContext::new()?;
    let setup_ms = start.elapsed().as_secs_f64() * 1000.0;
    verify_boundaries(&context, &mut cpu, &mut gpu)?;
    println!("CPU constructor: {cpu_description}; GPU: MCCL with MLX-owned shared storage");
    println!(
        "rows\tanswers\tingestion_ms\tsource_bytes\tcpu_total_ms\tgpu_total_ms\tcpu_build_ms\tgpu_build_ms\tcpu_remaining_ms\tgpu_remaining_ms"
    );
    let mut records = Vec::new();
    for n in options.sizes {
        let data = fixtures::h09_scaled(n);
        let prepared = manual_h09::Prepared::new(&context, &data)?;
        let ingestion_ms = prepared.ingestion.as_secs_f64() * 1000.0;
        // Two executions per constructor precede steady-state samples. The same
        // numeric source remains live across every CPU and GPU construction.
        let (first_cpu, reference) = run(&context, &data, &prepared, &mut cpu)?;
        check_compiled(&data, &reference);
        let (first_gpu, warm_gpu) = run(&context, &data, &prepared, &mut gpu)?;
        check(&reference, &warm_gpu);
        drop(warm_gpu);
        let (_, warm) = run(&context, &data, &prepared, &mut gpu)?;
        check(&reference, &warm);
        drop(warm);
        let (_, warm) = run(&context, &data, &prepared, &mut cpu)?;
        check(&reference, &warm);
        drop(warm);
        let mut cpu_samples = Vec::with_capacity(options.runs);
        let mut gpu_samples = Vec::with_capacity(options.runs);
        for pair in 0..options.runs {
            let cpu_first = pair % 2 == 0;
            for is_cpu in [cpu_first, !cpu_first] {
                let (sample, actual) = if is_cpu {
                    run(&context, &data, &prepared, &mut cpu)?
                } else {
                    run(&context, &data, &prepared, &mut gpu)?
                };
                check(&reference, &actual);
                if is_cpu {
                    cpu_samples.push(sample);
                } else {
                    gpu_samples.push(sample);
                }
            }
        }
        println!(
            "{n}\t{}\t{ingestion_ms:.3}\t{}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}",
            reference.answers.len(),
            prepared.source_bytes(),
            median(&cpu_samples, |s| s.total_ms),
            median(&gpu_samples, |s| s.total_ms),
            median(&cpu_samples, |s| s.build_ms),
            median(&gpu_samples, |s| s.build_ms),
            median(&cpu_samples, |s| s.remaining_ms),
            median(&gpu_samples, |s| s.remaining_ms)
        );
        records.push(json!({"lineitem_rows": n, "answers": reference.answers.len(),
            "ingestion_ms": ingestion_ms, "source_bytes": prepared.source_bytes(),
            "first_cpu_query_ms": first_cpu.total_ms, "first_gpu_query_ms": first_gpu.total_ms,
            "ingestion_plus_first_cpu_query_ms": ingestion_ms + first_cpu.total_ms,
            "ingestion_plus_first_gpu_query_ms": ingestion_ms + first_gpu.total_ms,
            "indexed_rows": reference.indexed_rows, "probes": reference.index_probes,
            "payload_gathers": reference.payload_gathers, "witnesses": reference.witnesses,
            "cpu": mode_summary(&cpu_samples), "gpu": mode_summary(&gpu_samples),
            "cpu_samples": cpu_samples.iter().enumerate().map(|(i,s)| s.json(i, i%2==0)).collect::<Vec<_>>(),
            "gpu_samples": gpu_samples.iter().enumerate().map(|(i,s)| s.json(i, i%2!=0)).collect::<Vec<_>>() }));
    }
    let report = json!({"schema": "mccl.query-construction.v2", "query": "authored retail H09 positive join core", "data": "deterministic synthetic fixtures, not dbgen",
        "cpu_constructor": cpu_description, "gpu_constructor": "MCCL stable 8-bit radix passes + radix12 directory",
        "shared_representation": "MacRun: immutable MLX-owned shared arrays, u32 key/source-rowID, identical CPU lookup",
        "warmups_per_mode": 2, "samples_per_mode": options.runs, "order": "alternating paired CPU-first/GPU-first",
        "context_setup_ms_excluded": setup_ms, "architecture": std::env::consts::ARCH,
        "timing": "repeated whole query includes shared-source construction/allocation/wait, CPU residual indexes/joins/dedup and temporary destruction; one-time numeric relation ingestion reported separately; excludes fixture generation, context setup, validation and output destruction",
        "first_query_totals": "measured ingestion plus each constructor's first query duration; sums, not contiguous end-to-end intervals; CPU runs first and neither is a physical cold-start claim",
        "source_bytes": "retained numeric relation logical bytes, not measured traffic or physical residency",
        "remaining_ms": "total minus constructor; includes CPU joins, residual indexes and temporary destruction, not pure probe latency",
        "limits": "single-process warmed sweep; normal runtime hash randomization and OS contention remain; not a fastest-CPU claim or portable crossover threshold",
        "results": records});
    if let Some(path) = options.out {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let mut writer = std::io::BufWriter::new(fs::File::create(&path)?);
        serde_json::to_writer_pretty(&mut writer, &report)?;
        std::io::Write::flush(&mut writer)?;
        println!("Raw paired samples and summaries: {}", path.display());
    }
    println!(
        "Every sample matched complete authored answers and identical CPU/GPU witness/order counts."
    );
    Ok(())
}
