// This source is included by one generated release crate per authored query.
// The crate contains all authored configurations; dependency compilation is excluded
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
    if (runs != 0 && runs % 2 == 0) || !query::CONFIGS.contains(&args[1].as_str()) {
        return Err("worker requires a known configuration and positive odd RUNS (or zero for verification)".into());
    }
    let witness: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[5])?)?;
    let oracle: std::collections::BTreeSet<Vec<String>> =
        serde_json::from_value(witness["rows"].clone())?;
    let decimals: std::collections::BTreeSet<usize> =
        serde_json::from_value(witness["decimal_columns"].clone())?;
    let start = Instant::now();
    let data = query::load(Path::new(&args[0]), &args[1])?;
    let load_ms = start.elapsed().as_secs_f64() * 1000.0;
    // One warm execution; its result is the correctness witness. Keep the
    // canonical witness outside measurement; do not retain warm query state.
    #[cfg(any(feature = "counters", feature = "profile"))]
    cq_storage::counters::reset();
    let profiling = std::env::var_os("CQ_OPERATOR_PROFILE").is_some();
    if cfg!(feature = "counters") && runs != 0 && !profiling {
        return Err("instrumented workers only count a separate untimed run".into());
    }
    let warm = query::run(&data, &args[1]);
    #[cfg(any(feature = "counters", feature = "profile"))]
    let counts = cq_storage::counters::snapshot();
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
    #[cfg(any(feature = "counters", feature = "profile"))]
    if profiling {
        return operator_profile(&data, &args, &expected, &decimals, runs);
    }
    if runs == 0 {
        std::fs::write(
            &args[4],
            serde_json::to_vec(&{
                #[allow(unused_mut)]
                let mut metrics = serde_json::json!({"verified":true,"answer_rows":answer_rows,"instrumented":cfg!(feature="counters"), "zone_metadata_bytes": crate::tpch_runtime::zone_metrics().1});
                #[cfg(any(feature = "counters", feature = "profile"))]
                for (name, value) in [
                    ("blocks_considered", counts.blocks_considered),
                    ("blocks_visited", counts.blocks_visited),
                    ("blocks_skipped", counts.blocks_skipped),
                    ("blocks_skipped_metadata", counts.blocks_skipped_metadata),
                    (
                        "blocks_skipped_empty_selection",
                        counts.blocks_skipped_empty_selection,
                    ),
                    ("rows_tested", counts.rows_tested),
                    ("rows_copied", counts.rows_copied),
                    ("bytes_retained_current", counts.bytes_retained_current),
                    ("bytes_retained_peak", counts.bytes_retained_peak),
                    (
                        "keyset_staging_bytes_peak",
                        counts.keyset_staging_bytes_peak,
                    ),
                    ("hash_computations", counts.hash_computations),
                    ("index_builds", counts.index_builds),
                    ("lookups", counts.lookups),
                    ("cursor_steps", counts.cursor_steps),
                    ("rows_enumerated", counts.rows_enumerated),
                    ("distinct_insertions", counts.distinct_insertions),
                    ("candidate_insertions", counts.candidate_insertions),
                    ("join_filter_tested", counts.join_filter_tested),
                    ("join_filter_passed", counts.join_filter_passed),
                ] {
                    metrics[name] = serde_json::json!(value);
                }
                #[cfg(any(feature = "counters", feature = "profile"))]
                {
                    metrics["operator_rows"] = operator_rows(&cq_storage::counters::operators());
                }
                metrics
            })?,
        )?;
        return Ok(());
    }
    let mut times = Vec::with_capacity(runs);
    for _ in 0..runs {
        let start = Instant::now();
        let answers = query::run(std::hint::black_box(&data), &args[1]);
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        std::hint::black_box(&answers);
        // Every measured execution must agree with the warm answer, not just
        // its cardinality. Conversion, sorting, and comparison are untimed.
        let canonical = comparison::normalize_set(query::canonical(&answers), &decimals);
        if canonical != expected {
            return Err("query answer changed between warm and measured runs".into());
        }
        times.push(elapsed);
    }
    times.sort_by(f64::total_cmp);
    std::fs::write(
        &args[4],
        serde_json::to_vec_pretty(&serde_json::json!({
            "load_ms": load_ms, "median_ms": times[runs / 2], "samples_ms": times,
            "answer_rows": answer_rows, "peak_rss_kib": peak_rss_kib(),
            "instrumented": false,
            "zone_build_ms": crate::tpch_runtime::zone_metrics().0,
            "zone_metadata_bytes": crate::tpch_runtime::zone_metrics().1,

        }))?,
    )?;
    Ok(())
}

#[cfg(any(feature = "counters", feature = "profile"))]
fn operator_profile(
    data: &query::Data,
    args: &[String],
    expected: &std::collections::BTreeSet<Vec<String>>,
    decimals: &std::collections::BTreeSet<usize>,
    runs: usize,
) -> Result<(), Box<dyn Error>> {
    use cq_storage::counters::{self, OperatorLabel, OperatorProgram};
    if runs < 3 {
        return Err("profile requires at least three runs".into());
    }
    let counted: serde_json::Value = serde_json::from_slice(&std::fs::read(
        std::env::var_os("CQ_OPERATOR_COUNTS").ok_or("missing independent operator counts")?,
    )?)?;
    let counted = counted["operator_rows"]
        .as_array()
        .ok_or("missing independent operator rows")?;
    let mut samples = Vec::new();
    let mut reference = None;
    for _ in 0..runs {
        counters::reset();
        let root = OperatorProgram::new(&[OperatorLabel {
            result: "query",
            kind: "QUERY",
            key: query::NAME,
            inputs: &[],
        }]);
        let started = Instant::now();
        let answers = root.scope(0, || query::run(std::hint::black_box(data), &args[1]));
        let elapsed = started.elapsed().as_secs_f64() * 1000.;
        std::hint::black_box(&answers);
        let actual = comparison::normalize_set(query::canonical(&answers), decimals);
        if &actual != expected {
            return Err("profile answers differ from DuckDB".into());
        }
        root.rows(0, 0, actual.len() as u64, 0);
        let mut records = counters::operators();
        records[0].rows_out = actual.len() as u64;
        let measured: Vec<_> = records
            .iter_mut()
            .filter(|r| r.label.kind != "QUERY")
            .collect();
        if measured.len() != counted.len() {
            return Err("timed operator graph differs from counted graph".into());
        }
        for (record, count) in measured.into_iter().zip(counted) {
            if count["result"] != record.label.result
                || count["kind"] != record.label.kind
                || count["key"] != record.label.key
                || count["inputs"] != serde_json::json!(record.label.inputs)
            {
                return Err("timed operator label differs from counted label".into());
            }
            record.rows_in = count["rows_in"].as_u64().ok_or("missing rows_in")?;
            record.rows_out = count["rows_out"].as_u64().ok_or("missing rows_out")?;
            record.rows_inserted = count["rows_inserted"]
                .as_u64()
                .ok_or("missing rows_inserted")?;
        }
        let rows = records
            .iter()
            .map(|r| (r.label.result, r.rows_in, r.rows_out, r.rows_inserted))
            .collect::<Vec<_>>();
        if reference.as_ref().is_some_and(|r| r != &rows) {
            return Err("operator counts changed between runs".into());
        }
        reference = Some(rows);
        let tree = counters::render_operators(&records);
        let counts = counters::snapshot();
        let bag_insertions: u64 = records
            .iter()
            .filter(|r| matches!(r.label.kind, "INDEX_BUILD" | "ROWS_BUILD" | "PARTITION_BUILD"))
            .map(|r| r.rows_inserted)
            .sum();
        let set_insertions: u64 = records
            .iter()
            .filter(|r| matches!(r.label.kind, "DEDUP" | "KEYSET_BUILD" | "JOIN_FILTER_BUILD"))
            .map(|r| r.rows_inserted)
            .sum();
        if counters::STORAGE_COUNTERS_ENABLED
            && (bag_insertions != counts.rows_copied
                || set_insertions + counts.candidate_insertions != counts.distinct_insertions)
        {
            std::fs::write(Path::new(&args[4]).with_extension("failure.txt"), &tree)?;
            return Err(format!("independent build counters disagree: bag {bag_insertions}/{} copied; set {set_insertions} + candidates {}/{} distinct attempts", counts.rows_copied, counts.candidate_insertions, counts.distinct_insertions).into());
        }
        let root_ms = records[0].elapsed.as_secs_f64() * 1000.;
        if (root_ms / elapsed - 1.).abs() > 0.03 {
            return Err("top-level scope differs from independently measured query wall time by more than 3%".into());
        }
        let operators=records.iter().map(|r|serde_json::json!({"id":r.id,"program":r.program,"result":r.label.result,"kind":r.label.kind,"key":r.label.key,"inputs":r.label.inputs,"rows_in":r.rows_in,"rows_out":r.rows_out,"rows_inserted":r.rows_inserted,"elapsed_ms":r.elapsed.as_secs_f64()*1000.,"calls":r.calls})).collect::<Vec<_>>();
        samples.push((elapsed,serde_json::json!({"tree":tree,"operators":operators,"operator_rows":operator_rows(&records),"answer_rows":actual.len(),"index_builds":counts.index_builds,"rows_copied":counts.rows_copied,"distinct_insertions":counts.distinct_insertions,"candidate_insertions":counts.candidate_insertions,"rows_enumerated":counts.rows_enumerated,"bag_insertions":bag_insertions,"set_insertions":set_insertions,"top_level_ms":root_ms,"build_rows_verified":counters::STORAGE_COUNTERS_ENABLED,"top_level_clock_verified":true})));
    }
    samples.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut metrics = samples[runs / 2].1.clone();
    metrics["median_ms"] = serde_json::json!(samples[runs / 2].0);
    metrics["samples_ms"] = serde_json::json!(samples.iter().map(|s| s.0).collect::<Vec<_>>());
    metrics["verified"] = serde_json::json!(true);
    std::fs::write(&args[4], serde_json::to_vec_pretty(&metrics)?)?;
    Ok(())
}

#[cfg(any(feature = "counters", feature = "profile"))]
fn operator_rows(records: &[cq_storage::counters::OperatorRecord]) -> serde_json::Value {
    serde_json::json!(records.iter().filter(|r|r.label.kind!="QUERY").map(|r|serde_json::json!({
        "result":r.label.result,"kind":r.label.kind,"key":r.label.key,"inputs":r.label.inputs,
        "rows_in":r.rows_in,"rows_out":r.rows_out,"rows_inserted":r.rows_inserted,
    })).collect::<Vec<_>>())
}
