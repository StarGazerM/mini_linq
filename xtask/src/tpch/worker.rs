// This source is included by one generated release crate per authored query.
// The crate contains both configurations; dependency compilation is excluded
// from its Cargo unit timing. Query output serialization is never timed.
use std::error::Error;
use std::path::Path;
use std::time::Instant;

fn peak_rss_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmHWM:")
                .and_then(|v| v.split_whitespace().next()?.parse().ok())
        })
}

fn main() -> Result<(), Box<dyn Error>> {
    if cfg!(debug_assertions) {
        return Err("TPC-H-derived workers require a release build".into());
    }
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["describe"] {
        println!(
            "{}",
            serde_json::json!({
                "name": query::NAME, "sql": query::SQL,
                "exports": query::EXPORTS, "configs": query::CONFIGS,
            })
        );
        return Ok(());
    }
    if args.len() != 6 {
        return Err("worker expects DATA CONFIG RUNS ANSWERS METRICS ORACLE".into());
    }
    let runs: usize = args[2].parse()?;
    if (runs != 0 && (runs < 3 || runs % 2 == 0)) || !query::CONFIGS.contains(&args[1].as_str()) {
        return Err("worker requires a known configuration and odd RUNS >= 3".into());
    }
    let witness: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[5])?)?;
    let oracle: std::collections::BTreeSet<Vec<String>> =
        serde_json::from_value(witness["rows"].clone())?;
    let decimals: std::collections::BTreeSet<usize> =
        serde_json::from_value(witness["decimal_columns"].clone())?;
    let start = Instant::now();
    let data = query::load(Path::new(&args[0]))?;
    let load_ms = start.elapsed().as_secs_f64() * 1000.0;
    // One warm execution; its result is the correctness witness. Keep the
    // canonical witness outside measurement; do not retain warm query state.
    let warm = query::run(&data, &args[1]);
    let expected = comparison::normalize_set(query::canonical(&warm), &decimals);
    if expected != oracle {
        return Err(format!("{} / {} warm answer differs from DuckDB before any measured run: expected {} rows, actual {}; missing {:?}; extra {:?}",
            query::NAME, args[1], oracle.len(), expected.len(),
            oracle.difference(&expected).take(3).collect::<Vec<_>>(),
            expected.difference(&oracle).take(3).collect::<Vec<_>>()).into());
    }
    drop(warm);
    std::fs::write(&args[3], serde_json::to_vec(&expected)?)?;
    let answer_rows = expected.len();
    if runs == 0 {
        std::fs::write(
            &args[4],
            serde_json::to_vec(&serde_json::json!({"verified":true,"answer_rows":answer_rows}))?,
        )?;
        return Ok(());
    }
    let mut times = Vec::with_capacity(runs);
    let mut snapshots = Vec::with_capacity(runs);
    for _ in 0..runs {
        cq_storage::counters::reset();
        let start = Instant::now();
        let answers = query::run(std::hint::black_box(&data), &args[1]);
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        let counts = cq_storage::counters::snapshot();
        std::hint::black_box(&answers);
        // Every measured execution must agree with the warm answer, not just
        // its cardinality. Conversion, sorting, and comparison are untimed.
        let canonical = comparison::normalize_set(query::canonical(&answers), &decimals);
        if canonical != expected {
            return Err("query answer changed between warm and measured runs".into());
        }
        times.push(elapsed);
        snapshots.push(counts);
    }
    let first = snapshots[0];
    if snapshots.iter().any(|s| s != &first) {
        return Err("deterministic query counters changed between runs".into());
    }
    times.sort_by(f64::total_cmp);
    std::fs::write(
        &args[4],
        serde_json::to_vec_pretty(&serde_json::json!({
            "load_ms": load_ms, "median_ms": times[runs / 2], "samples_ms": times,
            "answer_rows": answer_rows, "peak_rss_kib": peak_rss_kib(),
            "hash_computations": first.hash_computations, "index_builds": first.index_builds,
            "lookups": first.lookups, "cursor_steps": first.cursor_steps,
            "rows_enumerated": first.rows_enumerated, "distinct_insertions": first.distinct_insertions,
        }))?,
    )?;
    Ok(())
}
