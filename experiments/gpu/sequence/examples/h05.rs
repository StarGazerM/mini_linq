//! Execute the public Sequence translator, never a hand-written query kernel.
//! From experiments/gpu, build with the installed CUDA toolkit and cuda-oxide:
//! `cargo oxide build --arch sm_89 -- --release -p cq-gpu-sequence --features cuda-oracle --example h05`
//! Then run against the same files as h05-compiled:
//! `target/release/examples/h05 /data/tpch-gpu/sf1`
//! `target/release/examples/h05 /data/tpch-gpu/sf10`
//! For retry correctness, rebuild with `--features cuda-retry-oracle` instead:
//! it records capacity 1024 instead of the benchmark's explicit 1048576.
#![recursion_limit = "1024"]

use cq_gpu_runtime::{DeviceColumns, Pool};
use cq_gpu_sequence_runtime::DenseDictionary;
use gpu_compress::{Catalog, Source, tpch::schema};
use serde_json::{Value, json};
use std::{error::Error, fs, path::Path, sync::Arc, time::Instant};

#[allow(dead_code)]
#[path = "../tests/support/h05_program.rs"]
mod h05_program;
#[path = "../tests/support/h05_rows.rs"]
mod h05_rows;
use h05_rows::Rows;

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
type Answer = (u32, u32, u32);

macro_rules! recorded_query {
    ($capacity:literal) => {
        const OUTPUT_CAPACITY: usize = $capacity;
        fn query<'a>(data: &'a Rows) -> DeviceColumns<'a, Answer> {
            cq_gpu_sequence_test_macros::h05!($capacity)
        }
    };
}

#[cfg(not(feature = "cuda-retry-oracle"))]
recorded_query!(1048576);
#[cfg(feature = "cuda-retry-oracle")]
recorded_query!(1024);

fn load_answer(directory: &Path) -> Result<Vec<(i64, i64, i64)>> {
    let mut answer = Vec::new();
    for (line_number, line) in fs::read_to_string(directory.join("answer.tsv"))?
        .lines()
        .enumerate()
    {
        let fields = line
            .split('\t')
            .map(str::parse::<i64>)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let [order, supplier, nation] = fields[..] else {
            return Err(format!(
                "answer.tsv line {} must contain three columns",
                line_number + 1
            )
            .into());
        };
        answer.push((order, supplier, nation));
    }
    answer.sort_unstable();
    answer.dedup();
    Ok(answer)
}

fn metric(samples: &[f64]) -> Value {
    let mut ordered = samples.to_vec();
    ordered.sort_by(f64::total_cmp);
    json!({"median": ordered[ordered.len() / 2], "min": ordered[0]})
}

fn main() -> Result<()> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.len() > 2 {
        return Err("usage: h05 [DATA_DIR] [RESULTS_JSON]".into());
    }
    let directory = Path::new(
        arguments
            .first()
            .map(String::as_str)
            .unwrap_or("/data/tpch-gpu/sf1"),
    );
    let result_path = Path::new(
        arguments
            .get(1)
            .map(String::as_str)
            .unwrap_or("compare/gpu_h05_sequence.resident.results.json"),
    );
    let load_started = Instant::now();
    let source = Source::load_manifest(directory)?;
    let answer = load_answer(directory)?;
    let host_load_ms = load_started.elapsed().as_secs_f64() * 1000.0;
    let manifest = source.manifest();
    let sf = manifest["sf"].clone();
    let constant = |name: &str| -> Result<i32> {
        Ok(i32::try_from(
            manifest["constants"][name]
                .as_i64()
                .ok_or_else(|| format!("missing manifest constant {name}"))?,
        )?)
    };
    if (constant("date_start_days")?, constant("date_end_days")?)
        != (h05_program::DATE_START, h05_program::DATE_END)
    {
        return Err("H05 date literals differ from the source manifest".into());
    }
    let encode_started = Instant::now();
    let pool: Arc<Pool> = Pool::new(0, 0, 0)?;
    let catalog = Catalog::compress(&pool, &source, &schema())?;
    let rows = Rows::bind(&catalog)?;
    let asia = catalog.text_id("region.r_name", "ASIA")?;
    if asia != h05_program::ASIA || asia != constant("asia_id")? {
        return Err(
            "H05 ASIA literal differs from the compressed text dictionary or manifest".into(),
        );
    }
    let load_encode_ms = encode_started.elapsed().as_secs_f64() * 1000.0;
    let declared_bytes = source.declared_bytes();
    drop(source);
    let orders = DenseDictionary::from_catalog(&catalog, "orderkey")?;
    let suppliers = DenseDictionary::from_catalog(&catalog, "suppkey")?;
    let nations = DenseDictionary::from_catalog(&catalog, "nationkey")?;
    println!(
        "sf={sf} host_load_ms={host_load_ms:.6} load_encode_ms={load_encode_ms:.6} declared_bytes={declared_bytes} compressed_bytes={}",
        catalog.bytes()
    );

    let mut build = Vec::new();
    let mut kernel = Vec::new();
    let mut dedup = Vec::new();
    let mut total = Vec::new();
    let mut wall = Vec::new();
    let mut calls = Vec::new();
    for iteration in 0..6 {
        let started = Instant::now();
        let resident = query(&rows);
        let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
        let timing = cq_gpu_runtime::take_last_timings()
            .ok_or("Sequence translator did not publish build/kernel/dedup/total timings")?;
        let download_started = Instant::now();
        let mut actual = resident
            .to_host()?
            .into_iter()
            .map(|(order, supplier, nation)| {
                Ok((
                    orders
                        .decode(order)
                        .ok_or("generated order ID is outside its dictionary")?,
                    suppliers
                        .decode(supplier)
                        .ok_or("generated supplier ID is outside its dictionary")?,
                    nations
                        .decode(nation)
                        .ok_or("generated nation ID is outside its dictionary")?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let download_ms = download_started.elapsed().as_secs_f64() * 1000.0;
        actual.sort_unstable();
        // Do not deduplicate actual: duplicates would hide a broken canon.
        if actual != answer {
            let first_mismatch = actual
                .iter()
                .zip(&answer)
                .position(|(a, b)| a != b)
                .unwrap_or(actual.len().min(answer.len()));
            return Err(format!("generated Sequence H05 differs from answer.tsv: actual={}, expected={}, first mismatch={first_mismatch}, actual_row={:?}, expected_row={:?}", actual.len(), answer.len(), actual.get(first_mismatch), answer.get(first_mismatch)).into());
        }
        println!(
            "sf={sf} iteration={iteration} exact=true rows={} build_ms={:.6} kernel_ms={:.6} dedup_ms={:.6} total_ms={:.6} wall_ms={wall_ms:.6} download_ms={download_ms:.6}",
            actual.len(),
            timing.build_ms,
            timing.kernel_ms,
            timing.dedup_ms,
            timing.total_ms
        );
        calls.push(json!({
            "iteration": iteration, "warmup": iteration == 0, "exact": true,
            "build_ms": timing.build_ms, "kernel_ms": timing.kernel_ms,
            "dedup_ms": timing.dedup_ms, "total_ms": timing.total_ms,
            "wall_ms": wall_ms, "download_ms": download_ms,
            "raw_bytes": resident.raw_bytes(), "fastlanes_bytes": resident.encoded_bytes()
        }));
        if iteration > 0 {
            build.push(timing.build_ms as f64);
            kernel.push(timing.kernel_ms as f64);
            dedup.push(timing.dedup_ms as f64);
            total.push(timing.total_ms as f64);
            wall.push(wall_ms);
        }
    }
    let result = json!({
        "implementation": "cq-gpu-sequence/translate", "sf": sf, "plan": "R-N-C-O-L-S",
        "exact": true, "answer_count": answer.len(), "warmups": 1, "runs": 5,
        "output_capacity": OUTPUT_CAPACITY,
        "mode": if cfg!(feature = "cuda-retry-oracle") { "retry-correctness" } else { "benchmark" },
        "build_ms": metric(&build), "kernel_ms": metric(&kernel), "dedup_ms": metric(&dedup),
        "total_ms": metric(&total), "wall_ms": metric(&wall),
        "one_time": {"host_load_ms": host_load_ms, "load_encode_ms": load_encode_ms,
            "declared_bytes": declared_bytes, "compressed_bytes": catalog.bytes()},
        "timing_scope": "Runtime GPU build/kernel/dedup/total timings cover resident input through canonical resident output. One-time manifest/oracle loading and compression, and final answer download, are separate. Provider stores are rebuilt on every call.",
        "calls": calls,
        "samples": {"build_ms": build, "kernel_ms": kernel, "dedup_ms": dedup, "total_ms": total, "wall_ms": wall}
    });
    let mut results: Vec<Value> = if result_path.exists() {
        serde_json::from_slice(&fs::read(result_path)?)?
    } else {
        Vec::new()
    };
    results.retain(|old| {
        old["implementation"] != result["implementation"]
            || old["sf"] != result["sf"]
            || old["mode"] != result["mode"]
    });
    results.push(result);
    if let Some(parent) = result_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(result_path, serde_json::to_vec_pretty(&results)?)?;
    Ok(())
}
