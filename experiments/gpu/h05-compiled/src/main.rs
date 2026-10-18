#![recursion_limit = "1024"]

#[path = "../../h05/src/data.rs"]
mod data;
mod queries;

use data::{Data, Error};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fs, path::Path, time::Instant};

struct Rows {
    customer: Vec<(i64, i64)>,
    orders: Vec<(i64, i64, i64)>,
    lineitem: Vec<(i64, i64)>,
    supplier: Vec<(i64, i64)>,
    nation: Vec<(i64, i64)>,
    region: Vec<(i64, String)>,
}

impl Rows {
    fn load(data: &Data, dir: &Path) -> Result<Self, Error> {
        if (data.start, data.end) != (8766, 9131) {
            return Err("H05 query constants disagree with manifest date window".into());
        }
        let pairs = |left: &str, right: &str| {
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
            .collect::<Result<_, Error>>()?;
        Ok(Self {
            customer: pairs("customer.c_custkey", "customer.c_nationkey"),
            orders: data
                .col("orders.o_orderkey")
                .iter()
                .zip(data.col("orders.o_custkey"))
                .zip(data.col("orders.o_orderdate"))
                .map(|((&order, &customer), &date)| (order, customer, date))
                .collect(),
            lineitem: pairs("lineitem.l_orderkey", "lineitem.l_suppkey"),
            supplier: pairs("supplier.s_suppkey", "supplier.s_nationkey"),
            nation: pairs("nation.n_nationkey", "nation.n_regionkey"),
            region,
        })
    }
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
            .unwrap_or("compare/gpu_h05_compiled.results.json"),
    );
    if !dir.join("answer.tsv").is_file() {
        return Err("compiled benchmark requires the independent answer.tsv oracle".into());
    }
    let data = Data::load(dir)?;
    let rows = Rows::load(&data, dir)?;
    let mut results = Vec::new();
    for (plan, query) in [
        ("A", queries::plan_a as fn(&Rows) -> Vec<queries::Answer>),
        ("B", queries::plan_b as fn(&Rows) -> Vec<queries::Answer>),
    ] {
        if selected != "both" && selected != plan {
            continue;
        }
        let mut build = Vec::new();
        let mut kernel = Vec::new();
        let mut dedup = Vec::new();
        let mut total = Vec::new();
        let mut wall = Vec::new();
        for iteration in 0..6 {
            let started = Instant::now();
            let mut actual = query(&rows);
            let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
            let timing = cq_gpu_runtime::take_last_timings()
                .ok_or("printer supplied no execution timings")?;
            actual.sort_unstable();
            if actual != data.answer {
                return Err(format!("compiled Plan {plan} differs from answer.tsv: actual={}, expected={}, first mismatch={:?}",
                    actual.len(), data.answer.len(), actual.iter().zip(&data.answer).find(|(a,b)| a != b)).into());
            }
            println!(
                "sf={} plan={plan} iteration={iteration} exact=true rows={} build_ms={:.6} kernel_ms={:.6} dedup_ms={:.6} total_ms={:.6} wall_ms={wall_ms:.6}",
                data.sf,
                actual.len(),
                timing.build_ms,
                timing.kernel_ms,
                timing.dedup_ms,
                timing.total_ms
            );
            if iteration > 0 {
                build.push(timing.build_ms as f64);
                kernel.push(timing.kernel_ms as f64);
                dedup.push(timing.dedup_ms as f64);
                total.push(timing.total_ms as f64);
                wall.push(wall_ms);
            }
        }
        results.push(json!({
            "implementation": "cq_gpu::print", "sf": data.sf, "plan": plan,
            "exact": true, "answer_count": data.answer.len(), "warmups": 1, "runs": 5,
            "build_ms": metric(&build), "kernel_ms": metric(&kernel),
            "dedup_ms": metric(&dedup), "total_ms": metric(&total), "wall_ms": metric(&wall),
            "timing_scope": "GPU builds, query kernels and exact device dedup; host columnization, upload and answer download excluded; wall_ms includes all execution work",
            "samples": {"build_ms": build, "kernel_ms": kernel, "dedup_ms": dedup, "total_ms": total, "wall_ms": wall}
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
