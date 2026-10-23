use crate::manifest::{Manifest, Result};
use cq_testkit::conformance::assert_answers_equal;
use serde_json::{Value, json};
use std::fs;

pub const WARMUP_RUNS: usize = 1;
pub const MEASURED_RUNS: usize = 5;

pub fn answer(manifest: &Manifest, query: &str) -> Result<Vec<Vec<i64>>> {
    let path = manifest.root.join(format!("answer.retail_{query}.tsv"));
    let mut rows = fs::read_to_string(path)?
        .lines()
        .map(|line| {
            line.split('\t')
                .map(|field| field.parse::<i64>().map_err(Into::into))
                .collect::<Result<Vec<_>>>()
        })
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

fn exact(
    expected: &[Vec<i64>],
    actual: &mut [Vec<i64>],
    query: &str,
    sf: u32,
    iteration: usize,
) -> Result<()> {
    actual.sort_unstable();
    if actual != expected {
        let first = actual.iter().zip(expected).position(|(a, b)| a != b);
        return Err(format!("{query} SF{sf} iteration {iteration}: exact answer mismatch; expected {} rows, got {}; first differing row {first:?}", expected.len(), actual.len()).into());
    }
    Ok(())
}

pub fn run(
    manifest: &Manifest,
    query: &str,
    mut execute: impl FnMut() -> Result<Vec<Vec<i64>>>,
    reference: Option<Vec<Vec<i64>>>,
    conformance_only: bool,
) -> Result<Value> {
    let expected = answer(manifest, query)?;
    let mut warmup = execute()?;
    exact(&expected, &mut warmup, query, manifest.sf, 0)?;
    let conformance = if let Some(reference) = reference {
        // This is the external-target cq-testkit API used by H05. The generic
        // assert_conformant driver only exposes Sequential/Tasks targets.
        assert_answers_equal(
            "Sequential of the same Physical",
            reference,
            "planned CUDA Sequence",
            warmup.iter().cloned(),
            true,
        );
        json!({"status": "passed", "api": "cq_testkit::conformance::assert_answers_equal", "sf": 1})
    } else {
        Value::Null
    };
    let mut samples = Vec::new();
    if !conformance_only {
        for iteration in 1..=MEASURED_RUNS {
            cq_gpu_runtime::take_last_timings();
            let mut actual = execute()?;
            let timing = cq_gpu_runtime::take_last_timings()
                .ok_or("GPU translation published no phase timings")?;
            exact(&expected, &mut actual, query, manifest.sf, iteration)?;
            samples.push(json!({"build_ms": timing.build_ms, "kernels_ms": timing.kernel_ms,
                "dedup_ms": timing.dedup_ms, "total_ms": timing.total_ms, "wall_ms": timing.wall_ms}));
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
        json!({"status": "ran", "exact": true, "answer_rows": expected.len(), "conformance": conformance,
        "median_ms": {"total": median("total_ms"), "build": median("build_ms"), "kernels": median("kernels_ms"), "dedup": median("dedup_ms")},
        "samples": samples}),
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
        "| query | mode | scale | exact | GPU total ms (build/kernels/dedup) | DuckDB t24 ms | status |\n|---|---|---|---|---|---|---|\n",
    );
    for row in results {
        let query = row["query"].as_str().unwrap();
        let mode = row["mode"].as_str().unwrap();
        let sf = row["sf"].as_u64().unwrap();
        let exact = |row: &Value| match row["exact"].as_bool() {
            Some(true) => "yes",
            Some(false) => "NO",
            None => "not run",
        };
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
            exact(row),
            row["duckdb"]["t24_median_ms"].as_f64().unwrap()
        ));
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(rendered.contains("| h05 | automatic | SF10 | yes |"));
        assert!(rendered.contains("| h05 | advised | SF10 | yes |"));
        assert!(!rendered.contains("SF1 |"));
    }
}
