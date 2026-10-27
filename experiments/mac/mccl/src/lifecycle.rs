use super::{exact, parse_size, stable_merge, Pattern, Result, Stats};
use mccl::{memory, Gpu, U32Array};
use serde_json::{json, Value};
use std::{
    fs::File,
    io::{self, Write},
    path::PathBuf,
    time::Instant,
};

struct Options {
    sizes: Vec<usize>,
    reps: usize,
    warmup: usize,
    wired: usize,
    out: PathBuf,
    sync: bool,
}

impl Options {
    fn parse(mut args: impl Iterator<Item = String>) -> Result<Self> {
        let mut options = Self {
            sizes: vec![64 * 1024, 1024 * 1024, 16 * 1024 * 1024, 64 * 1024 * 1024],
            reps: 9,
            warmup: 2,
            wired: 0,
            out: "lifecycle.json".into(),
            sync: false,
        };
        while let Some(flag) = args.next() {
            if flag == "--sync" {
                options.sync = true;
                continue;
            }
            let value = args
                .next()
                .ok_or_else(|| format!("missing value for {flag}"))?;
            match flag.as_str() {
                "--sizes" => {
                    options.sizes = value.split(',').map(parse_size).collect::<Result<_>>()?
                }
                "--reps" => options.reps = value.parse()?,
                "--warmup" => options.warmup = value.parse()?,
                "--wired" => options.wired = parse_bytes(&value)?,
                "--out" => options.out = value.into(),
                _ => return Err(format!("unknown lifecycle option {flag:?}").into()),
            }
        }
        if options.sizes.is_empty() || options.reps == 0 || options.warmup == 0 {
            return Err("sizes, reps and warmup must be nonempty/nonzero".into());
        }
        Ok(options)
    }
}

fn parse_bytes(value: &str) -> Result<usize> {
    let (digits, scale) = match value.as_bytes().last() {
        Some(b'k' | b'K') => (&value[..value.len() - 1], 1024usize),
        Some(b'm' | b'M') => (&value[..value.len() - 1], 1024usize.pow(2)),
        Some(b'g' | b'G') => (&value[..value.len() - 1], 1024usize.pow(3)),
        _ => (value, 1),
    };
    digits
        .parse::<usize>()?
        .checked_mul(scale)
        .ok_or_else(|| "byte budget overflow".into())
}

#[derive(Clone, Copy)]
struct Faults {
    minor: i64,
    major: i64,
}

impl Faults {
    fn now() -> Result<Self> {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // RUSAGE_SELF is process-wide CPU accounting, not GPU page-fault telemetry.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        let usage = unsafe { usage.assume_init() };
        Ok(Self {
            minor: usage.ru_minflt,
            major: usage.ru_majflt,
        })
    }

    fn since(self, before: Self) -> Value {
        json!({"minor": self.minor - before.minor, "major": self.major - before.major})
    }
}

fn allocator() -> Result<Value> {
    let stats = memory::stats()?;
    Ok(
        json!({"active_bytes": stats.active_bytes, "cached_bytes": stats.cached_bytes,
        "peak_bytes": stats.peak_bytes}),
    )
}

// Counters and JSON allocation stay outside the phase's CPU wall-time interval.
fn measure<T>(action: impl FnOnce() -> Result<T>) -> Result<(T, Value)> {
    let before = allocator()?;
    let faults = Faults::now()?;
    let start = Instant::now();
    let result = action()?;
    let us = start.elapsed().as_secs_f64() * 1e6;
    let faults_after = Faults::now()?;
    let after = allocator()?;
    Ok((
        result,
        json!({"cpu_wall_us": us, "cpu_process_fault_delta": faults_after.since(faults),
        "allocator_before": before, "allocator_after": after}),
    ))
}

// Explicit restoration reports errors; Drop is only an unwind/early-setup fallback.
struct Settings {
    cache: Option<usize>,
    wired: Option<usize>,
}

impl Settings {
    fn restore(&mut self) -> Result<()> {
        let mut errors = Vec::new();
        if let Some(previous) = self.wired {
            match memory::set_wired_limit(previous) {
                Ok(_) => self.wired = None,
                Err(error) => errors.push(format!("restore wired limit: {error}")),
            }
        }
        if let Some(previous) = self.cache {
            match memory::set_cache_limit(previous) {
                Ok(_) => self.cache = None,
                Err(error) => errors.push(format!("restore cache limit: {error}")),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; ").into())
        }
    }
}

impl Drop for Settings {
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            eprintln!("mccl lifecycle: {error}");
        }
    }
}

#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    resident: bool,
    clear: bool,
    legacy: bool,
}

const CASES: [Case; 6] = [
    Case {
        name: "resident-output-cache-cleared",
        resident: true,
        clear: true,
        legacy: false,
    },
    Case {
        name: "resident-output-pooled",
        resident: true,
        clear: false,
        legacy: false,
    },
    Case {
        name: "whole-round-direct-cache-cleared",
        resident: false,
        clear: true,
        legacy: false,
    },
    Case {
        name: "whole-round-direct-pooled",
        resident: false,
        clear: false,
        legacy: false,
    },
    Case {
        name: "whole-round-extra-copy-cache-cleared",
        resident: false,
        clear: true,
        legacy: true,
    },
    Case {
        name: "whole-round-extra-copy-pooled",
        resident: false,
        clear: false,
        legacy: true,
    },
];

#[derive(Clone, Copy)]
enum Operation {
    Merge,
    Copy,
}

impl Operation {
    fn name(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Copy => "copy",
        }
    }
}

struct HostInputs {
    a: Vec<u32>,
    b: Vec<u32>,
    expected: Vec<u32>,
}

enum Inputs {
    Merge(U32Array, U32Array),
    Copy(U32Array),
}

impl Inputs {
    fn upload(gpu: &Gpu, host: &HostInputs, op: Operation) -> Result<Self> {
        Ok(match op {
            Operation::Merge => Self::Merge(gpu.upload(&host.a)?, gpu.upload(&host.b)?),
            Operation::Copy => Self::Copy(gpu.upload(&host.expected)?),
        })
    }

    fn extra_copy(self, gpu: &Gpu) -> Result<Self> {
        let copy = |input: &U32Array| -> Result<U32Array> {
            let output = gpu.copy(input)?;
            output.eval()?;
            Ok(output)
        };
        Ok(match self {
            Self::Merge(a, b) => Self::Merge(copy(&a)?, copy(&b)?),
            Self::Copy(input) => Self::Copy(copy(&input)?),
        })
    }

    fn touch(&self, gpu: &Gpu) -> Result<()> {
        let touch = |input: &U32Array| -> Result<()> {
            gpu.copy(input)?.eval()?;
            Ok(())
        };
        match self {
            Self::Merge(a, b) => {
                touch(a)?;
                touch(b)?;
            }
            Self::Copy(input) => touch(input)?,
        }
        Ok(())
    }

    fn graph(&self, gpu: &Gpu) -> mccl::Result<U32Array> {
        match self {
            Self::Merge(a, b) => gpu.merge(a, b),
            Self::Copy(input) => gpu.copy(input),
        }
    }
}

fn boundary(gpu: &Gpu, clear: bool) -> Result<Value> {
    let (_, sync) = measure(|| {
        gpu.synchronize()?;
        Ok(())
    })?;
    let clear = if clear {
        Some(
            measure(|| {
                memory::clear_cache()?;
                Ok(())
            })?
            .1,
        )
    } else {
        None
    };
    Ok(json!({"stream_sync": sync, "clear_inactive_cache": clear}))
}

fn prepare(gpu: &Gpu, host: &HostInputs, op: Operation, legacy: bool) -> Result<(Inputs, Value)> {
    let (inputs, upload) = measure(|| Inputs::upload(gpu, host, op))?;
    let (inputs, extra_copy) = if legacy {
        let (inputs, phase) = measure(|| inputs.extra_copy(gpu))?;
        (inputs, Some(phase))
    } else {
        (inputs, None)
    };
    Ok((
        inputs,
        json!({"direct_shared_upload": upload, "legacy_extra_gpu_copy_and_array_eval": extra_copy}),
    ))
}

fn sample(
    gpu: &Gpu,
    host: &HostInputs,
    op: Operation,
    case: Case,
    resident: Option<&Inputs>,
    explicit_sync: bool,
) -> Result<Value> {
    // Synchronization/clearing is not charged to graph/eval/read timings. Clearing
    // releases only inactive cached allocations; resident input handles stay live.
    let baseline = boundary(gpu, case.clear)?;
    let before = allocator()?;
    let (owned, prep) = if resident.is_none() {
        let (inputs, phase) = prepare(gpu, host, op, case.legacy)?;
        (Some(inputs), Some(phase))
    } else {
        (None, None)
    };
    let inputs = resident.or(owned.as_ref()).ok_or("missing sample inputs")?;
    let (output, graph) = measure(|| Ok(inputs.graph(gpu)?))?;
    let (_, eval) = measure(|| {
        output.eval()?;
        Ok(())
    })?;
    let sync = if explicit_sync {
        Some(
            measure(|| {
                gpu.synchronize()?;
                Ok(())
            })?
            .1,
        )
    } else {
        None
    };
    let (checksum, read) = measure(|| {
        let values = output.as_slice()?;
        let sum = values
            .iter()
            .fold(0u64, |sum, &value| sum.wrapping_add(value as u64));
        Ok(std::hint::black_box(sum))
    })?;
    // Compare every value outside all reported operation/read intervals.
    exact(
        "lifecycle complete result",
        output.as_slice()?,
        &host.expected,
    )?;
    let (_, release) = measure(|| {
        drop(output);
        drop(owned);
        Ok(())
    })?;
    Ok(
        json!({"operation": op.name(), "baseline_boundary": baseline,
        "allocator_before_lifecycle": before, "input_preparation": prep,
        "graph_construction": graph, "array_ready_eval_wait": eval,
        "optional_explicit_stream_sync": sync, "cpu_first_read_scan": read,
        "checksum_wrapping_u64": checksum, "complete_values_validated_outside_timing": true,
        "release_output_and_round_inputs": release, "allocator_after_lifecycle": allocator()?}),
    )
}

fn paired_samples(
    gpu: &Gpu,
    host: &HostInputs,
    case: Case,
    resident: Option<&[Inputs; 2]>,
    options: &Options,
    stage: &str,
    index: usize,
) -> Result<Vec<Value>> {
    let order = if index.is_multiple_of(2) {
        [0, 1]
    } else {
        [1, 0]
    };
    let mut records = Vec::with_capacity(2);
    for operation_index in order {
        let op = [Operation::Merge, Operation::Copy][operation_index];
        let mut record = sample(
            gpu,
            host,
            op,
            case,
            resident.map(|inputs| &inputs[operation_index]),
            options.sync,
        )?;
        record["stage"] = json!(stage);
        record["index"] = json!(index);
        records.push(record);
    }
    Ok(records)
}

fn retained_proof(gpu: &Gpu, host: &HostInputs) -> Result<Value> {
    let merge_inputs = Inputs::upload(gpu, host, Operation::Merge)?;
    let copy_input = Inputs::upload(gpu, host, Operation::Copy)?;
    let old_merge = merge_inputs.graph(gpu)?;
    let old_copy = copy_input.graph(gpu)?;
    drop(merge_inputs);
    drop(copy_input);
    // The original Rust inputs are gone before lazy materialization.
    let old_merge_values = old_merge.as_slice()?;
    let old_copy_values = old_copy.as_slice()?;
    exact("retained lazy merge", old_merge_values, &host.expected)?;
    exact("retained lazy copy", old_copy_values, &host.expected)?;
    for round in 0..3 {
        // Distinct values make accidental reuse of a still-live allocation visible.
        let a = vec![u32::MAX - 7 - round; host.a.len()];
        let b = vec![u32::MAX - 3 - round; host.b.len()];
        let expected = stable_merge(&a, &b);
        let fresh_host = HostInputs { a, b, expected };
        let inputs = Inputs::upload(gpu, &fresh_host, Operation::Merge)?;
        let fresh_merge = inputs.graph(gpu)?;
        let fresh_copy = gpu.copy(&fresh_merge)?;
        exact(
            "fresh merge while old borrowed",
            fresh_merge.as_slice()?,
            &fresh_host.expected,
        )?;
        exact(
            "fresh copy while old borrowed",
            fresh_copy.as_slice()?,
            &fresh_host.expected,
        )?;
        exact(
            "old merge not overwritten",
            old_merge_values,
            &host.expected,
        )?;
        exact("old copy not overwritten", old_copy_values, &host.expected)?;
    }
    Ok(
        json!({"complete_values_validated": true, "retained_outputs": 2,
        "fresh_rounds": 3, "inputs_dropped_before_eval": true,
        "old_slices_borrowed_during_fresh_allocations": true, "fresh_values_differ_from_retained": true}),
    )
}

fn run_cases(gpu: &Gpu, options: &Options) -> Result<Value> {
    let mut sizes = Vec::new();
    for &total in &options.sizes {
        let mut patterns = Vec::new();
        for pattern in Pattern::ALL {
            let initial_boundary = boundary(gpu, true)?;
            // An unequal split exercises merge source boundaries and irregular tails.
            let a_len = total / 3;
            let b_len = total - a_len;
            let (host, host_init) = measure(|| {
                let (a, b) = pattern.inputs(a_len, b_len);
                let expected = stable_merge(&a, &b);
                Ok(HostInputs { a, b, expected })
            })?;
            let mut cases = Vec::new();
            for case in CASES {
                let start_boundary = boundary(gpu, true)?;
                let mut preparation = Vec::new();
                let resident = if case.resident {
                    let (merge, merge_prep) = prepare(gpu, &host, Operation::Merge, false)?;
                    let (copy, copy_prep) = prepare(gpu, &host, Operation::Copy, false)?;
                    preparation.extend([merge_prep, copy_prep]);
                    let (_, gpu_touch) = measure(|| {
                        merge.touch(gpu)?;
                        copy.touch(gpu)?;
                        Ok(())
                    })?;
                    preparation.push(json!({"resident_input_gpu_touch_extra_copies": gpu_touch}));
                    Some([merge, copy])
                } else {
                    None
                };
                // Both variants begin with an empty inactive pool; resident inputs
                // remain alive. Only the pooled variant keeps subsequent free buffers.
                let prepared_boundary = boundary(gpu, true)?;
                let first =
                    paired_samples(gpu, &host, case, resident.as_ref(), options, "first", 0)?;
                let mut warmups = Vec::new();
                for index in 0..options.warmup {
                    warmups.extend(paired_samples(
                        gpu,
                        &host,
                        case,
                        resident.as_ref(),
                        options,
                        "warmup",
                        index,
                    )?);
                }
                let mut warmed = Vec::new();
                for index in 0..options.reps {
                    warmed.extend(paired_samples(
                        gpu,
                        &host,
                        case,
                        resident.as_ref(),
                        options,
                        "warmed",
                        index,
                    )?);
                }
                drop(resident);
                let end_boundary = boundary(gpu, true)?;
                cases.push(json!({"name": case.name, "retained_inputs": case.resident,
                    "clear_inactive_cache_before_each_sample": case.clear,
                    "extra_gpu_copy_preparation": case.legacy, "start_boundary": start_boundary,
                    "resident_preparation": preparation, "prepared_boundary": prepared_boundary,
                    "first_invocation": first, "warmup_samples": warmups, "warmed_samples": warmed,
                    "end_boundary": end_boundary}));
            }
            let retained = retained_proof(gpu, &host)?;
            let end_boundary = boundary(gpu, true)?;
            patterns.push(json!({"pattern": pattern.to_string(), "a_keys": a_len, "b_keys": b_len,
                "initial_boundary": initial_boundary, "host_initialization_and_reference_merge": host_init,
                "cases": cases, "bounded_retained_output_proof": retained, "end_boundary": end_boundary}));
        }
        sizes.push(json!({"total_keys": total, "patterns": patterns}));
    }
    Ok(json!(sizes))
}

fn phase_us(sample: &Value, phase: &str) -> f64 {
    sample[phase]["cpu_wall_us"]
        .as_f64()
        .expect("measured lifecycle phase contains a CPU wall duration")
}

fn print_table(results: &Value) -> Result<()> {
    let stdout = io::stdout();
    let mut table = io::BufWriter::new(stdout.lock());
    writeln!(
        table,
        "CPU wall microseconds; graph/eval/read are separate, not GPU timestamps."
    )?;
    writeln!(
        table,
        "keys pattern case operation first_eval_us warmed_eval_median_us warmed_read_median_us"
    )?;
    for size in results.as_array().ok_or("invalid size records")? {
        for pattern in size["patterns"]
            .as_array()
            .ok_or("invalid pattern records")?
        {
            for case in pattern["cases"].as_array().ok_or("invalid case records")? {
                for op in ["merge", "copy"] {
                    let first = case["first_invocation"]
                        .as_array()
                        .ok_or("missing first records")?
                        .iter()
                        .find(|record| record["operation"] == op)
                        .ok_or("missing first operation")?;
                    let warmed: Vec<_> = case["warmed_samples"]
                        .as_array()
                        .ok_or("missing warmed records")?
                        .iter()
                        .filter(|record| record["operation"] == op)
                        .collect();
                    let eval = Stats::new(
                        warmed
                            .iter()
                            .map(|record| phase_us(record, "array_ready_eval_wait"))
                            .collect(),
                    );
                    let read = Stats::new(
                        warmed
                            .iter()
                            .map(|record| phase_us(record, "cpu_first_read_scan"))
                            .collect(),
                    );
                    writeln!(
                        table,
                        "{} {} {} {op} {:.3} {:.3} {:.3}",
                        size["total_keys"],
                        pattern["pattern"].as_str().ok_or("missing pattern")?,
                        case["name"].as_str().ok_or("missing case name")?,
                        phase_us(first, "array_ready_eval_wait"),
                        eval.median,
                        read.median
                    )?;
                }
            }
        }
    }
    table.flush()?;
    Ok(())
}

pub(super) fn run(args: impl Iterator<Item = String>) -> Result<()> {
    let options = Options::parse(args)?;
    // Fail on an unusable output path before changing process-wide MLX settings.
    let output = File::create(&options.out)?;
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if page_size <= 0 {
        return Err("sysconf(_SC_PAGESIZE) failed".into());
    }
    let cache_budget = options
        .sizes
        .iter()
        .copied()
        .max()
        .ok_or("missing size")?
        .checked_mul(std::mem::size_of::<u32>())
        .and_then(|bytes| bytes.checked_mul(4))
        .ok_or("cache budget overflow")?;
    let gpu = Gpu::new()?;
    let mut settings = Settings {
        cache: None,
        wired: None,
    };
    // Store each previous setting inside the measured closure, before any fallible
    // post-measurement counter query can return. Thus setup errors restore too.
    let (_, cache_configuration) = measure(|| {
        settings.cache = Some(memory::set_cache_limit(cache_budget)?);
        Ok(())
    })?;
    let (_, wired_configuration) = measure(|| {
        settings.wired = Some(memory::set_wired_limit(options.wired)?);
        Ok(())
    })?;
    let previous_cache = settings.cache;
    let previous_wired = settings.wired;
    let result = run_cases(&gpu, &options);
    // All case arrays have left scope on either Ok or Err. Do not clear until the
    // explicit boundary has completed; restoration is attempted even if it fails.
    let cleanup = boundary(&gpu, true);
    let restore_start = Instant::now();
    let restoration = settings.restore();
    let restoration_us = restore_start.elapsed().as_secs_f64() * 1e6;
    let mut errors = Vec::new();
    let results = match result {
        Ok(value) => Some(value),
        Err(error) => {
            errors.push(error.to_string());
            None
        }
    };
    let cleanup = match cleanup {
        Ok(value) => Some(value),
        Err(error) => {
            errors.push(format!("final boundary: {error}"));
            None
        }
    };
    if let Err(error) = restoration {
        errors.push(error.to_string());
    }
    if !errors.is_empty() {
        return Err(errors.join("; ").into());
    }
    let results = results.ok_or("missing lifecycle results")?;
    let report = json!({
        "schema": "mccl.lifecycle.v1", "cpu_page_size_bytes": page_size,
        "sizes_keys": options.sizes, "warmed_reps_per_operation": options.reps,
        "warmups_per_operation": options.warmup, "separate_first_invocation_per_operation": true,
        "optional_explicit_stream_sync": options.sync,
        "process_wide_configuration": {"cache_limit_bytes": cache_budget, "wired_limit_bytes": options.wired,
            "previous_cache_limit_bytes": previous_cache, "previous_wired_limit_bytes": previous_wired,
            "cache_configuration": cache_configuration, "wired_configuration": wired_configuration,
            "restored": true, "restore_cpu_wall_us": restoration_us},
        "scope": [
            "All times are CPU wall microseconds, not GPU timestamps or measured DRAM bandwidth.",
            "Minor/major faults are getrusage(RUSAGE_SELF) process CPU counters, not GPU faults; concurrent MLX work and counter overhead can contribute.",
            "Phase counter snapshots and JSON bookkeeping are outside phase timers; fault deltas include the timing probes.",
            "Host values and CPU reference are initialized once per size/pattern, before cases, and reused identically for both upload paths.",
            "Resident inputs are retained shared arrays touched by explicit GPU copies before first output samples; no physical residency guarantee is made.",
            "Cache-cleared means inactive MLX cache freed at synchronized boundaries, not cold physical pages; live input buffers are never freed by the diagnostic.",
            "Pooled means inactive buffers may be reused by MLX; stats do not prove allocation counts or GPU residency.",
            "First is the first operation invocation in each case, before its warmups; cases share a process, so it is not a fresh-process/JIT-cold measurement.",
            "Each sample builds a fresh lazy graph, waits for that array only, scans every shared result value once, then validates every value outside timings.",
            "An explicit stream barrier always precedes each sample outside operation timing; --sync separately measures an additional barrier after eval.",
            "Each first pair runs merge then copy; subsequent warmup/measured pairs alternate order and share the bounded pool.",
            "Legacy preparation is direct shared upload followed by extra GPU copy and array eval for every input; it is not a different allocator.",
            "Allocator peak is the process lifetime high-water mark, never reset. Cache budget is four largest output buffers; wired budget is explicit and defaults to zero.",
            "Key-only exact comparison checks all values but cannot observe equal-key source provenance; the existing stable CPU merge is the reference."
        ],
        "results": results, "final_boundary": cleanup
    });
    let mut output = io::BufWriter::new(output);
    serde_json::to_writer_pretty(&mut output, &report)?;
    writeln!(output)?;
    output.flush()?;
    print_table(&report["results"])?;
    eprintln!("Raw lifecycle JSON: {}", options.out.display());
    Ok(())
}
