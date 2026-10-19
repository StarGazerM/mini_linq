#![recursion_limit = "1024"]

#[path = "../../h05/src/data.rs"]
mod data;
mod queries;

use cq_gpu_runtime::{
    detect_integer_codec, Codec, DeviceColumns, Dictionary, OwnedRowCodec, Runtime,
};
use data::{Data, Error};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fs, path::Path, time::Instant};

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

struct Rows {
    customer: DeviceColumns<'static, (i64, i64)>,
    orders: DeviceColumns<'static, (i64, i64, i64)>,
    lineitem: DeviceColumns<'static, (i64, i64)>,
    supplier: DeviceColumns<'static, (i64, i64)>,
    nation: DeviceColumns<'static, (i64, i64)>,
    region: DeviceColumns<'static, (i64, String)>,
}

impl Rows {
    fn load(data: &Data, dir: &Path) -> Result<(Self, Value), Error> {
        if (data.start, data.end) != (8766, 9131) {
            return Err("H05 query constants disagree with manifest date window".into());
        }
        let runtime = Runtime::new()?;
        let mut strings = Dictionary::default();
        let mut relations = Vec::new();
        let pairs = |left: &str, right: &str| -> Vec<(i64, i64)> {
            data.col(left)
                .iter()
                .copied()
                .zip(data.col(right).iter().copied())
                .collect()
        };
        let mut dictionary = BTreeMap::new();
        for (position, line) in fs::read_to_string(dir.join("region.r_name.dict.txt"))?
            .lines()
            .enumerate()
        {
            let (id, text) = match line.split_once('\t') {
                Some((id, text)) => (id.parse::<i64>()?, text),
                None => (i64::try_from(position)?, line),
            };
            dictionary.insert(id, text.to_owned());
        }
        if dictionary.get(&data.asia).map(String::as_str) != Some("ASIA") {
            return Err("ASIA dictionary constant disagrees with manifest".into());
        }
        let region = data
            .col("region.r_regionkey")
            .iter()
            .zip(data.col("region.r_name"))
            .map(|(&key, id)| {
                Ok((
                    key,
                    dictionary
                        .get(id)
                        .ok_or("unknown region dictionary id")?
                        .clone(),
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let orders: Vec<_> = data
            .col("orders.o_orderkey")
            .iter()
            .zip(data.col("orders.o_custkey"))
            .zip(data.col("orders.o_orderdate"))
            .map(|((&order, &customer), &date)| (order, customer, date))
            .collect();
        let codec = |name: &str| detect_integer_codec(data.col(name));
        let customer = upload(
            &runtime,
            &mut strings,
            "customer",
            &pairs("customer.c_custkey", "customer.c_nationkey"),
            &[codec("customer.c_custkey"), codec("customer.c_nationkey")],
            &mut relations,
        )?;
        let orders = upload(
            &runtime,
            &mut strings,
            "orders",
            &orders,
            &[
                codec("orders.o_orderkey"),
                codec("orders.o_custkey"),
                codec("orders.o_orderdate"),
            ],
            &mut relations,
        )?;
        let lineitem = upload(
            &runtime,
            &mut strings,
            "lineitem",
            &pairs("lineitem.l_orderkey", "lineitem.l_suppkey"),
            &[codec("lineitem.l_orderkey"), codec("lineitem.l_suppkey")],
            &mut relations,
        )?;
        let supplier = upload(
            &runtime,
            &mut strings,
            "supplier",
            &pairs("supplier.s_suppkey", "supplier.s_nationkey"),
            &[codec("supplier.s_suppkey"), codec("supplier.s_nationkey")],
            &mut relations,
        )?;
        let nation = upload(
            &runtime,
            &mut strings,
            "nation",
            &pairs("nation.n_nationkey", "nation.n_regionkey"),
            &[codec("nation.n_nationkey"), codec("nation.n_regionkey")],
            &mut relations,
        )?;
        let region = upload(
            &runtime,
            &mut strings,
            "region",
            &region,
            &[codec("region.r_regionkey"), Codec::Dictionary],
            &mut relations,
        )?;
        // Each relation retains its pool/stream/context and dictionary domain.
        // Dropping this construction Runtime does not invalidate resident inputs.
        Ok((
            Self {
                customer,
                orders,
                lineitem,
                supplier,
                nation,
                region,
            },
            json!(relations),
        ))
    }
}

fn upload<Row: OwnedRowCodec>(
    runtime: &Runtime,
    dictionary: &mut Dictionary<'static>,
    relation: &str,
    rows: &[Row],
    codecs: &[Codec],
    measurements: &mut Vec<Value>,
) -> Result<DeviceColumns<'static, Row>, Error> {
    let started = Instant::now();
    let resident = runtime.upload_owned_rows(rows, dictionary, codecs)?;
    let load_encode_ms = started.elapsed().as_secs_f64() * 1000.0;
    measurements.push(json!({
        "relation": relation, "rows": rows.len(),
        "raw_bytes": resident.raw_bytes(),
        "fastlanes_bytes": resident.encoded_bytes(),
        "codecs": codecs.iter().map(|codec| format!("{codec:?}")).collect::<Vec<_>>(),
        "load_encode_ms": load_encode_ms
    }));
    Ok(resident)
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
    if !["A", "B", "both"].contains(&selected) {
        return Err("usage: gpu-h05-compiled [DATA_DIR] [A|B|both] [RESULTS_JSON]".into());
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
    let data = Data::load(dir)?;
    let host_load_ms = host_started.elapsed().as_secs_f64() * 1000.0;
    let load_started = Instant::now();
    let (rows, relations) = Rows::load(&data, dir)?;
    let load_encode_ms = load_started.elapsed().as_secs_f64() * 1000.0;
    println!("sf={} host_load_ms={host_load_ms:.6} load_encode_ms={load_encode_ms:.6} relations={relations}", data.sf);
    let mut results = Vec::new();
    for (plan, query) in [
        (
            "A",
            queries::plan_a
                as for<'a> fn(&'a Rows) -> DeviceColumns<'a, queries::Answer, cq_gpu_runtime::For>,
        ),
        (
            "B",
            queries::plan_b
                as for<'a> fn(&'a Rows) -> DeviceColumns<'a, queries::Answer, cq_gpu_runtime::For>,
        ),
    ] {
        if selected != "both" && selected != plan {
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
            let resident = query(&rows);
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
            let mut actual = chained.to_host()?;
            let download_ms = download_started.elapsed().as_secs_f64() * 1000.0;
            actual.sort_unstable();
            if actual != data.answer {
                return Err(format!("compiled Plan {plan} differs from answer.tsv: actual={}, expected={}, first mismatch={:?}",
                    actual.len(), data.answer.len(), actual.iter().zip(&data.answer).find(|(a,b)| a != b)).into());
            }
            println!(
                "sf={} plan={plan} iteration={iteration} exact=true rows={} build_ms={:.6} kernel_ms={:.6} dedup_ms={:.6} total_ms={:.6} wall_ms={wall_ms:.6} chain_wall_ms={chain_wall_ms:.6} download_ms={download_ms:.6}",
                data.sf,
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
            "implementation": "cq_gpu::print/resident", "sf": data.sf, "plan": plan,
            "exact": true, "answer_count": data.answer.len(), "warmups": 1, "runs": 5,
            "build_ms": metric(&build), "kernel_ms": metric(&kernel),
            "dedup_ms": metric(&dedup), "total_ms": metric(&total), "wall_ms": metric(&wall),
            "download_ms": metric(&downloads),
            "one_time": {
                "host_load_ms": host_load_ms, "load_encode_ms": load_encode_ms,
                "relations": relations,
                "scope": "host_load_ms reads projected files and oracle; load_encode_ms includes Runtime creation, host row preparation, one-time upload and completed device encoding; relation load_encode_ms measures upload_owned_rows only",
                "bytes_scope": "raw_bytes is logical i64/dictionary-ID column bytes; fastlanes_bytes includes encoded column data, vector headers and extent metadata, not allocator capacity or host dictionary strings"
            },
            "timing_scope": "build/kernel/dedup/total are runtime GPU timings; wall_ms is the first resident compiled call, excluding one-time input load, the separately recorded second compiled call, and explicit final download; calls includes warmup and measured calls",
            "chain": "H05 packed DeviceColumns -> compiled identity query -> explicit download; no tuple download between queries",
            "calls": calls,
            "samples": {"build_ms": build, "kernel_ms": kernel, "dedup_ms": dedup, "total_ms": total, "wall_ms": wall, "download_ms": downloads}
        }));
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
