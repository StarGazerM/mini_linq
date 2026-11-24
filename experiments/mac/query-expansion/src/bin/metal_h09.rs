//! H09 over one GPU-built Lineitem run: compiled default route (CPU probes)
//! versus the batched Metal Part→Lineitem join. Also isolates that expansion.
use cq_mac::MacContext;
use mac_query_expansion::{
    compiled_h09::{self, Construction, PreparedIndex},
    fixtures, h09,
    manual_h09::{Prepared, green_parts},
    metal_h09,
};
use serde_json::{Value, json};
use std::{error::Error, hint::black_box, path::PathBuf, time::Instant};
type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

struct Options {
    sizes: Vec<usize>,
    data: Vec<PathBuf>,
    runs: usize,
    out: Option<PathBuf>,
}

impl Options {
    fn parse() -> Result<Self> {
        let mut options = Self {
            sizes: Vec::new(),
            data: Vec::new(),
            runs: 7,
            out: None,
        };
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| format!("missing value for {flag}"))?;
            match flag.as_str() {
                "--sizes" => {
                    options.sizes = value
                        .split(',')
                        .map(str::parse)
                        .collect::<std::result::Result<_, _>>()?
                }
                "--data" => options.data.push(value.into()),
                "--runs" => options.runs = value.parse()?,
                "--out" => options.out = Some(value.into()),
                _ => return Err(format!("unknown option {flag}").into()),
            }
        }
        if options.runs < 3
            || options.runs.is_multiple_of(2)
            || (options.sizes.is_empty() && options.data.is_empty())
            || options.sizes.iter().any(|&n| n > cq_mac::backend::MAX_ROWS)
        {
            return Err(
                "need --sizes and/or --data; sizes must fit the Mac row domain; runs odd >= 3"
                    .into(),
            );
        }
        Ok(options)
    }
}

fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn cpu_stage(index: &PreparedIndex, probes: &[i64]) -> Vec<(u32, u32)> {
    let lineitem = index.lineitem().expect("green demand");
    let mut pairs = Vec::new();
    for (ordinal, &key) in probes.iter().enumerate() {
        for row_id in lineitem.row_ids(key).into_iter().flatten() {
            pairs.push((ordinal as u32, row_id));
        }
    }
    pairs
}

fn measure(context: &MacContext, label: &str, data: &h09::Data, runs: usize) -> Result<Value> {
    let mut expected = h09::compiled(data);
    expected.sort_unstable();
    let source = Prepared::new(context, data)?;
    let start = Instant::now();
    let index = compiled_h09::build(context, &source, Construction::Gpu)?;
    let build_ms = ms(start);
    let probes: Vec<i64> = green_parts(data).collect();
    let lineitem = index.lineitem().ok_or("dataset has no green-part demand")?;
    // Untimed exactness: both stages and both complete queries.
    let cpu_pairs = cpu_stage(&index, &probes);
    let gpu_pairs: Vec<_> = lineitem.join_row_ids(context, &probes)?.pairs().collect();
    assert_eq!(gpu_pairs, cpu_pairs, "{label}: Metal join pairs differ");
    let mut compiled = compiled_h09::execute(data, &index)?;
    compiled.sort_unstable();
    assert_eq!(compiled, expected, "{label}: compiled answers differ");
    assert_eq!(
        metal_h09::execute(context, data, &index)?,
        expected,
        "{label}: Metal-join answers differ"
    );
    assert_eq!(
        metal_h09::execute_on_cpu(data, &index)?,
        expected,
        "{label}: host-completion answers differ"
    );
    let resident = metal_h09::Resident::prepare(context, data)?;
    assert_eq!(
        metal_h09::execute_resident(context, &probes, &index, &resident)?,
        expected,
        "{label}: GPU-resident answers differ"
    );
    let [
        mut compiled_query,
        mut cpu_query,
        mut gpu_query,
        mut cpu_join,
        mut gpu_join,
        mut resident_query,
        mut resident_fresh,
    ] = [(); 7].map(|_| Vec::with_capacity(runs));
    for round in 0..runs + 2 {
        // Alternate which provider runs first; two rounds warm both paths.
        for cpu_first in [round % 2 == 0, round % 2 != 0] {
            if cpu_first {
                let start = Instant::now();
                let answers = compiled_h09::execute(data, &index)?;
                let compiled = ms(start);
                assert_eq!(answers.len(), expected.len());
                let start = Instant::now();
                let answers = metal_h09::execute_on_cpu(data, &index)?;
                let query = ms(start);
                assert_eq!(answers.len(), expected.len());
                let start = Instant::now();
                let pairs = cpu_stage(&index, &probes);
                let join = ms(start);
                assert_eq!(black_box(pairs).len(), cpu_pairs.len());
                if round >= 2 {
                    compiled_query.push(compiled);
                    cpu_query.push(query);
                    cpu_join.push(join);
                }
            } else {
                let start = Instant::now();
                let answers = metal_h09::execute(context, data, &index)?;
                let query = ms(start);
                assert_eq!(answers.len(), expected.len());
                let start = Instant::now();
                let pairs = lineitem.join_row_ids(context, &probes)?;
                let join = ms(start);
                assert_eq!(black_box(pairs).len(), cpu_pairs.len());
                let start = Instant::now();
                let answers = metal_h09::execute_resident(context, &probes, &index, &resident)?;
                let prepared = ms(start);
                assert_eq!(answers.len(), expected.len());
                let start = Instant::now();
                let fresh = metal_h09::Resident::prepare(context, data)?;
                let answers = metal_h09::execute_resident(context, &probes, &index, &fresh)?;
                let fresh_ms = ms(start);
                assert_eq!(answers.len(), expected.len());
                if round >= 2 {
                    gpu_query.push(query);
                    gpu_join.push(join);
                    resident_query.push(prepared);
                    resident_fresh.push(fresh_ms);
                }
            }
        }
    }
    let summary = json!({
        "dataset": label, "lineitem_rows": data.lineitem.len(), "probes": probes.len(),
        "join_pairs": cpu_pairs.len(), "answers": expected.len(), "gpu_build_ms": build_ms,
        "compiled_query_ms": median(compiled_query.clone()),
        "cpu_query_ms": median(cpu_query.clone()), "metal_query_ms": median(gpu_query.clone()),
        "cpu_join_ms": median(cpu_join.clone()), "metal_join_ms": median(gpu_join.clone()),
        "resident_query_ms": median(resident_query.clone()),
        "resident_with_builds_ms": median(resident_fresh.clone()),
        "compiled_query_samples": compiled_query,
        "cpu_query_samples": cpu_query, "metal_query_samples": gpu_query,
        "cpu_join_samples": cpu_join, "metal_join_samples": gpu_join,
        "resident_query_samples": resident_query, "resident_with_builds_samples": resident_fresh,
    });
    println!(
        "{label}\t{}\t{}\t{}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{:.3}",
        data.lineitem.len(),
        probes.len(),
        cpu_pairs.len(),
        summary["cpu_join_ms"].as_f64().unwrap(),
        summary["metal_join_ms"].as_f64().unwrap(),
        summary["compiled_query_ms"].as_f64().unwrap(),
        summary["cpu_query_ms"].as_f64().unwrap(),
        summary["metal_query_ms"].as_f64().unwrap(),
        summary["resident_query_ms"].as_f64().unwrap(),
        summary["resident_with_builds_ms"].as_f64().unwrap(),
    );
    Ok(summary)
}

fn main() -> Result<()> {
    let options = Options::parse()?;
    let context = MacContext::new()?;
    println!(
        "dataset\tlineitem\tprobes\tpairs\tcpu_join_ms\tmetal_join_ms\tcompiled_query_ms\tcpu_query_ms\tmetal_query_ms\tresident_query_ms\tresident_with_builds_ms"
    );
    let mut records = Vec::new();
    for &n in &options.sizes {
        records.push(measure(
            &context,
            &format!("synthetic-{n}"),
            &fixtures::h09_scaled(n),
            options.runs,
        )?);
    }
    for directory in &options.data {
        let data = h09::load(directory, "default")?;
        records.push(measure(
            &context,
            &directory.display().to_string(),
            &data,
            options.runs,
        )?);
    }
    let report = json!({
        "schema": "mccl.metal-join-h09.v1",
        "args": std::env::args().collect::<Vec<_>>(),
        "architecture": std::env::consts::ARCH,
        "warmups": 2, "runs": options.runs,
        "order": "alternating CPU-first/Metal-first pairs after two warmup rounds",
        "index": "one GPU-built grouped RadixRun over the retained i64 Lineitem source (l_partkey -> original row ID), built untimed and shared by both paths",
        "compiled": "compiled authored default route reading that run through its native l_partkey host read",
        "cpu": "the Metal path's identical host completion, with the Part->Lineitem expansion as CPU row_ids reads of the same run; CPU join stage = row_ids per green part key into (probe ordinal, row ID) pairs",
        "metal": "Part/GreenNames prefix on host; Part->Lineitem as one batched Metal join (binary-search ranks, exact u64 offsets, 2048-pair tiles); remaining joins on CPU with the default route's providers; Metal join stage includes probe upload, both synchronous GPU evaluations and pair publication",
        "resident": "One GPU graph after the join's pair-count readback: Metal join expansion, PartSupp (part key, supplier item), Orders and Supplier-selected-by-Nation semijoins with GPU-resident row counts, then GPU gather, LSD stable radix sort and dedup of (order, part, supplier); only distinct answers return (two host waits per query). The p_name LIKE '%green%' prefilter (unsupported query language) is evaluated once outside timing. resident_query reuses runs prepared once; resident_with_builds includes ingesting and GPU-building those three runs and the host Nation mask",
        "correctness": "complete answers equal the compiled default route and sorted; GPU pairs equal CPU pairs exactly, checked untimed before sampling; every sample checks counts",
        "limits": "query times exclude source ingestion and index build; remaining operators and canonicalization run on CPU; synthetic datasets are not dbgen",
        "results": records,
    });
    if let Some(path) = options.out {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        serde_json::to_writer_pretty(
            std::io::BufWriter::new(std::fs::File::create(&path)?),
            &report,
        )?;
        println!("Raw samples: {}", path.display());
    }
    println!("Metal-join H09: pairs and complete answers matched the compiled default route.");
    Ok(())
}
