//! TPC-H-derived measurements over real dbgen projections. This is deliberately
//! separate from the tiny synthetic workload/golden corpus.
mod comparison;
mod compile;
pub(crate) mod profile;
use comparison::normalize_set;
mod data;

use crate::Result;
use duckdb::{Connection, types::ValueRef};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

#[derive(Debug, PartialEq, Eq)]
struct Options {
    sf: String,
    runs: usize,
    query: Option<String>,
    config: Option<String>,
    reduction_comparison: bool,
    cap_seconds: Option<u64>,
}
impl Options {
    fn parse(arguments: Vec<String>) -> Result<Self> {
        let mut tpch = false;
        let mut sf = None;
        let mut runs = None;
        let mut query = None;
        let mut config = None;
        let mut reduction_comparison = false;
        let mut cap_seconds = None;
        let mut args = arguments.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--tpch" if !tpch => tpch = true,
                "--reduction-comparison" if !reduction_comparison => reduction_comparison = true,
                "--query" if query.is_none() => {
                    query = Some(args.next().ok_or("--query requires a catalog query")?)
                }
                "--config" if config.is_none() => {
                    config = Some(
                        args.next()
                            .ok_or("--config requires a configuration name")?,
                    )
                }
                "--sf" if sf.is_none() => sf = Some(args.next().ok_or("--sf requires 0.1, 1 or 10")?),
                "--cap-seconds" if cap_seconds.is_none() => {
                    let seconds = args.next().ok_or("--cap-seconds requires a positive integer")?.parse::<u64>()?;
                    if seconds == 0 {
                        return Err("--cap-seconds must be positive".into());
                    }
                    cap_seconds = Some(seconds);
                }
                "--runs" if runs.is_none() => {
                    runs = Some(
                        args.next()
                            .ok_or("--runs requires a positive odd integer")?
                            .parse::<usize>()?,
                    )
                }
                _ => {
                    return Err(
                        format!("unknown or repeated TPC-H-derived bench option `{arg}`").into(),
                    );
                }
            }
        }
        let sf = sf.ok_or("TPC-H-derived bench requires --sf 0.1|1|10")?;
        if !tpch || !matches!(sf.as_str(), "0.1" | "1" | "10") {
            return Err("TPC-H-derived bench requires --tpch --sf 0.1|1|10".into());
        }
        let runs = runs.unwrap_or(5);
        if runs == 0 || runs % 2 == 0 {
            return Err("--runs must be a positive odd integer".into());
        }
        if config.is_some() && reduction_comparison {
            return Err("--config and --reduction-comparison are mutually exclusive".into());
        }
        Ok(Self {
            sf,
            runs,
            query,
            config,
            reduction_comparison,
            cap_seconds,
        })
    }
}

struct Query {
    name: String,
    sql: String,
    configs: Vec<String>,
    exports: Vec<(String, String)>,
    built: compile::Built,
}

fn describe(root: &Path, source: &Path) -> Result<Query> {
    describe_in(root, source, None)
}
fn describe_in(root: &Path, source: &Path, cache: Option<&Path>) -> Result<Query> {
    let name = source
        .file_stem()
        .unwrap()
        .to_str()
        .ok_or("non-UTF8 query filename")?;
    eprintln!("building TPC-H-derived query {name} in release...");
    let built = match cache {
        Some(cache) => compile::build_in(root, source, name, cache, true)?,
        None => compile::build(root, source, name)?,
    };
    let output = std::process::Command::new(&built.binary)
        .arg("describe")
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "query description failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let value: Value = serde_json::from_slice(&output.stdout)?;
    let declared = value["name"].as_str().ok_or("query has no name")?;
    if declared != name {
        return Err(format!("query {name} declares mismatching name {declared}").into());
    }
    let sql = value["sql"].as_str().ok_or("query has no SQL")?.to_owned();
    let configs = value["configs"]
        .as_array()
        .ok_or("query has no configurations")?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "invalid configuration".into())
        })
        .collect::<Result<Vec<_>>>()?;
    if !configs.iter().any(|v| v == "default") {
        return Err(format!("{name} requires a default configuration").into());
    }
    let exports = value["exports"]
        .as_array()
        .ok_or("query has no exports")?
        .iter()
        .map(|v| {
            Ok((
                v[0].as_str().ok_or("invalid export table")?.to_owned(),
                v[1].as_str().ok_or("invalid export columns")?.to_owned(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Query {
        name: name.to_owned(),
        sql,
        configs,
        exports,
        built,
    })
}

pub(super) fn run(arguments: Vec<String>) -> Result<()> {
    let options = Options::parse(arguments)?;
    let root = crate::repository_root();
    let bench = root.join("target/bench");
    std::fs::create_dir_all(&bench)?;
    let coverage = cq_workloads::tpch::COVERAGE;
    std::fs::write(bench.join("tpch-coverage.tsv"), coverage)?;
    eprintln!(
        "TPC-H-derived full-query coverage: Q6 and Q19 implemented; exact exclusions for the other 20 are in target/bench/tpch-coverage.tsv"
    );
    let database = data::dbgen(&root, &options.sf)?;
    // Compile all queries before measuring any of them. Build activity must not
    // overlap query timings or change the comparison's CPU contention.
    if let Some(name) = &options.query {
        if !cq_workloads::tpch::CASES
            .iter()
            .any(|case| case.name == name)
        {
            return Err(format!("unknown TPC query {name}").into());
        }
    }
    let mut queries = cq_workloads::tpch::CASES
        .iter()
        .filter(|case| options.query.as_ref().is_none_or(|name| case.name == name))
        .map(|case| {
            describe(
                &root,
                &root.join(
                    case.rust_path
                        .strip_prefix("workloads/")
                        .unwrap_or(case.rust_path),
                ),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    if let Some(config) = &options.config {
        for query in &mut queries {
            if !query.configs.contains(config) {
                return Err(format!("{} has no configuration {config}", query.name).into());
            }
            query
                .configs
                .retain(|name| name == "default" || name == config);
        }
    } else if options.reduction_comparison {
        for query in &mut queries {
            query
                .configs
                .retain(|config| matches!(config.as_str(), "default" | "no-reduce"));
        }
    } else {
        for query in &mut queries {
            query.configs.retain(|config| config != "no-reduce");
        }
    }
    // Gate every query/configuration on SF 0.1 before any timed measurements,
    // even when this invocation requests SF 1. Use the freshly compiled workers.
    let small = if options.sf == "0.1" {
        None
    } else {
        Some(data::dbgen(&root, "0.1")?)
    };
    let verification_database = small.as_ref().unwrap_or(&database);
    let mut gate_timeouts = BTreeSet::new();
    for query in &queries {
        let directory = data::export(verification_database, &query.name, &query.exports)?;
        let (oracle, _) = data::preload(&directory, &query.exports)?;
        oracle.execute_batch("SET threads=1")?;
        let expected = answer_set(&oracle, &query.sql)?;
        for config in &query.configs {
            let artifact = bench
                .join("verification/sf-0.1")
                .join(&query.name)
                .join(config);
            let deadline = options.cap_seconds.map(|s| Instant::now() + std::time::Duration::from_secs(s));
            if execute_worker_limited(query, &query.built.binary, &directory, config, 0, &artifact, &expected, deadline)?.is_none() {
                gate_timeouts.insert((query.name.clone(), config.clone()));
                continue;
            }
            eprintln!(
                "verified TPC-H-derived {}/{} SF 0.1: {} rows",
                query.name,
                config,
                expected.rows.len()
            );
        }
    }
    let mut suffix = options
        .query
        .as_ref()
        .map_or(String::new(), |name| format!("-{name}"));
    if let Some(config) = &options.config {
        suffix.push_str(&format!("-config-{config}"));
    }
    if options.reduction_comparison {
        suffix.push_str("-reduction-comparison");
    }
    let path = bench.join(format!("tpch-{}{suffix}.tsv", options.sf));
    let pending = path.with_extension("tsv.partial");
    let mut table = String::from(
        "suite\tsf\tquery\tconfiguration\truns\tanswer_rows\tload_ms\tquery_median_ms\tduckdb_load_ms\tduckdb_1_median_ms\tduckdb_default_median_ms\tduckdb_default_threads\tcompile_unit_ms\tpeak_rss_kib\tblocks_considered\tblocks_visited\tblocks_skipped\tblocks_skipped_metadata\tblocks_skipped_empty_selection\trows_tested\trows_copied\tbytes_retained_current\tbytes_retained_peak\tkeyset_staging_bytes_peak\thash_computations\tindex_builds\tlookups\tcursor_steps\trows_enumerated\tdistinct_insertions\tcorrectness\tjoin_filter_tested\tjoin_filter_passed\tduckdb_1_execute_median_ms\n",
    );
    println!(
        "TPC-H-derived; SF {}; release; one warm run + median of {}; DuckDB {} default threads={}",
        options.sf, options.runs, database.version, database.default_threads
    );
    println!(
        "{:24} {:20} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "query",
        "configuration",
        "rows",
        "load ms",
        "query ms",
        "DB 1 ms",
        "DB 1 ex ms",
        "DB def ms"
    );
    let mut details = Vec::new();
    for query in &queries {
        let directory = data::export(&database, &query.name, &query.exports)?;
        let (oracle, duck_load_ms) = data::preload(&directory, &query.exports)?;
        oracle.execute_batch("SET threads=1")?;
        let expected = answer_set(&oracle, &query.sql)?;
        let duck_one = duck_time(&oracle, &query.sql, options.runs)?;
        let duck_one_execute = duck_execute_time(&oracle, &query.sql, options.runs)?;
        oracle.execute_batch(&format!("SET threads={}", database.default_threads))?;
        let default_expected = answer_set(&oracle, &query.sql)?;
        if default_expected != expected {
            return Err(format!("DuckDB thread modes disagree for {}", query.name).into());
        }
        let duck_default = duck_time(&oracle, &query.sql, options.runs)?;
        let mut configs = Vec::new();
        for config in &query.configs {
            let artifact = bench
                .join("runs")
                .join(format!("sf-{}", options.sf))
                .join(&query.name)
                .join(config);
            eprintln!(
                "measuring TPC-H-derived {}/{} SF {}...",
                query.name, config, options.sf
            );
            let deadline = options.cap_seconds.map(|s| Instant::now() + std::time::Duration::from_secs(s));
            let results = (|| -> Result<Option<(Value, Value, Value)>> {
                if gate_timeouts.contains(&(query.name.clone(), config.clone())) {
                    return Ok(None);
                }
                let Some(measured) = execute_worker_limited(query, &query.built.binary,
                    &directory, config, options.runs, &artifact, &expected, deadline)? else {
                    return Ok(None);
                };
                let Some(counted) = execute_worker_limited(query, &query.built.count_binary,
                    &directory, config, 0, &artifact.with_extension("counts"), &expected, deadline)? else {
                    return Ok(None);
                };
                let Some(repeated) = execute_worker_limited(query, &query.built.count_binary,
                    &directory, config, 0, &artifact.with_extension("counts-repeat"), &expected, deadline)? else {
                    return Ok(None);
                };
                Ok(Some((measured, counted, repeated)))
            })()?;
            let Some((mut measured, counted, repeated)) = results else {
                let mut row = vec!["NA".to_owned(); 34];
                row[0] = "TPC-H-derived".into();
                row[1] = options.sf.clone();
                row[2] = query.name.clone();
                row[3] = config.clone();
                row[4] = options.runs.to_string();
                row[8] = format!("{duck_load_ms:.6}");
                row[9] = format!("{:.6}", duck_one.0);
                row[10] = format!("{:.6}", duck_default.0);
                row[11] = database.default_threads.to_string();
                row[12] = query.built.unit_ms.to_string();
                row[30] = "timeout".into();
                row[33] = format!("{:.6}", duck_one_execute.0);
                table.push_str(&row.join("\t"));
                table.push('\n');
                std::fs::write(&pending, &table)?;
                eprintln!("{} / {config}: timeout", query.name);
                configs.push(serde_json::json!({"configuration":config,"status":"timeout"}));
                continue;
            };
            // Parallel task lifetimes overlap according to the scheduler, so
            // retained-memory high water is an observation, not a work count.
            // All primitive work and final retained bytes must still agree.
            if counted.as_object().zip(repeated.as_object()).is_none_or(|(left, right)|
                left.len() != right.len() || left.iter().any(|(name, value)|
                    name != "bytes_retained_peak" && right.get(name) != Some(value)))
            {
                return Err("separate deterministic counting runs disagree".into());
            }
            if measured["instrumented"].as_bool() != Some(false)
                || counted["instrumented"].as_bool() != Some(true)
            {
                return Err("TPC timing/counting build separation failed".into());
            }
            for name in [
                "blocks_considered",
                "blocks_visited",
                "blocks_skipped",
                "blocks_skipped_metadata",
                "blocks_skipped_empty_selection",
                "rows_tested",
                "rows_copied",
                "bytes_retained_current",
                "bytes_retained_peak",
                "keyset_staging_bytes_peak",
                "hash_computations",
                "index_builds",
                "lookups",
                "cursor_steps",
                "rows_enumerated",
                "distinct_insertions",
                "join_filter_tested",
                "join_filter_passed",
            ] {
                measured[name] = counted[name].clone();
            }
            let load = number(&measured, "load_ms")?;
            let median = number(&measured, "median_ms")?;
            println!(
                "{:24} {:20} {:>10} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>10.3}",
                query.name,
                config,
                expected.rows.len(),
                load,
                median,
                duck_one.0,
                duck_one_execute.0,
                duck_default.0
            );
            let counters = [
                "peak_rss_kib",
                "blocks_considered",
                "blocks_visited",
                "blocks_skipped",
                "blocks_skipped_metadata",
                "blocks_skipped_empty_selection",
                "rows_tested",
                "rows_copied",
                "bytes_retained_current",
                "bytes_retained_peak",
                "keyset_staging_bytes_peak",
                "hash_computations",
                "index_builds",
                "lookups",
                "cursor_steps",
                "rows_enumerated",
                "distinct_insertions",
            ]
            .iter()
            .map(|k| {
                measured[k]
                    .as_u64()
                    .map_or_else(|| "NA".to_owned(), |n| n.to_string())
            })
            .collect::<Vec<_>>()
            .join("\t");
            let tested = measured["join_filter_tested"]
                .as_u64()
                .ok_or("missing join_filter_tested counter")?;
            let passed = measured["join_filter_passed"]
                .as_u64()
                .ok_or("missing join_filter_passed counter")?;
            table.push_str(&format!("TPC-H-derived\t{}\t{}\t{config}\t{}\t{}\t{load:.6}\t{median:.6}\t{duck_load_ms:.6}\t{:.6}\t{:.6}\t{}\t{:.6}\t{counters}\tpass\t{tested}\t{passed}\t{:.6}\n", options.sf, query.name, options.runs, expected.rows.len(), duck_one.0, duck_default.0, database.default_threads, query.built.unit_ms, duck_one_execute.0));
            std::fs::write(&pending, &table)?;
            configs.push(serde_json::json!({"configuration":config,"metrics":measured}));
        }
        let best = configs
            .iter()
            .filter(|v| v["metrics"]["median_ms"].is_number())
            .min_by(|a, b| {
                a["metrics"]["median_ms"]
                    .as_f64()
                    .unwrap()
                    .total_cmp(&b["metrics"]["median_ms"].as_f64().unwrap())
            })
            .map(|v| v["configuration"].clone());
        details.push(serde_json::json!({"query":query.name,"sql":query.sql,"exports":query.exports,
            "compile_unit_ms":query.built.unit_ms,"cargo_timing_report":query.built.timing_report,
            "best_measured_configuration":best,"duckdb_load_ms":duck_load_ms,"duckdb_1_samples_ms":duck_one.1,"duckdb_1_execute_samples_ms":duck_one_execute.1,"duckdb_default_samples_ms":duck_default.1,"configs":configs}));
    }
    std::fs::rename(&pending, &path)?;
    let rustc = std::process::Command::new("rustc")
        .arg("--version")
        .output()?;
    let metadata = serde_json::json!({
        "label":"TPC-H-derived", "selection":options.query, "sf":options.sf,"runs":options.runs,"warm_runs":1,
        "cap_seconds":options.cap_seconds,
        "duckdb_version":database.version,"duckdb_default_threads":database.default_threads,
        "rustc":String::from_utf8_lossy(&rustc.stdout).trim(),
        "compile_method":"Cargo --release --timings per-query binary unit (all authored configurations); macro expansion + optimization + linking; dependency units excluded; not isolated proc-macro CPU time",
        "query_boundary":"fresh query initialization, index construction, full typed result materialization; excludes load, serialization, canonical set comparison and result drop; duckdb_1_median_ms includes prepare/optimization and Arrow result materialization; duckdb_1_execute_median_ms prepares once untimed and times execution plus Arrow result materialization (MiniLinq plans at compile time)",
        "sf_0_1_gate":"all freshly compiled query/configuration workers checked before timing either scale",
        "memory_method":"Linux /proc/self/status VmHWM per configuration worker process, includes load, warmup, answers and correctness; not incremental query-only memory; NA elsewhere",
        "counter_scope":"cq-storage instrumentation disabled in timed binary; counters collected in a separately compiled binary and untimed execution; extension aggregate folds and uninstrumented host work are not counted; DuckDB lacks equivalent instrumentation",
        "cpu_model":std::fs::read_to_string("/proc/cpuinfo").ok().and_then(|text| text.lines().find_map(|line| line.strip_prefix("model name").and_then(|v|v.split_once(':').map(|(_,v)|v.trim().to_owned())))),
        "coverage":coverage,"queries":details,
    });
    std::fs::write(
        path.with_extension("json"),
        serde_json::to_vec_pretty(&metadata)?,
    )?;
    println!(
        "completed configurations matched DuckDB exactly; see status rows for timeouts; wrote {}",
        path.display()
    );
    Ok(())
}

fn execute_worker(
    query: &Query,
    directory: &Path,
    config: &str,
    runs: usize,
    artifact: &Path,
    expected: &Answers,
) -> Result<Value> {
    execute_worker_binary(
        query,
        &query.built.binary,
        directory,
        config,
        runs,
        artifact,
        expected,
    )
}

fn worker_command(binary: &Path) -> std::process::Command {
    if let Some(cpu) = std::env::var_os("CQ_PROFILE_CPU") {
        let mut command = std::process::Command::new("taskset");
        command.arg("--cpu-list").arg(cpu).arg(binary);
        command
    } else {
        std::process::Command::new(binary)
    }
}

fn execute_worker_binary(
    query: &Query,
    binary: &Path,
    directory: &Path,
    config: &str,
    runs: usize,
    artifact: &Path,
    expected: &Answers,
) -> Result<Value> {
    execute_worker_limited(query, binary, directory, config, runs, artifact, expected, None)?
        .ok_or_else(|| "uncapped worker unexpectedly timed out".into())
}

fn execute_worker_limited(
    query: &Query,
    binary: &Path,
    directory: &Path,
    config: &str,
    runs: usize,
    artifact: &Path,
    expected: &Answers,
    deadline: Option<Instant>,
) -> Result<Option<Value>> {
    std::fs::create_dir_all(artifact)?;
    let answers = artifact.join("answers.json");
    let metrics = artifact.join("metrics.json");
    let witness = artifact.join("oracle.json");
    std::fs::write(
        &witness,
        serde_json::to_vec(
            &serde_json::json!({"rows":expected.rows,"decimal_columns":expected.decimal_columns}),
        )?,
    )?;
    let mut child = worker_command(binary)
        .arg(directory)
        .arg(config)
        .arg(runs.to_string())
        .arg(&answers)
        .arg(&metrics)
        .arg(&witness)
        .spawn()?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            child.kill()?;
            child.wait()?;
            return Ok(None);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    if !status.success() {
        return Err(format!("{} / {config} failed: {status}", query.name).into());
    }
    let actual: Vec<Vec<String>> = serde_json::from_slice(&std::fs::read(&answers)?)?;
    let actual = normalize_set(actual, &expected.decimal_columns);
    if actual != expected.rows {
        return Err(format!(
            "{} / {config} worker answer differs from DuckDB",
            query.name
        )
        .into());
    }
    Ok(Some(serde_json::from_slice(&std::fs::read(&metrics)?)?))
}

fn number(value: &Value, key: &str) -> Result<f64> {
    value[key]
        .as_f64()
        .ok_or_else(|| format!("missing numeric metric {key}").into())
}

fn duck_time(connection: &Connection, sql: &str, runs: usize) -> Result<(f64, Vec<f64>)> {
    let mut samples = Vec::new();
    for index in 0..=runs {
        let started = Instant::now();
        let mut statement = connection.prepare(sql)?;
        let result: Vec<_> = statement.query_arrow([])?.collect();
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        std::hint::black_box(&result);
        if index > 0 {
            samples.push(elapsed);
        }
    }
    samples.sort_by(f64::total_cmp);
    Ok((samples[runs / 2], samples))
}

/// The same query boundary without parse, bind and optimization: the statement
/// is prepared once outside the timer, then each sample executes it and
/// materializes its Arrow result. This matches MiniLinq's boundary, whose
/// planning happens at Rust compile time.
fn duck_execute_time(connection: &Connection, sql: &str, runs: usize) -> Result<(f64, Vec<f64>)> {
    let mut statement = connection.prepare(sql)?;
    let mut samples = Vec::new();
    for index in 0..=runs {
        let started = Instant::now();
        let result: Vec<_> = statement.query_arrow([])?.collect();
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        std::hint::black_box(&result);
        if index > 0 {
            samples.push(elapsed);
        }
    }
    samples.sort_by(f64::total_cmp);
    Ok((samples[runs / 2], samples))
}

#[derive(PartialEq, Eq)]
struct Answers {
    rows: BTreeSet<Vec<String>>,
    decimal_columns: BTreeSet<usize>,
}

fn answer_set(connection: &Connection, sql: &str) -> Result<Answers> {
    let mut statement = connection.prepare(sql)?;
    let mut query = statement.query([])?;
    let columns = query
        .as_ref()
        .ok_or("missing oracle statement")?
        .column_count();
    let mut rows = Vec::new();
    let mut decimal_columns = BTreeSet::new();
    while let Some(row) = query.next()? {
        let mut values = Vec::new();
        for column in 0..columns {
            let text =
                match row.get_ref(column)? {
                    ValueRef::Null => "\\N".to_owned(),
                    ValueRef::Int(v) => v.to_string(),
                    ValueRef::BigInt(v) => v.to_string(),
                    ValueRef::HugeInt(v) => v.to_string(),
                    ValueRef::Decimal(v) => {
                        decimal_columns.insert(column);
                        v.to_string()
                    }
                    ValueRef::Text(v) => std::str::from_utf8(v)?.to_owned(),
                    other => return Err(format!(
                        "oracle output type requires explicit exact comparison support: {other:?}"
                    )
                    .into()),
                };
            values.push(text);
        }
        rows.push(values);
    }
    Ok(Answers {
        rows: normalize_set(rows, &decimal_columns),
        decimal_columns,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_scale_and_sampling_options() {
        let parse = |args: &[&str]| Options::parse(args.iter().map(|s| (*s).to_owned()).collect());
        assert_eq!(
            parse(&["--tpch", "--sf", "0.1"]).unwrap(),
            Options {
                sf: "0.1".into(),
                runs: 5,
                query: None,
                config: None,
                reduction_comparison: false,
                cap_seconds: None,
            }
        );
        assert_eq!(
            parse(&["--tpch", "--sf", "1", "--runs", "5", "--config", "le"])
                .unwrap()
                .config
                .as_deref(),
            Some("le"),
        );
        assert_eq!(parse(&["--tpch", "--sf", "10", "--runs", "1", "--cap-seconds", "2"]).unwrap().cap_seconds, Some(2));
        for args in [
            vec!["--tpch"],
            vec!["--tpch", "--sf", "2"],
            vec!["--tpch", "--sf", "1", "--runs", "2"],
            vec!["--tpch", "--sf", "1", "--runs", "0"],
            vec!["--tpch", "--sf", "1", "--cap-seconds", "0"],
            vec!["--tpch", "--sf", "1", "--cap-seconds"],
            vec!["--tpch", "--sf", "1", "--sf", "1"],
            vec!["--tpch", "--sf", "1", "--config"],
            vec![
                "--tpch", "--sf", "1", "--config", "le", "--config", "le_runs",
            ],
            vec![
                "--tpch",
                "--sf",
                "1",
                "--config",
                "le",
                "--reduction-comparison",
            ],
            vec![
                "--tpch",
                "--sf",
                "1",
                "--reduction-comparison",
                "--reduction-comparison",
            ],
        ] {
            assert!(parse(&args).is_err());
        }
    }
}
