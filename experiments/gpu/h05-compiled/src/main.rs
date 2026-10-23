#![recursion_limit = "1024"]
mod queries;
use cq_gpu_runtime::{DeviceColumns, For, Pool};
use gpu_compress::{tpch::schema, Catalog, Source};
use serde_json::{json, Value};
use std::{fs, path::Path, sync::Arc, time::Instant};

type Error = Box<dyn std::error::Error>;
#[link(name = "cuda")]
unsafe extern "C" {
    fn cuProfilerStart() -> u32;
    fn cuProfilerStop() -> u32;
}

fn profiler_boundary(start: bool) -> Result<(), Error> {
    let status = unsafe {
        if start {
            cuProfilerStart()
        } else {
            cuProfilerStop()
        }
    };
    if status == 0 {
        Ok(())
    } else {
        Err(format!("CUDA profiler boundary failed: {status}").into())
    }
}

/// Stored tables bound as resident relations (domain IDs, declared i32).
pub struct Rows {
    customer: DeviceColumns<'static, (u32, u32), For>,
    orders: DeviceColumns<'static, (u32, u32, i32), For>,
    lineitem: DeviceColumns<'static, (u32, u32), For>,
    supplier: DeviceColumns<'static, (u32, u32), For>,
    nation: DeviceColumns<'static, (u32, u32), For>,
    region: DeviceColumns<'static, (u32, i32), For>,
}

impl Rows {
    fn bind(catalog: &Catalog) -> Result<Self, Error> {
        Ok(Self {
            customer: catalog.device_columns("customer")?,
            orders: catalog.device_columns("orders")?,
            lineitem: catalog.device_columns("lineitem")?,
            supplier: catalog.device_columns("supplier")?,
            nation: catalog.device_columns("nation")?,
            region: catalog.device_columns("region")?,
        })
    }
}

/// Independent oracle: DuckDB's distinct answer over source keys.
fn load_answer(dir: &Path) -> Result<Vec<(i64, i64, i64)>, Error> {
    let mut answer = Vec::new();
    for line in fs::read_to_string(dir.join("answer.tsv"))?.lines() {
        let fields = line
            .split('\t')
            .map(str::parse::<i64>)
            .collect::<Result<Vec<_>, _>>()?;
        let [o, s, n] = fields[..] else {
            return Err("answer must have three columns".into());
        };
        answer.push((o, s, n));
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

fn main() -> Result<(), Error> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let dir = Path::new(
        args.first()
            .map(String::as_str)
            .unwrap_or("/data/tpch-gpu/sf1"),
    );
    let selected = args.get(1).map(String::as_str).unwrap_or("both");
    if ![
        "A",
        "B",
        "R",
        "planned-A",
        "planned-B",
        "planned-R",
        "both",
        "compare",
    ]
    .contains(&selected)
    {
        return Err("usage: gpu-h05-compiled [DATA_DIR] [A|B|R|planned-A|planned-B|planned-R|both|compare] [RESULTS_JSON]".into());
    }
    let result_path = Path::new(
        args.get(2)
            .map(String::as_str)
            .unwrap_or("compare/gpu_h05_compiled.resident.results.json"),
    );
    if !dir.join("answer.tsv").is_file() {
        return Err("compiled benchmark requires the independent answer.tsv oracle".into());
    }
    let profile = std::env::var_os("CQ_GPU_PROFILE").is_some();
    let host_started = Instant::now();
    let source = Source::load_manifest(dir)?;
    let answer = load_answer(dir)?;
    let host_load_ms = host_started.elapsed().as_secs_f64() * 1000.0;
    let manifest = source.manifest();
    let sf = manifest["sf"].clone();
    let constant = |name: &str| -> Result<i32, Error> {
        Ok(i32::try_from(
            manifest["constants"][name]
                .as_i64()
                .ok_or_else(|| format!("missing constant {name}"))?,
        )?)
    };
    if (constant("date_start_days")?, constant("date_end_days")?) != (8766, 9131) {
        return Err("H05 query constants disagree with manifest date window".into());
    }

    let load_started = Instant::now();
    let pool: Arc<Pool> = Pool::new(0, 0, 0)?;
    let catalog = Catalog::compress(&pool, &source, &schema())?;
    let rows = Rows::bind(&catalog)?;
    // The query's ASIA literal must be the stored text ID.
    let asia = catalog.text_id("region.r_name", "ASIA")?;
    if asia != queries::ASIA || asia != constant("asia_id")? {
        return Err(
            format!("ASIA stored ID {asia} disagrees with the query literal or manifest").into(),
        );
    }
    let load_encode_ms = load_started.elapsed().as_secs_f64() * 1000.0;
    let declared_bytes = source.declared_bytes();
    drop(source);
    let relations: Vec<Value> = catalog
        .tables()
        .map(|(table, stored)| {
            json!({"relation": table, "rows": stored.rows,
                "encoded_bytes": stored.columns.iter().map(|c| c.column.bytes()).sum::<usize>()})
        })
        .collect();
    let (orders, suppliers, nations) = (
        catalog.domain("orderkey")?,
        catalog.domain("suppkey")?,
        catalog.domain("nationkey")?,
    );
    println!(
        "sf={sf} host_load_ms={host_load_ms:.6} load_encode_ms={load_encode_ms:.6} declared_bytes={declared_bytes} compressed_bytes={}",
        catalog.bytes()
    );
    let mut results = Vec::new();
    type Query = Box<dyn for<'a> Fn(&'a Rows) -> queries::QueryResult<'a>>;
    let plans: [(&str, Query); 6] = [
        ("A", Box::new(queries::plan_a)),
        ("B", Box::new(queries::plan_b)),
        ("R", Box::new(queries::plan_r)),
        ("planned-A", Box::new(queries::planned_a)),
        ("planned-B", Box::new(queries::planned_b)),
        ("planned-R", Box::new(queries::planned_r)),
    ];
    for (plan, query) in plans {
        if selected != "compare"
            && selected != plan
            && !(selected == "both" && !plan.starts_with("planned-"))
        {
            continue;
        }
        let mut build = Vec::new();
        let mut kernel = Vec::new();
        let mut dedup = Vec::new();
        let mut total = Vec::new();
        let mut wall = Vec::new();
        let mut calls = Vec::new();
        let mut downloads = Vec::new();
        for iteration in 0..6 {
            let capture = profile && iteration == 1;
            if capture {
                profiler_boundary(true)?;
            }
            let started = Instant::now();
            let resident = query(&rows)?;
            let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
            let timing = cq_gpu_runtime::take_last_timings()
                .ok_or("printer supplied no execution timings")?;
            let chain_started = Instant::now();
            let chained = queries::chain(&resident);
            let chain_wall_ms = chain_started.elapsed().as_secs_f64() * 1000.0;
            let chain_timing = cq_gpu_runtime::take_last_timings()
                .ok_or("chained printer supplied no execution timings")?;
            if capture {
                profiler_boundary(false)?;
            }
            let download_started = Instant::now();
            let mut actual: Vec<(i64, i64, i64)> = chained
                .to_host()?
                .into_iter()
                .map(|(o, s, n)| (orders.value(o), suppliers.value(s), nations.value(n)))
                .collect();
            let download_ms = download_started.elapsed().as_secs_f64() * 1000.0;
            actual.sort_unstable();
            if actual != answer {
                return Err(format!("compiled Plan {plan} differs from answer.tsv: actual={}, expected={}, first mismatch={:?}",
                    actual.len(), answer.len(), actual.iter().zip(&answer).find(|(a,b)| a != b)).into());
            }
            println!(
                "sf={sf} plan={plan} iteration={iteration} exact=true rows={} build_ms={:.6} kernel_ms={:.6} dedup_ms={:.6} total_ms={:.6} wall_ms={wall_ms:.6} chain_wall_ms={chain_wall_ms:.6} download_ms={download_ms:.6}",
                actual.len(),
                timing.build_ms,
                timing.kernel_ms,
                timing.dedup_ms,
                timing.total_ms
            );
            calls.push(json!({
                "iteration": iteration, "warmup": iteration == 0, "exact": true,
                "query": {
                    "wall_ms": wall_ms, "build_ms": timing.build_ms,
                    "kernel_ms": timing.kernel_ms, "dedup_ms": timing.dedup_ms,
                    "total_ms": timing.total_ms,
                    "raw_bytes": resident.raw_bytes(),
                    "fastlanes_bytes": resident.encoded_bytes()
                },
                "chain": {
                    "wall_ms": chain_wall_ms, "build_ms": chain_timing.build_ms,
                    "kernel_ms": chain_timing.kernel_ms, "dedup_ms": chain_timing.dedup_ms,
                    "total_ms": chain_timing.total_ms
                },
                "download_ms": download_ms
            }));
            if iteration > 0 {
                build.push(timing.build_ms as f64);
                kernel.push(timing.kernel_ms as f64);
                dedup.push(timing.dedup_ms as f64);
                total.push(timing.total_ms as f64);
                wall.push(wall_ms);
                downloads.push(download_ms);
            }
        }
        results.push(json!({
            "implementation": "cq_gpu_sequence::translate/resident", "sf": sf, "plan": plan,
            "exact": true, "answer_count": answer.len(), "warmups": 1, "runs": 5,
            "build_ms": metric(&build), "kernel_ms": metric(&kernel),
            "dedup_ms": metric(&dedup), "total_ms": metric(&total), "wall_ms": metric(&wall),
            "download_ms": metric(&downloads),
            "one_time": {
                "host_load_ms": host_load_ms, "load_encode_ms": load_encode_ms,
                "relations": relations, "declared_bytes": declared_bytes, "compressed_bytes": catalog.bytes(),
                "scope": "host_load_ms reads manifest columns and the oracle; load_encode_ms is gpu-compress (dictionary IDs per domain, FOR per column) and resident binding",
                "bytes_scope": "declared_bytes is source columns at manifest types; compressed_bytes is resident FOR-encoded stored columns (domain IDs, i32)"
            },
            "timing_scope": "build/kernel/dedup/total are runtime GPU timings; wall_ms is the first resident compiled call, excluding one-time input load, the separately recorded second compiled call, and explicit final download; calls includes warmup and measured calls",
            "chain": "H05 packed DeviceColumns -> compiled identity query -> explicit download; no tuple download between queries",
            "calls": calls,
            "samples": {"build_ms": build, "kernel_ms": kernel, "dedup_ms": dedup, "total_ms": total, "wall_ms": wall, "download_ms": downloads}
        }));
    }
    // Compare the same loaded resident inputs and identical timing scopes.
    // Keep both raw runs even when the performance acceptance is not met.
    if selected == "compare" {
        for name in ["A", "B", "R"] {
            let authored = results
                .iter()
                .find(|result| result["plan"] == name)
                .unwrap();
            let authored_ms = authored["total_ms"]["median"].as_f64().unwrap();
            let planned_name = format!("planned-{name}");
            let planned = results
                .iter_mut()
                .find(|result| result["plan"] == planned_name)
                .unwrap();
            let planned_ms = planned["total_ms"]["median"].as_f64().unwrap();
            let within_ten_percent = planned_ms <= authored_ms * 1.10;
            planned["authored_comparison"] = json!({
                "plan": name,
                "authored_total_ms": authored_ms,
                "planned_total_ms": planned_ms,
                "within_ten_percent": within_ten_percent
            });
            println!("sf={sf} plan={name} authored_total_ms={authored_ms:.6} planned_total_ms={planned_ms:.6} within_ten_percent={within_ten_percent}");
        }
    }
    let mut existing: Vec<Value> = if result_path.exists() {
        serde_json::from_slice(&fs::read(result_path)?)?
    } else {
        Vec::new()
    };
    existing.retain(|old| {
        !results.iter().any(|new| {
            old["implementation"] == new["implementation"]
                && old["sf"] == new["sf"]
                && old["plan"] == new["plan"]
        })
    });
    existing.extend(results);
    if let Some(parent) = result_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(result_path, serde_json::to_vec_pretty(&existing)?)?;
    Ok(())
}
