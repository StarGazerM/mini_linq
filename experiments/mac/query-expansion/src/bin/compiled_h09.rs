//! Actual compiled H09: same source-bound host/read path, CPU versus GPU build.
use cq_mac::MacContext;
use mac_query_expansion::{
    compiled_h09::{self, Construction},
    fixtures, h09,
    manual_h09::Prepared,
};
use serde_json::{Value, json};
use std::{error::Error, path::PathBuf, time::Instant};
type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

struct Options {
    sizes: Vec<usize>,
    runs: usize,
    out: Option<PathBuf>,
}
impl Options {
    fn parse() -> Result<Self> {
        let mut options = Self {
            sizes: vec![65_539, 1_048_579, 4_194_307],
            runs: 7,
            out: None,
        };
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| format!("missing value for {flag}"))?;
            match flag.as_str() {
                "--sizes" => {
                    options.sizes = value
                        .split(',')
                        .map(str::parse)
                        .collect::<std::result::Result<_, _>>()?
                }
                "--runs" => options.runs = value.parse()?,
                "--out" => options.out = Some(value.into()),
                _ => return Err(format!("unknown option {flag}").into()),
            }
        }
        if options.runs < 3
            || options.runs.is_multiple_of(2)
            || options.sizes.is_empty()
            || options.sizes.iter().any(|&n| n > cq_mac::backend::MAX_ROWS)
        {
            return Err(
                "sizes must fit Mac row domain; runs must be odd and at least three".into(),
            );
        }
        Ok(options)
    }
}

struct Sample {
    ingestion_ms: f64,
    build_ms: f64,
    query_ms: f64,
    total_ms: f64,
}
impl Sample {
    fn json(&self, round: usize, first_in_pair: bool) -> Value {
        json!({"round":round,"first_in_pair":first_in_pair,"ingestion_ms":self.ingestion_ms,
            "build_ms":self.build_ms,"query_ms":self.query_ms,"total_ms":self.total_ms})
    }
}
fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
fn run(
    context: &MacContext,
    data: &h09::Data,
    source: Option<&Prepared>,
    mode: Construction,
) -> Result<(Sample, Vec<h09::Answer>)> {
    let total = Instant::now();
    let owned_source;
    let ingestion_ms;
    let source = if let Some(source) = source {
        ingestion_ms = 0.0;
        source
    } else {
        let start = Instant::now();
        owned_source = Prepared::new(context, data)?;
        ingestion_ms = ms(start);
        &owned_source
    };
    let start = Instant::now();
    let index = compiled_h09::build(context, source, mode)?;
    let build_ms = ms(start);
    let start = Instant::now();
    let answers = compiled_h09::execute(data, &index)?;
    let query_ms = ms(start);
    let total_ms = ms(total);
    // Prepared index/source destruction and validation are outside the interval.
    Ok((
        Sample {
            ingestion_ms,
            build_ms,
            query_ms,
            total_ms,
        },
        answers,
    ))
}
fn validate(mut actual: Vec<h09::Answer>, expected: &[h09::Answer]) {
    actual.sort_unstable();
    assert_eq!(actual, expected, "complete compiled H09 answers differ");
}
fn oracle(data: &h09::Data) -> Vec<h09::Answer> {
    let mut answers = h09::compiled(data);
    answers.sort_unstable();
    answers
}
fn median(samples: &[Sample], f: impl Fn(&Sample) -> f64) -> f64 {
    let mut values = samples.iter().map(f).collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}
fn summary(samples: &[Sample]) -> Value {
    json!({"ingestion_ms":median(samples,|s|s.ingestion_ms),"build_ms":median(samples,|s|s.build_ms),
        "query_ms":median(samples,|s|s.query_ms),"total_ms":median(samples,|s|s.total_ms)})
}
fn boundaries(context: &MacContext) -> Result<()> {
    let data = fixtures::h09_boundaries();
    let mut empty = data.clone();
    empty.lineitem.clear();
    let mut no_green = data.clone();
    no_green
        .part
        .iter_mut()
        .for_each(|(_, name)| *name = "red".into());
    no_green.lineitem.push((i64::MIN, -1, i64::MAX));
    for data in [&data, &empty, &no_green] {
        let expected = oracle(data);
        let source = Prepared::new(context, data)?;
        for mode in [Construction::Cpu, Construction::Gpu] {
            validate(run(context, data, Some(&source), mode)?.1, &expected);
        }
    }
    let mut invalid = data;
    invalid.lineitem.push((0, -1, 0));
    let source = Prepared::new(context, &invalid)?;
    for mode in [Construction::Cpu, Construction::Gpu] {
        assert!(
            compiled_h09::build(context, &source, mode).is_err(),
            "invalid key must not be narrowed"
        );
    }
    Ok(())
}
fn main() -> Result<()> {
    let options = Options::parse()?;
    let start = Instant::now();
    let context = MacContext::new()?;
    let setup_ms = ms(start);
    boundaries(&context)?;
    let mut records = Vec::new();
    println!(
        "rows\tboundary\tcpu_build_ms\tgpu_build_ms\tcpu_query_ms\tgpu_query_ms\tcpu_total_ms\tgpu_total_ms"
    );
    for n in options.sizes {
        let data = fixtures::h09_scaled(n);
        let expected = oracle(&data);
        let start = Instant::now();
        let source = Prepared::new(&context, &data)?;
        let preparation_ms = ms(start);
        for fresh in [false, true] {
            let mut cpu = Vec::new();
            let mut gpu = Vec::new();
            let mut first = Vec::new();
            for round in 0..options.runs + 2 {
                for is_cpu in [round % 2 == 0, round % 2 != 0] {
                    let mode = if is_cpu {
                        Construction::Cpu
                    } else {
                        Construction::Gpu
                    };
                    let (sample, actual) = run(&context, &data, (!fresh).then_some(&source), mode)?;
                    validate(actual, &expected);
                    if round == 0 {
                        first.push(json!({"provider":if is_cpu {"cpu"} else {"gpu"},"sample":sample.json(round,is_cpu)}));
                    }
                    if round >= 2 {
                        if is_cpu {
                            cpu.push(sample)
                        } else {
                            gpu.push(sample)
                        }
                    }
                }
            }
            let boundary = if fresh {
                "ingest_build_query"
            } else {
                "build_query"
            };
            println!(
                "{n}\t{boundary}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}",
                median(&cpu, |s| s.build_ms),
                median(&gpu, |s| s.build_ms),
                median(&cpu, |s| s.query_ms),
                median(&gpu, |s| s.query_ms),
                median(&cpu, |s| s.total_ms),
                median(&gpu, |s| s.total_ms)
            );
            records.push(json!({"rows":n,"answers":expected.len(),"boundary":boundary,
                "shared_preparation_ms":preparation_ms,"source_ingestion_ms":source.ingestion.as_secs_f64()*1000.0,
                "source_bytes":source.source_bytes(),"cpu":summary(&cpu),"gpu":summary(&gpu),"first":first,
                "cpu_samples":cpu.iter().enumerate().map(|(i,s)|s.json(i+2,i%2==0)).collect::<Vec<_>>(),
                "gpu_samples":gpu.iter().enumerate().map(|(i,s)|s.json(i+2,i%2!=0)).collect::<Vec<_>>() }));
        }
    }
    let report = json!({"schema":"mccl.compiled-h09.v1","args":std::env::args().collect::<Vec<_>>(),
        "architecture": std::env::consts::ARCH,
        "order": "alternating CPU-first/GPU-first pairs; first_in_pair records actual order; first contains each provider's first observation",
        "context_setup_ms_excluded":setup_ms,"warmups":2,"runs":options.runs,
        "query":"unchanged authored H09 CQ compiled through normal positional selection, Sequence and pull; source-bound Lineitem host at declaration slot1; all remaining operators identical",
        "cpu":"native RadixRun<u32,u32> row-ID index over the same retained i64 source; explicit domain validation included",
        "gpu":"GPU grouping/parallel spline RadixRun, source-bound original row IDs; same concrete host and compiled query function",
        "timing":"contiguous completed wall intervals; build_query reuses one ingested source; ingest_build_query includes fresh preparation/ingestion; query includes remaining CPU index construction, joins, canon and answer collection; source/index/output drop, fixtures, context and validation excluded",
        "limits":"synthetic data, not dbgen; warmed device/allocator and randomized CPU hash state; fresh source does not imply cold device; no automatic placement or CPU/GPU overlap",
        "results":records});
    if let Some(path) = options.out {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        serde_json::to_writer_pretty(
            std::io::BufWriter::new(std::fs::File::create(&path)?),
            &report,
        )?;
        println!("Raw compiled H09 samples: {}", path.display());
    }
    println!(
        "Compiled H09 exact answers, duplicate/wide boundaries and construction-domain errors passed."
    );
    Ok(())
}
