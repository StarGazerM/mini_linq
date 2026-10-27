//! Compare clearing inactive MLX buffers with retaining the same allocator pool.
#[cfg(target_os = "macos")]
mod mac {
    use cq_mac::{IndexSpec, MacContext, MacRun, Payload, Relation, backend::MAX_ROWS, memory};
    use mac_query_expansion::{ManualResult, fixtures, h09, manual_h09};
    use serde_json::{Value, json};
    use std::{error::Error, fs, path::PathBuf, time::Instant};

    type Result<T> = std::result::Result<T, Box<dyn Error>>;

    struct Options {
        sizes: Vec<usize>,
        runs: usize,
        out: Option<PathBuf>,
        constructor_only: bool,
    }

    impl Options {
        fn parse() -> Result<Self> {
            let mut options = Self {
                sizes: vec![65_539, 1_048_579, 4_194_307],
                runs: 9,
                out: None,
                constructor_only: false,
            };
            let mut args = std::env::args().skip(1);
            while let Some(flag) = args.next() {
                let value = args
                    .next()
                    .ok_or_else(|| format!("missing value after {flag}"))?;
                match flag.as_str() {
                    "--sizes" => options.sizes = value.split(',').map(str::parse).collect::<std::result::Result<_, _>>()?,
                    "--runs" => options.runs = value.parse()?,
                    "--out" => options.out = Some(value.into()),
                    "--mode" => options.constructor_only = match value.as_str() {
                        "h09" => false,
                        "constructor" => true,
                        _ => return Err("--mode must be h09 or constructor".into()),
                    },
                    _ => return Err(format!("unknown option {flag}; use --sizes N,N --runs ODD --mode h09|constructor --out FILE").into()),
                }
            }
            if options.runs < 3
                || options.runs.is_multiple_of(2)
                || options.sizes.is_empty()
                || options.sizes.iter().any(|&n| n > MAX_ROWS)
            {
                return Err("use odd --runs >=3 and nonempty --sizes within MAX_ROWS".into());
            }
            Ok(options)
        }
    }

    struct RestoreCache(Option<usize>);

    impl RestoreCache {
        fn restore(&mut self) -> Result<()> {
            if let Some(previous) = self.0 {
                memory::set_cache_limit(previous)?;
                self.0 = None;
            }
            Ok(())
        }
    }

    impl Drop for RestoreCache {
        fn drop(&mut self) {
            if let Err(error) = self.restore() {
                eprintln!("failed to restore process-wide MLX cache limit: {error}");
            }
        }
    }

    fn stats() -> Result<Value> {
        let stats = memory::stats()?;
        Ok(
            json!({"active_bytes":stats.active_bytes, "cached_bytes":stats.cached_bytes,
            "peak_bytes":stats.peak_bytes}),
        )
    }

    fn counts(result: &ManualResult<h09::Answer>) -> Value {
        json!({"answers":result.answers.len(), "index_builds":result.index_builds,
            "indexed_rows":result.indexed_rows, "index_probes":result.index_probes,
            "payload_gathers":result.payload_gathers, "witnesses":result.witnesses})
    }

    fn check_run(run: &MacRun, expected: &[(u32, u32)]) {
        assert!(
            run.rows().eq(expected.iter().copied()),
            "complete stable rows differ"
        );
        let mut begin = 0;
        while begin < expected.len() {
            let key = expected[begin].0;
            let mut end = begin + 1;
            while end < expected.len() && expected[end].0 == key {
                end += 1;
            }
            assert!(
                run.lookup(&key)
                    .eq(expected[begin..end].iter().map(|row| row.1)),
                "exact lookup payload order differs for key {key}"
            );
            begin = end;
        }
    }

    enum Fixture {
        Constructor {
            source: Relation,
            ingestion_ms: f64,
            expected: Vec<(u32, u32)>,
        },
        Query {
            data: h09::Data,
            prepared: manual_h09::Prepared,
            first_cpu_query_ms: f64,
            reference: ManualResult<h09::Answer>,
        },
    }

    impl Fixture {
        fn new(context: &MacContext, n: usize, constructor_only: bool) -> Result<Self> {
            if constructor_only {
                let start = Instant::now();
                let source = context
                    .relation_u32(n, |i| [((i as u32).wrapping_mul(40503) & 0xffff) * 65537])?;
                let ingestion_ms = start.elapsed().as_secs_f64() * 1000.0;
                let mut expected: Vec<_> = source
                    .u32_values()?
                    .iter()
                    .enumerate()
                    .map(|(i, &key)| (key, i as u32))
                    .collect();
                expected.sort_by_key(|row| row.0);
                Ok(Self::Constructor {
                    source,
                    ingestion_ms,
                    expected,
                })
            } else {
                let data = fixtures::h09_scaled(n);
                let prepared = manual_h09::Prepared::new(context, &data)?;
                let start = Instant::now();
                let reference =
                    manual_h09::execute(context, &data, &prepared, &mut MacContext::build_on_cpu)?
                        .result;
                let first_cpu_query_ms = start.elapsed().as_secs_f64() * 1000.0;
                let mut compiled = h09::compiled(&data);
                compiled.sort_unstable();
                let mut answers = reference.answers.clone();
                answers.sort_unstable();
                assert_eq!(answers, compiled, "complete authored H09 answers differ");
                Ok(Self::Query {
                    data,
                    prepared,
                    first_cpu_query_ms,
                    reference,
                })
            }
        }

        fn staging(&self) -> Value {
            match self {
                Self::Constructor {
                    source,
                    ingestion_ms,
                    ..
                } => json!({"ingestion_ms":ingestion_ms, "source_bytes":source.len() * 4,
                        "source_columns":1, "source_dtype":"u32"}),
                Self::Query {
                    prepared,
                    first_cpu_query_ms,
                    ..
                } => json!({"ingestion_ms":prepared.ingestion.as_secs_f64() * 1000.0,
                        "source_bytes":prepared.source_bytes(), "source_columns":3, "source_dtype":"i64",
                        "first_cpu_reference_query_ms":first_cpu_query_ms,
                        "ingestion_plus_first_cpu_reference_query_ms":prepared.ingestion.as_secs_f64() * 1000.0 + first_cpu_query_ms}),
            }
        }

        fn sample(&self, context: &MacContext) -> Result<Value> {
            match self {
                Self::Constructor {
                    source, expected, ..
                } => {
                    let start = Instant::now();
                    let run = context.build(
                        source,
                        IndexSpec {
                            key_column: 0,
                            payload: Payload::RowId,
                        },
                        None,
                    )?;
                    let constructor_ms = start.elapsed().as_secs_f64() * 1000.0;
                    check_run(&run, expected);
                    Ok(json!({"constructor_ms":constructor_ms}))
                }
                Self::Query {
                    data,
                    prepared,
                    reference,
                    ..
                } => {
                    let start = Instant::now();
                    let execution =
                        manual_h09::execute(context, data, prepared, &mut MacContext::build)?;
                    let full_query_ms = start.elapsed().as_secs_f64() * 1000.0;
                    assert_eq!(
                        execution.result.answers, reference.answers,
                        "enumeration order differs"
                    );
                    assert_eq!(
                        counts(&execution.result),
                        counts(reference),
                        "query work/multiplicity differs"
                    );
                    Ok(json!({"full_query_ms":full_query_ms,
                        "constructor_ms":execution.construction.as_secs_f64() * 1000.0,
                        "counts":counts(reference)}))
                }
            }
        }
    }

    fn sample(fixture: &Fixture, context: &MacContext, cleared: bool) -> Result<Value> {
        let start = Instant::now();
        if cleared {
            memory::clear_cache()?;
        }
        let cache_boundary_ms = start.elapsed().as_secs_f64() * 1000.0;
        let before = stats()?;
        let mut result = fixture.sample(context)?;
        result["cache_boundary_ms_excluded"] = json!(cache_boundary_ms);
        result["before"] = before;
        result["after_validation_and_release"] = stats()?;
        Ok(result)
    }

    fn median(samples: &[Value], field: &str) -> f64 {
        let mut values: Vec<_> = samples
            .iter()
            .map(|sample| sample[field].as_f64().expect("recorded timing"))
            .collect();
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    }

    fn summary(samples: &[Value], constructor_only: bool) -> Value {
        let mut result = json!({"constructor_ms":median(samples, "constructor_ms")});
        if !constructor_only {
            result["full_query_ms"] = json!(median(samples, "full_query_ms"));
        }
        result
    }

    pub fn run() -> Result<()> {
        let options = Options::parse()?;
        let start = Instant::now();
        let context = MacContext::new()?;
        let setup_ms = start.elapsed().as_secs_f64() * 1000.0;
        let budget = options.sizes.iter().max().expect("nonempty sizes") * 32 + 4097 * 16;
        let previous_limit = memory::set_cache_limit(budget)?;
        let mut restore = RestoreCache(Some(previous_limit));
        let mut records = Vec::new();
        for &n in &options.sizes {
            let fixture = Fixture::new(&context, n, options.constructor_only)?;
            // These are first observations after oracle preparation, not physical cold starts.
            let first = json!({"cleared":sample(&fixture, &context, true)?,
                "pooled":sample(&fixture, &context, false)?});
            let staging = fixture.staging();
            let first_field = if options.constructor_only {
                "constructor_ms"
            } else {
                "full_query_ms"
            };
            let ingestion_plus_first_gpu_ms =
                staging["ingestion_ms"].as_f64().expect("ingestion timing")
                    + first["cleared"][first_field]
                        .as_f64()
                        .expect("first execution timing");
            let mut cleared = Vec::with_capacity(options.runs);
            let mut pooled = Vec::with_capacity(options.runs);
            for pair in 0..options.runs {
                for (order, clear) in [pair.is_multiple_of(2), !pair.is_multiple_of(2)]
                    .into_iter()
                    .enumerate()
                {
                    let mut value = sample(&fixture, &context, clear)?;
                    value["pair"] = json!(pair);
                    value["order"] = json!(order);
                    if clear {
                        cleared.push(value);
                    } else {
                        pooled.push(value);
                    }
                }
            }
            records.push(json!({"rows":n, "staging":staging,
                "ingestion_plus_first_gpu_ms":ingestion_plus_first_gpu_ms, "first_observations":first,
                "cleared_medians_ms":summary(&cleared, options.constructor_only),
                "pooled_medians_ms":summary(&pooled, options.constructor_only),
                "cleared_samples":cleared, "pooled_samples":pooled}));
        }
        restore.restore()?;
        let report = json!({"schema":"mccl.query-pool.v2",
            "mode":if options.constructor_only {"constructor"} else {"h09"},
            "cache_limit_bytes":budget, "previous_cache_limit_bytes":previous_limit, "restored":true,
            "wired_policy":"unchanged; no implicit residency request",
            "context_setup_ms_excluded":setup_ms, "samples_per_mode":options.runs,
            "order":"first cleared/pooled observations separate, then alternating paired order",
            "policy":"same MCCL constructor; clear inactive MLX cache before cleared samples, retain it before pooled samples; cache boundary excluded",
            "timing":"CPU wall time through completed publication from one retained shared numeric source; H09 additionally includes CPU joins/dedup and temporary destruction; ingestion reported separately and reused across cache clears; no GPU timestamps or DRAM counters",
            "first_execution_total":"measured ingestion plus first cleared GPU execution duration, a sum excluding oracle validation and cache-boundary work, not a contiguous cold-start interval",
            "source_bytes":"retained numeric relation logical bytes, not physical residency or measured traffic",
            "validation":"complete stable rows and per-key lookup, or authored H09 answers plus exact enumeration and work counters; outside timing; outputs released before next sample",
            "limits":"bounded process-wide MLX cache, not a per-context workspace; no guarantee of cold physical pages; other allocator users may interfere; single-process synthetic fixtures",
            "results":records});
        println!("{}", serde_json::to_string_pretty(&report)?);
        if let Some(path) = options.out {
            if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
                fs::create_dir_all(parent)?;
            }
            serde_json::to_writer_pretty(fs::File::create(path)?, &report)?;
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    mac::run()
}

#[cfg(not(target_os = "macos"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("MLX pool diagnostics require macOS; no CPU fallback".into())
}
