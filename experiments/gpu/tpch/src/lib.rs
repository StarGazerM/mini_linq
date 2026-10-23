#![recursion_limit = "1024"]

pub mod benchmark;
pub mod loader;
pub mod manifest;
#[allow(warnings)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/queries.rs"));
}

use manifest::{Manifest, Result};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn plans() -> Result<Vec<Value>> {
    Ok(serde_json::from_str(include_str!(concat!(
        env!("OUT_DIR"),
        "/plans.json"
    )))?)
}

pub fn data_root() -> PathBuf {
    PathBuf::from(env!("TPCH_GPU_COMPILED_ROOT"))
}

pub fn run(
    output: &Path,
    queries: &[String],
    scales: &[u32],
    modes: &[String],
    authored: Option<&Path>,
    baseline: Option<&Path>,
) -> Result<bool> {
    let mut results = plans()?;
    if modes
        .iter()
        .any(|mode| !matches!(mode.as_str(), "automatic" | "advised"))
    {
        return Err("mode must be automatic or advised".into());
    }
    let authored_results: Option<Vec<Value>> = authored
        .map(|path| -> Result<_> { Ok(serde_json::from_slice(&fs::read(path)?)?) })
        .transpose()?;
    // Capture the previous measurements before replacing the output file.
    let baseline_path = baseline.or_else(|| output.is_file().then_some(output));
    let baseline_results: Vec<Value> = if let Some(path) = baseline_path {
        let document: Value = serde_json::from_slice(&fs::read(path)?)?;
        document["results"]
            .as_array()
            .ok_or("baseline requires a results array")?
            .clone()
    } else {
        Vec::new()
    };
    for query in queries {
        if !results
            .iter()
            .any(|result| result["query"].as_str() == Some(query.as_str()))
        {
            return Err(format!("unknown query: {query}").into());
        }
    }
    if scales.iter().any(|scale| !matches!(scale, 1 | 10)) {
        return Err("scale must be sf1 or sf10".into());
    }
    results.retain(|result| {
        (queries.is_empty()
            || queries
                .iter()
                .any(|query| result["query"].as_str() == Some(query.as_str())))
            && (scales.is_empty()
                || scales
                    .iter()
                    .any(|&scale| result["sf"].as_u64() == Some(u64::from(scale))))
            && (modes.is_empty()
                || modes
                    .iter()
                    .any(|mode| result["mode"].as_str() == Some(mode.as_str())))
    });
    let mut successful = true;
    for result in &mut results {
        let query = result["query"].as_str().unwrap().to_owned();
        let sf = result["sf"].as_u64().unwrap();
        let mode = result["mode"].as_str().unwrap().to_owned();
        let manifest = Manifest::load(&data_root().join(format!("sf{sf}")))?;
        result["duckdb"] = benchmark::duckdb(&manifest, &query)?;
        if result["status"] == "accepted" {
            // A runtime failure is neither a planning rejection nor a passing
            // measurement. Preserve it, continue the inventory, then fail main.
            let execution = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                generated::execute(&query, &manifest, &mode, false)
            }));
            let execution = match execution {
                Ok(value) => value,
                Err(payload) => {
                    let error = payload
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| payload.downcast_ref::<&str>().copied())
                        .unwrap_or("non-string panic");
                    Err(format!("GPU/reference execution panicked: {error}").into())
                }
            };
            match execution {
                Ok(Some(measurement)) => result
                    .as_object_mut()
                    .unwrap()
                    .extend(measurement.as_object().unwrap().clone()),
                Ok(None) => {
                    return Err(
                        format!("accepted query has no compiled executor: {query} SF{sf}").into(),
                    );
                }
                Err(error) => {
                    successful = false;
                    result["status"] = json!("ran");
                    result["exact"] = json!(false);
                    result["error"] = json!(error.to_string());
                }
            }
        } else {
            successful &= query == "h07";
            result["exact"] = Value::Null;
            result["conformance"] =
                json!({"status": "not run", "reason": result["status"], "error": result["error"]});
        }
        let previous = baseline_results
            .iter()
            .filter(|row| row["query"] == query && row["sf"] == sf && row["exact"] == true)
            .filter_map(|row| row["median_ms"]["total"].as_f64().map(|total| (row, total)))
            .min_by(|(_, a), (_, b)| a.total_cmp(b));
        if let Some((previous, previous_ms)) = previous {
            result["baseline"] = json!({
                "source": baseline_path,
                "mode": previous["mode"],
                "storage": previous["pipeline"]["storage"],
                "median_ms": previous["median_ms"],
                "samples": previous["samples"],
                "exact": previous["exact"],
            });
            if let Some(total) = result["median_ms"]["total"].as_f64() {
                result["baseline_comparison"] = json!({
                    "total_ratio": total / previous_ms,
                    "no_slower_than_previous_best": total <= previous_ms,
                });
            }
        }
        if query == "h05" && mode == "advised" && result["exact"] == true {
            if let Some(total) = result["median_ms"]["total"].as_f64() {
                let reference = authored_results.as_ref().and_then(|rows| {
                    rows.iter()
                        .find(|row| row["sf"] == sf && row["plan"] == "R" && row["exact"] == true)
                });
                if authored.is_some() && reference.is_none() {
                    return Err(
                        format!("authored report has no exact H05 R SF{sf} measurement").into(),
                    );
                }
                let authored_ms = reference.and_then(|row| row["total_ms"]["median"].as_f64());
                let within_reference = authored_ms.map(|time| total <= time);
                let within_sf10_ceiling = sf != 10 || total <= 10.35;
                result["authored_comparison"] = json!({
                    "source": authored,
                    "plan": "R",
                    "authored_total_ms": authored_ms,
                    "advised_total_ms": total,
                    "no_slower_than_authored": within_reference,
                    "sf10_ceiling_ms": 10.35,
                    "within_sf10_ceiling": within_sf10_ceiling,
                });
                successful &= within_sf10_ceiling && within_reference.unwrap_or(true);
            }
        }
    }
    if !queries.is_empty() || !scales.is_empty() || !modes.is_empty() {
        println!("{}", serde_json::to_string_pretty(&results)?);
    }
    let table = benchmark::table(&results);
    println!("{table}");
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        output,
        serde_json::to_vec_pretty(&json!({
            "schema_version": 2,
            "data_root": data_root(),
            "warmup_runs": benchmark::WARMUP_RUNS,
            "measured_runs": benchmark::MEASURED_RUNS,
            "timing": "Median of per-run stage sums: CUDA phase events for device stages, elapsed execution for host-planned stages; initial input loading, final answer download and comparison excluded",
            "source": "CPU retail_kernel and standalone compile! CQ ASTs imported with include_str; only manifest storage types, date/decimal encodings and dictionary literals adapted",
            "results": results,
            "table_markdown": table,
        }))?,
    )?;
    Ok(successful)
}

#[cfg(test)]
#[path = "source.rs"]
mod source;
#[cfg(test)]
mod stages;

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_conformant(query: &str) -> Result<()> {
        let reports = plans()?;
        for report in reports
            .iter()
            .filter(|report| report["query"] == query && report["sf"] == 1)
        {
            let mode = report["mode"].as_str().unwrap();
            if report["status"] != "accepted" {
                // These opaque String predicates retain a valuation that the
                // current PackedCanon catalog cannot canonicalize.
                assert!(
                    matches!(query, "h07" | "h14" | "h19"),
                    "{query} {mode}: {}; term: {}",
                    report["error"], report["term"]
                );
                assert!(
                    report["error"].as_str().unwrap().contains("unmet catalog obligation Canon"),
                    "{query} {mode}: {}",
                    report["error"]
                );
                continue;
            }
            let manifest = Manifest::load(&data_root().join("sf1"))?;
            let measurement = generated::execute(query, &manifest, mode, true)?
                .ok_or("accepted conformance executor missing")?;
            assert_eq!(measurement["conformance"]["status"], "passed");
        }
        Ok(())
    }

    macro_rules! conformance {
        ($($query:ident),* $(,)?) => { $(
            #[test]
            fn $query() -> Result<()> { assert_conformant(stringify!($query)) }
        )* };
    }
    conformance!(
        h01,
        h02,
        h11,
        h13,
        h15,
        h18,
        h20,
        h06,
        h10,
        h12,
        h14,
        h19,
        h03,
        h05,
        h07,
        h07_block,
        h08,
        h09,
        h21_positive,
        d19,
        d27,
        d72_inner,
        d85
    );

    #[test]
    fn accepted_schedule_artifacts_replay_exactly() -> Result<()> {
        let reports = plans()?;
        let mut artifacts = Vec::new();
        for report in reports.iter().filter(|report| report["status"] == "accepted") {
            if let Some(stages) = report["stages"].as_array() {
                artifacts.extend(stages);
            } else {
                artifacts.push(report);
            }
        }
        for report in artifacts {
            let mut current: cq_lang_sequence::Program =
                syn::parse_str(report["sequence_input"].as_str().unwrap())?;
            for invocation in report["pipeline"]["sequence"].as_array().unwrap() {
                let advice: cq_sequence_plan::Advice =
                    syn::parse_str(invocation.as_str().unwrap())?;
                current = advice.apply(&current)?;
            }
            let expected: cq_lang_sequence::Program =
                syn::parse_str(report["sequence_planned"].as_str().unwrap())?;
            assert_eq!(
                current, expected,
                "{} SF{} {}",
                report["query"], report["sf"], report["mode"]
            );
        }
        Ok(())
    }

    #[test]
    fn advised_h05_records_the_reference_boundaries() -> Result<()> {
        for sf in [1, 10] {
            let reports = plans()?;
            let report = reports
                .iter()
                .find(|row| row["query"] == "h05" && row["sf"] == sf && row["mode"] == "advised")
                .unwrap();
            assert_eq!(report["status"], "accepted", "{}", report["error"]);
            let advice = report["pipeline"]["sequence"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| syn::parse_str::<cq_sequence_plan::Advice>(value.as_str().unwrap()))
                .collect::<syn::Result<Vec<_>>>()?;
            assert_eq!(
                advice
                    .iter()
                    .filter(|step| matches!(
                        step,
                        cq_sequence_plan::Advice::Segment {
                            n: 1024,
                            items: Some(4),
                            ..
                        }
                    ))
                    .count(),
                2
            );
            assert_eq!(
                advice
                    .iter()
                    .filter(|step| matches!(
                        step,
                        cq_sequence_plan::Advice::Partition {
                            n: 256,
                            items: None,
                            ..
                        }
                    ))
                    .count(),
                1
            );
            assert!(
                report["pipeline"]["physical"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|value| value.as_str().unwrap().starts_with("retain_streams"))
            );
        }
        Ok(())
    }
}
