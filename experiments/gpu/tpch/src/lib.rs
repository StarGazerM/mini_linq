#![recursion_limit = "1024"]

pub mod benchmark;
pub mod loader;
pub mod manifest;
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

pub fn run_all(output: &Path) -> Result<bool> {
    let mut results = plans()?;
    let mut successful = true;
    for result in &mut results {
        let query = result["query"].as_str().unwrap().to_owned();
        let sf = result["sf"].as_u64().unwrap();
        let manifest = Manifest::load(&data_root().join(format!("sf{sf}")))?;
        result["duckdb"] = benchmark::duckdb(&manifest, &query)?;
        if result["status"] == "accepted" {
            // A runtime failure is neither a planning rejection nor a passing
            // measurement. Preserve it, continue the inventory, then fail main.
            let execution = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                generated::execute(&query, &manifest, false)
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
            result["exact"] = Value::Null;
            result["conformance"] =
                json!({"status": "not run", "reason": result["status"], "error": result["error"]});
        }
    }
    let table = benchmark::table(&results);
    println!("{table}");
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        output,
        serde_json::to_vec_pretty(&json!({
            "schema_version": 1,
            "data_root": data_root(),
            "warmup_runs": benchmark::WARMUP_RUNS,
            "measured_runs": benchmark::MEASURED_RUNS,
            "timing": "CUDA event medians; resident inputs; build + kernels + final dedup; loading, answer download and comparison excluded",
            "source": "CPU retail_kernel invocations imported verbatim with include_str; only manifest storage types/literals adapted",
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
mod tests {
    use super::*;

    fn assert_conformant(query: &str) -> Result<()> {
        let reports = plans()?;
        let report = reports
            .iter()
            .find(|report| report["query"] == query && report["sf"] == 1)
            .unwrap();
        if report["status"] != "accepted" {
            eprintln!(
                "NOT RUN {query}: {}: {}; term: {}",
                report["status"], report["error"], report["term"]
            );
            return Ok(());
        }
        let manifest = Manifest::load(&data_root().join("sf1"))?;
        let measurement = generated::execute(query, &manifest, true)?
            .ok_or("accepted conformance executor missing")?;
        assert_eq!(measurement["conformance"]["status"], "passed");
        Ok(())
    }

    macro_rules! conformance {
        ($($query:ident),* $(,)?) => { $(
            #[test]
            fn $query() -> Result<()> { assert_conformant(stringify!($query)) }
        )* };
    }
    conformance!(
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
}
