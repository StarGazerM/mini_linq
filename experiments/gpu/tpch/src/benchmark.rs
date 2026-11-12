use crate::manifest::{Manifest, Result};
use cq_testkit::conformance::assert_answers_equal;
use serde_json::{Value, json};
use bench_check::{Mode, Policy, check_relation, make_witness, write_witness};
use std::{collections::BTreeSet, fs, path::PathBuf, time::Instant};

pub const WARMUP_RUNS: usize = 1;
pub const MEASURED_RUNS: usize = 5;

/// Populated by the executed accounting paths, rather than a report label.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct TimingBoundary {
    pub stage_phase_sums: usize,
    pub final_downloads_excluded: usize,
    pub intermediate_downloads_included: usize,
}

#[derive(Default)]
pub struct Measurement {
    pub timings: cq_gpu_runtime::Timings,
    pub boundary: TimingBoundary,
}

impl Measurement {
    pub fn add_stage(&mut self, timing: cq_gpu_runtime::Timings, final_download: bool) {
        self.boundary.stage_phase_sums += 1;
        self.timings.upload_ms += timing.upload_ms;
        self.timings.build_ms += timing.build_ms;
        self.timings.kernel_ms += timing.kernel_ms;
        self.timings.dedup_ms += timing.dedup_ms;
        self.timings.total_ms += timing.total_ms;
        self.timings.wall_ms += timing.wall_ms;
        if final_download {
            self.timings.download_ms += timing.download_ms;
            self.boundary.final_downloads_excluded += 1;
        } else {
            // A download needed by another stage remains query execution work.
            self.timings.build_ms += timing.download_ms;
            self.timings.total_ms += timing.download_ms;
            self.boundary.intermediate_downloads_included += 1;
        }
    }

    pub fn final_download<T>(&mut self, download: impl FnOnce() -> Result<T>) -> Result<T> {
        let started = std::time::Instant::now();
        let result = download()?;
        let elapsed = started.elapsed().as_secs_f64() * 1000.0;
        self.timings.download_ms += elapsed;
        self.timings.wall_ms += elapsed;
        self.boundary.final_downloads_excluded += 1;
        Ok(result)
    }
}

pub fn answer(manifest: &Manifest, query: &str) -> Result<Vec<Vec<String>>> {
    let path = manifest.root.join(format!("answer.retail_{query}.tsv"));
    let mut rows = csv::ReaderBuilder::new()
        .has_headers(false)
        .delimiter(b'\t')
        .from_path(path)?
        .records()
        .map(|record| Ok(record?.iter().map(str::to_owned).collect::<Vec<_>>()))
        .collect::<Result<Vec<_>>>()?;
    let count: usize =
        fs::read_to_string(manifest.root.join(format!("answer.retail_{query}.count")))?
            .trim()
            .parse()?;
    if rows.len() != count {
        return Err("answer TSV and count disagree".into());
    }
    rows.sort_unstable();
    if rows.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("SELECT DISTINCT oracle contains duplicate rows".into());
    }
    Ok(rows)
}


pub fn run<T: Ord>(
    manifest: &Manifest,
    query: &str,
    mode: &str,
    mut execute: impl FnMut() -> Result<(Vec<T>, TimingBoundary)>,
    decode: impl Fn(&T) -> Vec<String>,
    reference: Option<Vec<Vec<String>>>,
    reference_seconds: f64,
    conformance_only: bool,
) -> Result<Value> {
    let reference_started = Instant::now();
    let expected: BTreeSet<_> = answer(manifest, query)?.into_iter().collect();
    let reference_seconds = reference_seconds + reference_started.elapsed().as_secs_f64();
    cq_gpu_sequence_runtime::index_bytes::begin();
    let (mut warmup, boundary) = execute()?;
    let index_bytes: Vec<_> = cq_gpu_sequence_runtime::index_bytes::finish()
        .into_iter().enumerate().map(|(ordinal, record)| json!({
            "ordinal": ordinal, "kind": record.kind,
            "allocated_bytes": record.allocated, "live_bytes": record.live,
        })).collect();
    let publication_bounds: Vec<_> = cq_gpu_sequence_runtime::index_bytes::take_publications()
        .into_iter().map(|(producer, bounds)| json!({
            "producer": producer,
            "columns": bounds.into_iter().map(|bounds| match bounds {
                gpu_device_columns::Bounds::Empty => Value::Null,
                gpu_device_columns::Bounds::Inclusive { min, max } => json!({
                    "minimum": min, "maximum": max,
                    "bits": 64 - ((i128::from(max) - i128::from(min)) as u64).leading_zeros(),
                }),
            }).collect::<Vec<_>>(),
        })).collect();
    let warmup_started = Instant::now();
    warmup.sort_unstable();
    warmup.dedup();
    let warmup_check = check_relation(&expected, query, warmup.len(), |i| decode(&warmup[i]), Policy::full())?;
    let mut validation_seconds = warmup_started.elapsed().as_secs_f64();
    let conformance_started = Instant::now();
    let conformance = if let Some(reference) = reference {
        // This is the external-target cq-testkit API used by H05. The generic
        // assert_conformant driver only exposes Sequential/Tasks targets.
        assert_answers_equal(
            "Sequential of the same Physical",
            reference,
            "planned CUDA Sequence",
            warmup.iter().map(&decode),
            true,
        );
        json!({"status": "passed", "api": "cq_testkit::conformance::assert_answers_equal", "sf": 1})
    } else {
        Value::Null
    };
    validation_seconds += reference_seconds + conformance_started.elapsed().as_secs_f64();
    drop(warmup);
    let mut samples = Vec::new();
    let mut full_samples = 0;
    if !conformance_only {
        for iteration in 0..MEASURED_RUNS {
            cq_gpu_runtime::take_last_timings();
            let (mut actual, measured_boundary) = execute()?;
            if measured_boundary != boundary {
                return Err("benchmark timing boundary changed between iterations".into());
            }
            let timing = cq_gpu_runtime::take_last_timings()
                .ok_or("GPU translation published no phase timings")?;
            let started = Instant::now();
            actual.sort_unstable();
            actual.dedup();
            let policy = if cfg!(test) { Policy::full() } else { Policy::from_env(iteration)? };
            let evidence = check_relation(&expected, query, actual.len(), |i| decode(&actual[i]), policy)?;
            full_samples += usize::from(policy.mode == Mode::Full);
            if let Some(directory) = std::env::var_os("BENCH_CHECK_WITNESS_DIR") {
                let directory = PathBuf::from(directory).join(format!("sf{}", manifest.sf)).join(mode);
                fs::create_dir_all(&directory)?;
                let witness = [(query.to_owned(), make_witness(query, actual.len(), |i| decode(&actual[i]), policy))].into();
                write_witness(&directory.join(format!("{query}-{iteration}.json")), &witness)?;
            }
            let check_seconds = started.elapsed().as_secs_f64();
            validation_seconds += check_seconds;
            samples.push(json!({"build_ms": timing.build_ms, "kernels_ms": timing.kernel_ms,
                "dedup_ms": timing.dedup_ms, "download_ms": timing.download_ms,
                "total_ms": timing.total_ms, "wall_ms": timing.wall_ms,
                "check": evidence, "validation_seconds": check_seconds}));
        }
    }
    let median = |key: &str| -> Value {
        if samples.is_empty() {
            return Value::Null;
        }
        let mut values: Vec<_> = samples
            .iter()
            .map(|sample| sample[key].as_f64().unwrap())
            .collect();
        values.sort_by(f64::total_cmp);
        json!(values[values.len() / 2])
    };
    Ok(
        json!({"status": "ran", "exact": conformance_only || full_samples == MEASURED_RUNS, "validated": true,
        "index_bytes": index_bytes,
        "publication_bounds": publication_bounds,
        "validation": {"policy": if conformance_only || full_samples == MEASURED_RUNS { "full" } else if full_samples > 0 { "full_then_sampled" } else { "sampled" },
            "full_samples": full_samples, "warmup": warmup_check, "seconds": validation_seconds,
            "reference_seconds": reference_seconds},
        "answer_rows": expected.len(), "conformance": conformance,
        "median_ms": {"total": median("total_ms"), "build": median("build_ms"), "kernels": median("kernels_ms"), "dedup": median("dedup_ms"), "download": median("download_ms")},
        "timing_boundary": boundary, "samples": samples}),
    )
}

pub fn duckdb(manifest: &Manifest, query: &str) -> Result<Value> {
    let path = manifest
        .root
        .join(format!("duckdb_timing.retail_{query}.json"));
    let timing: Value = serde_json::from_slice(&fs::read(&path)?)?;
    if timing["units"] != "seconds" {
        return Err("DuckDB timing units must be seconds".into());
    }
    let median = timing["threads"]["24"]["median_seconds"]
        .as_f64()
        .ok_or("missing DuckDB t24 median")?;
    Ok(json!({"t24_median_ms": median * 1000.0, "source": path, "measurement": timing}))
}

pub fn table(results: &[Value]) -> String {
    let mut table = String::from(
        "| query | mode | scale | validation | GPU total ms (build/kernels/dedup) | DuckDB t24 ms | status |\n|---|---|---|---|---|---|---|\n",
    );
    for row in results {
        let query = row["query"].as_str().unwrap();
        let mode = row["mode"].as_str().unwrap();
        let sf = row["sf"].as_u64().unwrap();
        let validation = row["validation"]["policy"].as_str().unwrap_or_else(|| match row["exact"].as_bool() {
            Some(true) => "full",
            Some(false) => "FAILED",
            None => "not run",
        });
        let timing = &row["median_ms"];
        let gpu = if let Some(total) = timing["total"].as_f64() {
            format!(
                "{total:.3} ({:.3}/{:.3}/{:.3})",
                timing["build"].as_f64().unwrap(),
                timing["kernels"].as_f64().unwrap(),
                timing["dedup"].as_f64().unwrap()
            )
        } else {
            "—".into()
        };
        let status = if let Some(error) = row["error"].as_str() {
            format!(
                "{}: {}",
                row["status"].as_str().unwrap(),
                error.replace('|', "\\|").replace('\n', " ")
            )
        } else {
            row["status"].as_str().unwrap().to_owned()
        };
        table.push_str(&format!(
            "| {query} | {mode} | SF{sf} | {} | {gpu} | {:.3} | {status} |\n",
            validation,
            row["duckdb"]["t24_median_ms"].as_f64().unwrap()
        ));
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_terminal_download_is_outside_query_phases() {
        let stage = cq_gpu_runtime::Timings {
            build_ms: 2.0, kernel_ms: 3.0, dedup_ms: 1.0,
            download_ms: 7.0, total_ms: 6.0, wall_ms: 14.0,
            ..Default::default()
        };
        let mut final_stage = Measurement::default();
        final_stage.add_stage(stage, true);
        assert_eq!(final_stage.timings.build_ms, 2.0);
        assert_eq!(final_stage.timings.total_ms, 6.0);
        assert_eq!(final_stage.timings.download_ms, 7.0);
        assert_eq!(final_stage.boundary.final_downloads_excluded, 1);
        let mut intermediate = Measurement::default();
        intermediate.add_stage(stage, false);
        assert_eq!(intermediate.timings.build_ms, 9.0);
        assert_eq!(intermediate.timings.total_ms, 13.0);
        assert_eq!(intermediate.timings.download_ms, 0.0);
        assert_eq!(intermediate.boundary.intermediate_downloads_included, 1);
        assert_eq!(intermediate.timings.wall_ms, final_stage.timings.wall_ms);
    }

    #[test]
    fn host_built_input_queries_disclose_load_side_upload_boundary() -> Result<()> {
        // Inspect the selected executors, as the conformance fixtures do.
        for report in crate::plans()?.iter().filter(|report| {
            matches!(report["query"].as_str(), Some("h13" | "h04_not_exists"))
        }) {
            assert_eq!(report["status"], "accepted", "{}", report["error"]);
            let note = report["timing"].as_str().expect("stage timing metadata");
            assert!(note.contains("NOTE: host-built input upload/normalization is load-side"));
            assert!(note.contains("host_input_upload_ms, separate from base domain_ms"));
            assert!(note.contains("unlike base e4e08c99"));
            assert!(note.contains("upload occurred in execute"));
            assert!(note.contains("authored host preprocessing remains load-side"));
            assert!(note.contains("indexes remain timed"));
        }
        Ok(())
    }

    #[test]
    fn filtered_table_keeps_each_mode_and_needs_no_other_scale() {
        let rows = ["automatic", "advised"].map(|mode| {
            json!({
                "query": "h05", "mode": mode, "sf": 10, "status": "ran", "exact": true,
                "median_ms": {"total": 10.0, "build": 2.0, "kernels": 7.0, "dedup": 1.0},
                "duckdb": {"t24_median_ms": 172.0}
            })
        });
        let rendered = table(&rows);
        assert!(rendered.contains("| h05 | automatic | SF10 | full |"));
        assert!(rendered.contains("| h05 | advised | SF10 | full |"));
        assert!(!rendered.contains("SF1 |"));
    }

    #[test]
    fn sampled_table_does_not_claim_full_exactness() {
        let row = json!({
            "query": "h05", "mode": "advised", "sf": 10, "status": "ran",
            "exact": false, "validated": true,
            "validation": {"policy": "full_then_sampled"},
            "median_ms": {"total": 10.0, "build": 2.0, "kernels": 7.0, "dedup": 1.0},
            "duckdb": {"t24_median_ms": 172.0}
        });
        let rendered = table(&[row]);
        assert!(rendered.contains("| h05 | advised | SF10 | full_then_sampled |"));
        assert!(!rendered.contains("FAILED"));
    }
}
