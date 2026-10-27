//! Matched radix/directory lifecycle diagnostic; no production-runtime API changes.
#[path = "../common.rs"]
mod common;
#[path = "lifetime/residency.rs"]
mod residency;
#[path = "../sort.rs"]
mod sort;

use common::{Gpu, Row, upload, view};
use metal::{Buffer, BufferRef, ComputePipelineStateRef};
use serde_json::{Value, json};
use sort::Sort;
use std::time::Instant;

const DIRECTORY_ENTRIES: usize = 4097;
const CASES: [&str; 14] = [
    "reuse",
    "reuse_upload",
    "reuse_readback",
    "reuse_both",
    "fresh_all",
    "fresh_pretouched",
    "fresh_input",
    "fresh_output",
    "fresh_scratch",
    "reuse_resident",
    "reuse_both_resident",
    "fresh_resident",
    "reuse_deferred_pool",
    "reuse_both_deferred_pool",
];

type Result<T, E = String> = std::result::Result<T, E>;

#[derive(Clone, Copy, Default)]
struct Faults {
    minor: i64,
    major: i64,
}
impl Faults {
    fn now() -> Result<Self> {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
            return Err(format!(
                "getrusage(RUSAGE_SELF): {}",
                std::io::Error::last_os_error()
            ));
        }
        let usage = unsafe { usage.assume_init() };
        Ok(Self {
            minor: usage.ru_minflt as i64,
            major: usage.ru_majflt as i64,
        })
    }
}

#[derive(Clone, Copy, Default)]
struct Phase {
    wall_ms: f64,
    faults: Faults,
}
impl Phase {
    fn json(self) -> Value {
        json!({"wall_ms": self.wall_ms,
            "cpu_process_minor_faults": self.faults.minor,
            "cpu_process_major_faults": self.faults.major})
    }
}
struct Stamp {
    faults: Faults,
    start: Instant,
}
impl Stamp {
    fn start() -> Result<Self> {
        Ok(Self {
            faults: Faults::now()?,
            start: Instant::now(),
        })
    }
    fn finish(self) -> Result<Phase> {
        let wall_ms = self.start.elapsed().as_secs_f64() * 1000.0;
        let end = Faults::now()?;
        Ok(Phase {
            wall_ms,
            faults: Faults {
                minor: end.minor - self.faults.minor,
                major: end.major - self.faults.major,
            },
        })
    }
}
fn measure<T>(f: impl FnOnce() -> T) -> Result<(T, Phase)> {
    let stamp = Stamp::start()?;
    let result = f();
    Ok((result, stamp.finish()?))
}

#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    input: bool,
    output: bool,
    scratch: bool,
    upload: bool,
    readback: bool,
    pretouch: bool,
    resident: bool,
    deferred_pool: bool,
}
impl Case {
    fn named(name: &str) -> Result<Self> {
        let name = *CASES
            .iter()
            .find(|&&candidate| candidate == name)
            .ok_or_else(|| format!("unknown case {name:?}; expected {}", CASES.join(",")))?;
        let all = matches!(name, "fresh_all" | "fresh_pretouched" | "fresh_resident");
        let input = all || name == "fresh_input";
        Ok(Self {
            name,
            input,
            output: all || name == "fresh_output",
            scratch: all || name == "fresh_scratch",
            upload: input
                || matches!(
                    name,
                    "reuse_upload"
                        | "reuse_both"
                        | "reuse_both_resident"
                        | "reuse_both_deferred_pool"
                ),
            readback: matches!(
                name,
                "reuse_readback"
                    | "reuse_both"
                    | "reuse_both_resident"
                    | "reuse_both_deferred_pool"
            ),
            pretouch: name == "fresh_pretouched",
            resident: matches!(
                name,
                "reuse_resident" | "reuse_both_resident" | "fresh_resident"
            ),
            deferred_pool: matches!(name, "reuse_deferred_pool" | "reuse_both_deferred_pool"),
        })
    }
    fn json(self) -> Value {
        json!({"replace_input_each_round": self.input,
            "replace_output_and_directory_each_round": self.output,
            "replace_scratch_and_scan_each_round": self.scratch,
            "upload_each_round": self.upload, "readback_each_round": self.readback,
            "cpu_zero_new_allocations": self.pretouch,
            "explicit_residency_request": self.resident,
            "command_autorelease_pool": if self.deferred_pool { "whole_case" } else { "each_submission" },
            "residency_scope": if self.name == "fresh_resident" { "each_build" }
                else if self.resident { "resource_setup_once" } else { "none" }})
    }
}

struct Options {
    sizes: Vec<usize>,
    runs: usize,
    cases: Vec<Case>,
    out: Option<String>,
}
fn options() -> Result<Options> {
    let mut result = Options {
        sizes: vec![1_048_576, 16_777_216],
        runs: 9,
        cases: CASES
            .iter()
            .map(|name| Case::named(name))
            .collect::<Result<_>>()?,
        out: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if matches!(arg.as_str(), "--help" | "-h") {
            println!(
                "lifetime [--sizes N,N] [--runs ODD>=3] [--cases NAME,NAME] [--out PATH]\n\
                Defaults: sizes=1048576,16777216; runs=9 warmed samples plus one cold sample; all cases.\n\
                Cases: {}\nJSON goes to stdout unless --out is supplied.",
                CASES.join(",")
            );
            std::process::exit(0);
        }
        if !matches!(arg.as_str(), "--sizes" | "--runs" | "--cases" | "--out") {
            return Err(format!("unknown argument {arg:?}"));
        }
        let value = args
            .next()
            .ok_or_else(|| format!("missing value for {arg}"))?;
        match arg.as_str() {
            "--sizes" => {
                result.sizes = value
                    .split(',')
                    .map(|s| {
                        s.parse::<usize>()
                            .map_err(|e| format!("invalid size {s:?}: {e}"))
                    })
                    .collect::<Result<_>>()?;
            }
            "--runs" => result.runs = value.parse().map_err(|e| format!("invalid runs: {e}"))?,
            "--cases" => result.cases = value.split(',').map(Case::named).collect::<Result<_>>()?,
            "--out" => result.out = Some(value),
            _ => unreachable!(),
        }
    }
    if result.runs < 3 || result.runs % 2 == 0 {
        return Err("--runs must be odd and >= 3".into());
    }
    if result.sizes.is_empty()
        || result
            .sizes
            .iter()
            .any(|&n| n == 0 || n > u32::MAX as usize)
    {
        return Err("--sizes must contain positive row counts fitting u32".into());
    }
    for (i, case) in result.cases.iter().enumerate() {
        if result.cases[..i]
            .iter()
            .any(|other| other.name == case.name)
        {
            return Err(format!("duplicate case {:?}", case.name));
        }
    }
    Ok(result)
}

fn random(state: &mut u64) -> u32 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
    ((z ^ (z >> 31)) >> 32) as u32
}
struct Data {
    input: Vec<Row>,
    expected: Vec<Row>,
    expected_directory: [u32; DIRECTORY_ENTRIES],
    readback: Vec<Row>,
    readback_directory: [u32; DIRECTORY_ENTRIES],
}
impl Data {
    fn new(n: usize) -> Self {
        let mut state = 0x6d617070696e6773;
        let mut input: Vec<Row> = Vec::with_capacity(n);
        for i in 0..n {
            let random_key = random(&mut state);
            // Full-u32 keys, deliberate duplicate keys, unrelated nonmonotonic payloads.
            let key = if i % 8 == 1 {
                input[i - 1].key
            } else {
                random_key
            };
            input.push(Row {
                key,
                value: random(&mut state),
            });
        }
        let mut expected = input.clone();
        expected.sort_by_key(|row| row.key);
        let mut expected_directory = [0; DIRECTORY_ENTRIES];
        let mut position = 0;
        for (prefix, offset) in expected_directory.iter_mut().enumerate() {
            let lower = (prefix as u64) << 20;
            while position < n && (expected[position].key as u64) < lower {
                position += 1;
            }
            *offset = position as u32;
        }
        Self {
            input,
            expected,
            expected_directory,
            readback: vec![Row { key: 0, value: 0 }; n],
            readback_directory: [0; DIRECTORY_ENTRIES],
        }
    }
    fn read(&mut self, buffers: &Buffers) {
        self.readback
            .copy_from_slice(view::<Row>(&buffers.output, self.input.len()));
        self.readback_directory
            .copy_from_slice(view::<u32>(&buffers.directory, DIRECTORY_ENTRIES));
        std::hint::black_box((&self.readback, &self.readback_directory));
    }
    fn validate(&self) -> Result<()> {
        if self.readback != self.expected {
            let index = self
                .readback
                .iter()
                .zip(&self.expected)
                .position(|(a, b)| a != b)
                .unwrap();
            return Err(format!(
                "stable row mismatch at {index}: {:?}, expected {:?}",
                self.readback[index], self.expected[index]
            ));
        }
        if self.readback_directory != self.expected_directory {
            let index = self
                .readback_directory
                .iter()
                .zip(&self.expected_directory)
                .position(|(a, b)| a != b)
                .unwrap();
            return Err(format!(
                "directory mismatch at {index}: {}, expected {}",
                self.readback_directory[index], self.expected_directory[index]
            ));
        }
        Ok(())
    }
}

struct Buffers {
    input: Buffer,
    output: Buffer,
    directory: Buffer,
}
impl Buffers {
    fn new(gpu: &Gpu, n: usize, pretouch: bool) -> Self {
        Self {
            input: gpu.allocate_buffer(n * std::mem::size_of::<Row>(), pretouch),
            output: gpu.allocate_buffer(n * std::mem::size_of::<Row>(), pretouch),
            directory: gpu.allocate_buffer(DIRECTORY_ENTRIES * 4, pretouch),
        }
    }
    fn replace(&mut self, gpu: &Gpu, sort: &mut Sort, n: usize, case: Case) {
        if case.input {
            self.input = gpu.allocate_buffer(n * std::mem::size_of::<Row>(), case.pretouch);
        }
        if case.output {
            self.output = gpu.allocate_buffer(n * std::mem::size_of::<Row>(), case.pretouch);
            self.directory = gpu.allocate_buffer(DIRECTORY_ENTRIES * 4, case.pretouch);
        }
        if case.scratch {
            sort.reallocate_scratch(gpu, case.pretouch);
        }
    }
    fn resources<'a>(&'a self, sort: &'a Sort, slots: &mut Vec<&'a BufferRef>) {
        slots.clear();
        slots.extend([
            self.input.as_ref(),
            self.output.as_ref(),
            self.directory.as_ref(),
        ]);
        slots.extend(sort.scratch_buffers());
    }
    fn request(&self, gpu: &Gpu, sort: &Sort) -> Result<residency::Residency> {
        // This temporary host allocation is charged to residency preparation.
        let mut resources = Vec::with_capacity(3 + sort.scratch_buffers().count());
        self.resources(sort, &mut resources);
        residency::request(&gpu.device, &resources)
    }
}

#[derive(Default)]
struct Sample {
    allocation: Phase,
    upload: Phase,
    residency: Phase,
    execution: Phase,
    gpu_ms: f64,
    readback: Phase,
    residency_release: Phase,
    total: Phase,
    validation: Phase,
}
impl Sample {
    fn json(&self, round: usize, ordinal: usize) -> Value {
        json!({"round": round, "submission_ordinal": ordinal,
            "first_process_submission": ordinal == 0,
            "sample_scope": if round == 0 { "first_submission_for_case" } else { "warmed_submission" },
            "allocation": self.allocation.json(), "cpu_upload": self.upload.json(),
            "residency_prep": self.residency.json(),
            "encode_commit_wait": self.execution.json(), "gpu_timestamp_ms": self.gpu_ms,
            "cpu_readback": self.readback.json(), "residency_release": self.residency_release.json(),
            "total_build": self.total.json(), "validation": self.validation.json()})
    }
}

fn run_sample(
    gpu: &Gpu,
    sort: &mut Sort,
    directory: &ComputePipelineStateRef,
    buffers: &mut Buffers,
    data: &mut Data,
    case: Case,
) -> Result<Sample> {
    let n = data.input.len();
    let total = Stamp::start()?;
    let mut sample = Sample::default();
    if case.input || case.output || case.scratch {
        sample.allocation = measure(|| buffers.replace(gpu, sort, n, case))?.1;
    }
    if case.upload {
        sample.upload = measure(|| upload(&buffers.input, &data.input))?.1;
    }
    let resident = if case.name == "fresh_resident" {
        let (result, phase) = measure(|| buffers.request(gpu, sort))?;
        sample.residency = phase;
        Some(result?)
    } else {
        None
    };
    // Deferred-pool cases match the original reference benchmark's command
    // object lifetime. Every command still completes before any CPU access.
    let encode = || {
        gpu.run(|cb| {
            sort.encode(gpu, cb, &buffers.input, &buffers.output, n);
            gpu.dispatch(
                cb,
                directory,
                &[&buffers.output, &buffers.directory],
                &[n as u32],
                DIRECTORY_ENTRIES.div_ceil(256),
                256,
            );
        })
    };
    let (execution, phase) = measure(|| {
        if case.deferred_pool {
            encode()
        } else {
            metal::objc::rc::autoreleasepool(encode)
        }
    })?;
    sample.execution = phase;
    sample.gpu_ms = execution.gpu_ms;
    if !sample.gpu_ms.is_finite() || sample.gpu_ms <= 0.0 {
        return Err(format!(
            "invalid completed-command GPU timestamp: {}",
            sample.gpu_ms
        ));
    }
    if case.readback {
        sample.readback = measure(|| data.read(buffers))?.1;
    }
    if resident.is_some() {
        sample.residency_release = measure(|| drop(resident))?.1;
    }
    sample.total = total.finish()?;
    if case.readback {
        let (valid, phase) = measure(|| data.validate())?;
        sample.validation = phase;
        valid?;
    }
    Ok(sample)
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}
fn medians(samples: &[Sample]) -> Value {
    let mut result = serde_json::Map::new();
    let phases: [(&str, fn(&Sample) -> Phase); 9] = [
        ("allocation", |s| s.allocation),
        ("cpu_upload", |s| s.upload),
        ("residency_prep", |s| s.residency),
        ("encode_commit_wait", |s| s.execution),
        ("cpu_readback", |s| s.readback),
        ("residency_release", |s| s.residency_release),
        ("total_build", |s| s.total),
        ("validation", |s| s.validation),
        ("gpu_timestamp", |s| Phase {
            wall_ms: s.gpu_ms,
            ..Phase::default()
        }),
    ];
    for (name, get) in phases {
        if name == "gpu_timestamp" {
            result.insert(
                "gpu_timestamp_ms".into(),
                json!(median(samples.iter().map(|s| s.gpu_ms).collect())),
            );
        } else {
            result.insert(name.into(), json!({
                "wall_ms": median(samples.iter().map(|s| get(s).wall_ms).collect()),
                "cpu_process_minor_faults": median(samples.iter().map(|s| get(s).faults.minor as f64).collect()),
                "cpu_process_major_faults": median(samples.iter().map(|s| get(s).faults.major as f64).collect()),
            }));
        }
    }
    Value::Object(result)
}

fn run_case(
    gpu: &Gpu,
    sort: &mut Sort,
    directory: &ComputePipelineStateRef,
    data: &mut Data,
    case: Case,
    runs: usize,
    ordinal: &mut usize,
) -> Result<Value> {
    let mut drain_start = None;
    let result = metal::objc::rc::autoreleasepool(|| {
        let result = run_case_inner(gpu, sort, directory, data, case, runs, ordinal);
        drain_start = Some(Stamp::start());
        result
    });
    let drain = drain_start
        .expect("case pool always reaches drainage")?
        .finish()?;
    let mut result = result?;
    result["mode_end"]["autorelease_pool_drain"] = drain.json();
    Ok(result)
}

fn run_case_inner(
    gpu: &Gpu,
    sort: &mut Sort,
    directory: &ComputePipelineStateRef,
    data: &mut Data,
    case: Case,
    runs: usize,
    ordinal: &mut usize,
) -> Result<Value> {
    let n = data.input.len();
    let setup = Stamp::start()?;
    let (mut buffers, allocation) = measure(|| {
        sort.resize_scratch(gpu, n, case.pretouch);
        Buffers::new(gpu, n, case.pretouch)
    })?;
    let (_, upload_phase) = measure(|| upload(&buffers.input, &data.input))?;
    let (persistent_resident, residency_phase) = if case.resident && case.name != "fresh_resident" {
        let (resident, phase) = measure(|| buffers.request(gpu, sort))?;
        (Some(resident?), phase)
    } else {
        (None, Phase::default())
    };
    let setup_total = setup.finish()?;
    let bytes = buffers.input.length()
        + buffers.output.length()
        + buffers.directory.length()
        + sort.scratch_buffers().map(|b| b.length()).sum::<u64>();
    let allocation_count = 3 + sort.scratch_buffers().count();
    let mut samples = Vec::with_capacity(runs);
    let cold_ordinal = *ordinal;
    let cold = run_sample(gpu, sort, directory, &mut buffers, data, case)?;
    *ordinal += 1;
    for _ in 0..runs {
        samples.push(run_sample(gpu, sort, directory, &mut buffers, data, case)?);
        *ordinal += 1;
    }
    // No shared output aliases escape this read, and it occurs after all submissions.
    let final_readback = if case.readback {
        Phase::default()
    } else {
        measure(|| data.read(&buffers))?.1
    };
    let (valid, final_validation) = measure(|| data.validate())?;
    valid?;
    let (_, residency_release) = measure(|| drop(persistent_resident))?;
    let medians = medians(&samples);
    eprintln!(
        "n={n} case={} cold_gpu_ms={:.4} warm_gpu_ms={:.4} warm_total_ms={:.4}",
        case.name,
        cold.gpu_ms,
        medians["gpu_timestamp_ms"].as_f64().unwrap(),
        medians["total_build"]["wall_ms"].as_f64().unwrap()
    );
    Ok(json!({"rows": n, "case": case.name, "factors": case.json(),
        "buffer_count": allocation_count, "allocated_bytes": bytes,
        "resource_setup_once": {"allocation": allocation.json(), "cpu_upload": upload_phase.json(),
            "residency_prep": residency_phase.json(), "total": setup_total.json()},
        "cold": cold.json(0, cold_ordinal),
        "samples": samples.iter().enumerate().map(|(i, s)| s.json(i + 1, cold_ordinal + i + 1)).collect::<Vec<_>>(),
        "warm_medians": medians,
        "mode_end": {"cpu_readback": final_readback.json(), "validation": final_validation.json(),
            "persistent_residency_release": residency_release.json(),
            "full_stable_rows_validated": true, "all_4097_directory_offsets_validated": true}}))
}

fn run() -> Result<()> {
    let options = options()?;
    let ((gpu, mut sort, directory), initialization) = measure(|| {
        let gpu = Gpu::new();
        // Only these pipelines are compiled, once for the entire process. The tiny
        // initial scratch allocation is reported here and replaced in mode setup.
        let sort = Sort::with_digit_bits(&gpu, 1, 8, 8);
        let directory = gpu.pipeline("maintain_directory");
        (gpu, sort, directory)
    })?;
    let mut results = Vec::with_capacity(options.sizes.len() * options.cases.len());
    let mut data_preparation = Vec::with_capacity(options.sizes.len());
    let mut ordinal = 0;
    for &n in &options.sizes {
        if n * std::mem::size_of::<Row>() > gpu.device.max_buffer_length() as usize {
            return Err(format!("size {n} exceeds device maximum buffer length"));
        }
        let (mut data, phase) = measure(|| Data::new(n))?;
        data_preparation.push(json!({"rows": n, "cpu_data_and_stable_reference": phase.json()}));
        for &case in &options.cases {
            results.push(run_case(
                &gpu,
                &mut sort,
                &directory,
                &mut data,
                case,
                options.runs,
                &mut ordinal,
            )?);
        }
    }
    let report = json!({
        "schema": "gpu-index-lifetime-v1", "device": gpu.device.name(),
        "sizes": options.sizes, "warm_runs": options.runs,
        "case_order": options.cases.iter().map(|case| case.name).collect::<Vec<_>>(),
        "scope": {
            "execution": "canonical Sort(8 digit bits, 8 items/thread), then maintain_directory; one command buffer; same retained pipelines throughout",
            "storage": "shared, default CPU cache, default tracked hazards, direct encoder buffer bindings",
            "topology": "distinct immutable-source input, output, radix-12 directory, radix scratch, every hierarchical scan level; same sizes/count per row size",
            "input": "deterministic SplitMix64 full-u32 keys with deliberate duplicate keys; independent full-u32 nonmonotonic payloads; identical unsorted input every submission",
            "cold": "first measured submission per case; no unmeasured GPU warmups; only ordinal 0 is process-first; fresh allocation does NOT guarantee cold physical pages",
            "warm": "subsequent submissions; fresh cases still replace their declared buffers each round",
            "resource_setup_once": "initial per-mode buffers, scratch resize, input upload, optional persistent residency; separate from builds, including for fresh cases",
            "allocation": "new buffers and replacement releases; CPU zeroing included only for fresh_pretouched; pipelines never recreated",
            "encode_commit_wait": "CPU wall including command creation, encoding, commit and wait; command autorelease drainage per-submission except explicit deferred-pool controls",
            "gpu_timestamp_ms": "completed command buffer GPUEndTime minus GPUStartTime; not CPU wall",
            "total_build": "enclosing wall/fault snapshot of allocation, upload, residency preparation, encode/commit/wait, optional readback, per-build residency release; includes phase instrumentation; excludes validation and one-time setup; deferred pool drainage reported separately at mode end",
            "faults": "libc getrusage(RUSAGE_SELF) CPU process minor/major fault deltas, NOT GPU faults and NOT evidence of disk I/O; phases include all process threads",
            "phase_faults": "before/after snapshots; disabled phases are zero; GPU timestamps have no separate CPU fault interval; no counter failures ignored",
            "readback": "full output and directory copied into preallocated CPU storage; only readback cases read between submissions; other cases first read after every repetition",
            "validation": "full stable row equality against CPU stable sort and all 4097 exact directory offsets; each readback round and every mode end; excluded from total build",
            "residency": "explicit create/add/commit/requestResidency; prep is a hint and may leave deferred work; original tracked bindings remain; endResidency after completion",
            "medians": "componentwise medians of the odd warmed sample count, excluding first measured cold submission; not sums of medians"
        },
        "initialization_once": initialization.json(),
        "data_preparation": data_preparation, "results": results,
    });
    let encoded = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    if let Some(path) = options.out {
        std::fs::write(&path, encoded).map_err(|e| format!("writing {path:?}: {e}"))?;
    } else {
        println!("{encoded}");
    }
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("lifetime: {error}");
        std::process::exit(1);
    }
}
