//! Operator profiles use the same authored sources, exports and exact oracle as bench.
use super::*;
const RUNS: usize = 11;
const EVIDENCE: &str = "/data/codex/opprofile-evidence";

pub(crate) fn run(arguments: Vec<String>) -> Result<()> {
    let mut args = arguments.into_iter();
    let mut sf = None;
    let mut query = None;
    let mut config = None;
    let mut tpch = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--tpch" if !tpch => tpch = true,
            "--sf" if sf.is_none() => sf = Some(args.next().ok_or("--sf needs a value")?),
            "--query" if query.is_none() => {
                query = Some(args.next().ok_or("--query needs a value")?)
            }
            "--config" if config.is_none() => {
                config = Some(args.next().ok_or("--config needs a value")?)
            }
            _ => return Err(format!("unknown or duplicate profile argument {arg}").into()),
        }
    }
    let sf = sf.ok_or("profile requires --tpch --sf 0.1|1")?;
    if !tpch || !matches!(sf.as_str(), "0.1" | "1") {
        return Err("profile requires --tpch --sf 0.1|1".into());
    }
    if query
        .as_ref()
        .is_some_and(|q| !cq_workloads::tpch::CASES.iter().any(|c| c.name == q))
    {
        return Err("unknown TPC query".into());
    }
    let config = config.unwrap_or_else(|| "default".into());
    let root = crate::repository_root();
    let evidence = Path::new(EVIDENCE);
    std::fs::create_dir_all(evidence)?;
    let database = data::dbgen(&root, &sf)?;
    database.connection.execute_batch("SET threads=1")?;
    // Finish compilation before any timed runs.
    let queries = cq_workloads::tpch::CASES
        .iter()
        .filter(|c| query.as_ref().is_none_or(|q| q == c.name))
        .map(|c| {
            describe_in(
                &root,
                &root.join(
                    c.rust_path
                        .strip_prefix("workloads/")
                        .unwrap_or(c.rust_path),
                ),
                Some(&evidence.join("workers")),
            )
        })
        .collect::<Result<Vec<_>>>()?;
    for q in queries {
        if !q.configs.contains(&config) {
            return Err(format!("{} has no configuration {config}", q.name).into());
        }
        let directory = data::export(&database, &q.name, &q.exports)?;
        let expected = answer_set(&database.connection, &q.sql)?;
        let artifact = evidence
            .join("runs")
            .join(format!("sf-{sf}"))
            .join(&q.name)
            .join(&config);
        eprintln!("profiling {}/{} SF {sf} after warm-up...", q.name, config);
        let normal = execute_worker(
            &q,
            &directory,
            &config,
            RUNS,
            &artifact.join("plain"),
            &expected,
        )?;
        let raw = execute_worker_binary(
            &q,
            &q.built.count_binary,
            &directory,
            &config,
            0,
            &artifact.join("primitive-counts"),
            &expected,
        )?;
        let counted_dir = artifact.join("profile");
        std::fs::create_dir_all(&counted_dir)?;
        let witness = counted_dir.join("oracle.json");
        std::fs::write(
            &witness,
            serde_json::to_vec(
                &serde_json::json!({"rows":expected.rows,"decimal_columns":expected.decimal_columns}),
            )?,
        )?;
        let status = worker_command(
            q.built
                .profile_binary
                .as_ref()
                .ok_or("missing profile worker")?,
        )
        .env("CQ_OPERATOR_PROFILE", "1")
        .env(
            "CQ_OPERATOR_COUNTS",
            artifact.join("primitive-counts/metrics.json"),
        )
        .arg(&directory)
        .arg(&config)
        .arg(RUNS.to_string())
        .arg(counted_dir.join("answers.json"))
        .arg(counted_dir.join("metrics.json"))
        .arg(&witness)
        .status()?;
        if !status.success() {
            return Err(format!("profile worker failed for {}: {status}", q.name).into());
        }
        let mut measured: Value =
            serde_json::from_slice(&std::fs::read(counted_dir.join("metrics.json"))?)?;
        if measured["operator_rows"] != raw["operator_rows"] {
            return Err(format!(
                "{} / {config}: operator rows differ from independent counters worker",
                q.name
            )
            .into());
        }
        let bag = measured["bag_insertions"]
            .as_u64()
            .ok_or("missing bag count")?;
        let sets = measured["set_insertions"]
            .as_u64()
            .ok_or("missing set count")?;
        let candidate = raw["candidate_insertions"]
            .as_u64()
            .ok_or("missing candidate count")?;
        if Some(bag) != raw["rows_copied"].as_u64()
            || Some(sets + candidate) != raw["distinct_insertions"].as_u64()
        {
            return Err(format!(
                "{} / {config}: independent primitive build counts disagree",
                q.name
            )
            .into());
        }
        measured["build_rows_verified"] = serde_json::json!(true);
        for key in [
            "rows_copied",
            "index_builds",
            "distinct_insertions",
            "candidate_insertions",
            "rows_enumerated",
        ] {
            measured[key] = raw[key].clone();
        }
        measured["storage_counter_source"] = serde_json::json!("separate untimed counters worker");
        measured["independent_primitive_counts"] = raw;
        let plain = number(&normal, "median_ms")?;
        let instrumented = number(&measured, "median_ms")?;
        let overhead = 100. * (instrumented / plain - 1.);
        let ours = format!(
            "{} / {} SF {}\nuninstrumented median {:.3} ms; profile median {:.3} ms; overhead {:+.2}%\n{}\n{}",
            q.name,
            config,
            sf,
            plain,
            instrumented,
            overhead,
            "Stream time: inclusive first-demand-to-exhaustion/drop span; nested times overlap. Build time: initialization wall clock, including deferred initialization. in/inserted count actual input occurrences; inserted includes duplicates. Rows come from a separate counters execution; timings come from the same operator graph with row counters disabled. Both executions are verified exactly against DuckDB.",
            measured["tree"]
                .as_str()
                .ok_or("profile worker omitted tree")?
        );
        std::fs::write(
            evidence.join(format!("{}.{config}.ours.txt", q.name)),
            &ours,
        )?;
        let raw = evidence.join(format!("{}.duckdb.json", q.name));
        database.connection.execute_batch(&format!(
            "SET threads=1; PRAGMA enable_profiling='json'; PRAGMA profiling_output={};",
            data::literal(&raw.to_string_lossy())
        ))?;
        let actual = answer_set(&database.connection, &q.sql)?;
        database
            .connection
            .execute_batch("PRAGMA disable_profiling")?;
        if actual != expected {
            return Err("DuckDB profile changed answers".into());
        }
        let json: Value = serde_json::from_slice(&std::fs::read(&raw)?)?;
        let duck = render_duck(&json);
        std::fs::write(evidence.join(format!("{}.duckdb.txt", q.name)), &duck)?;
        let mut side = String::new();
        for c in &q.configs {
            let path = evidence.join(format!("{}.{c}.ours.txt", q.name));
            if !path.exists() {
                continue;
            }
            let text = std::fs::read_to_string(path)?;
            let left: Vec<_> = text.lines().collect();
            let right: Vec<_> = duck.lines().collect();
            side.push_str(&format!("{} / {c} | DuckDB threads=1\n", q.name));
            for i in 0..left.len().max(right.len()) {
                side.push_str(&format!(
                    "{:<120} | {}\n",
                    left.get(i).unwrap_or(&""),
                    right.get(i).unwrap_or(&"")
                ));
            }
        }
        std::fs::write(evidence.join(format!("{}.side.txt", q.name)), side)?;
        let result = serde_json::json!({"sf":sf,"worker_cpu":std::env::var("CQ_PROFILE_CPU").ok(),"runs":RUNS,"query":q.name,"configuration":config,"plain":normal,"profile":measured,"overhead_percent":overhead,"within_5_percent_of_uninstrumented":overhead.abs()<=5.,"duckdb_version":database.version});
        std::fs::write(
            evidence.join(format!("{}.{config}.validation.json", q.name)),
            serde_json::to_vec_pretty(&result)?,
        )?;
        println!(
            "{} / {config}: {plain:.3} -> {instrumented:.3} ms ({overhead:+.2}%)",
            q.name
        );
    }
    Ok(())
}
fn render_duck(profile: &Value) -> String {
    use std::fmt::Write;
    fn walk(out: &mut String, node: &Value, depth: usize) {
        let kind = node["operator_name"].as_str().unwrap_or("QUERY");
        let extra = &node["extra_info"];
        let key = extra
            .get("Conditions")
            .or_else(|| extra.get("Table"))
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string())
            })
            .unwrap_or_default();
        writeln!(
            out,
            "{}{kind:<16} [{key}] rows={} t={:.3}ms",
            "  ".repeat(depth),
            node["operator_cardinality"],
            node["operator_timing"].as_f64().unwrap_or(0.) * 1000.
        )
        .unwrap();
        if let Some(children) = node["children"].as_array() {
            for c in children {
                walk(out, c, depth + 1);
            }
        }
    }
    let mut out = format!(
        "DuckDB threads=1 total {:.3} ms\nOperator times are DuckDB's operator timings (not inclusive pipeline spans).\n",
        profile["latency"].as_f64().unwrap_or(0.) * 1000.
    );
    if let Some(children) = profile["children"].as_array() {
        for c in children {
            walk(&mut out, c, 0);
        }
    }
    out
}
