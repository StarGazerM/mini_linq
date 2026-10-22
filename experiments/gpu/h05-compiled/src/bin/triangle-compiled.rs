//! Directed triangles through the compiler: `order(le)` + Smallest + sorted
//! tries + recorded expansion. Input is canonicalized (sorted, distinct) on
//! the host like `gpu-triangles` and uploaded once; timing covers the compiled
//! query only (input set certificate, index builds, kernel), not upload.
use cq_gpu_runtime::{Codec, DeviceColumns, Dictionary, Runtime};
use std::{
    error::Error,
    io::{BufRead, BufReader},
    time::Instant,
};

fn load(path: &str) -> Result<Vec<(i64, i64)>, Box<dyn Error>> {
    let mut rows = Vec::new();
    for line in BufReader::with_capacity(1 << 20, std::fs::File::open(path)?).lines() {
        let line = line?;
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let mut fields = line.split_whitespace();
        let (Some(a), Some(b)) = (fields.next(), fields.next()) else {
            return Err(format!("malformed edge line: {line}").into());
        };
        rows.push((a.parse()?, b.parse()?));
    }
    rows.sort_unstable();
    rows.dedup();
    Ok(rows)
}

fn triangles<'a>(edges: &'a DeviceColumns<'_, (i64, i64)>) -> DeviceColumns<'a, (i64, i64, i64)> {
    mini_linq::compile! {
        pipeline {
            plan = ::cq_planners::order(le);
            physical = [::cq_rewrites::host_sets(E), ::cq_rewrites::build_host_reads,
                ::cq_rewrites::smallest_intersections];
            structures = [::cq_rewrites::sorted_runs, ::cq_rewrites::hashed_runs, ::cq_gpu::prefix_runs,
                ::cq_rewrites::retain_lookups, ::cq_rewrites::expand_lookups(256)];
            print = ::cq_gpu::print(tile(256,4), frontier(8), directory(rank), output(retry));
        }
        struct Triangle;
        relation E(src:i64,dst:i64) = (edges);
        answer(a,b,c) :- E(a,b), E(b,c), E(a,c);
    }
}

/// Independent CPU reference over the same canonical edges.
fn reference(edges: &[(i64, i64)]) -> Vec<(i64, i64, i64)> {
    let range = |key: i64| {
        edges.partition_point(|&(s, _)| s < key)..edges.partition_point(|&(s, _)| s <= key)
    };
    let mut out = Vec::new();
    for &(a, b) in edges {
        let nb = &edges[range(b)];
        let na = &edges[range(a)];
        for &(_, c) in nb {
            if na.binary_search_by(|&(_, x)| x.cmp(&c)).is_ok() {
                out.push((a, b, c));
            }
        }
    }
    out
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().ok_or("triangle-compiled EDGES [--check]")?;
    let check = args.iter().any(|a| a == "--check");
    let rows = load(path)?;
    println!("distinct_edges={}", rows.len());
    let runtime = Runtime::new()?;
    let mut dictionary = Dictionary::default();
    let edges = runtime.upload_rows(&rows, &mut dictionary, &[Codec::Plain; 2])?;
    runtime.pool().stream().synchronize()?;
    let mut times = Vec::new();
    let mut count = 0;
    for iteration in 0..6 {
        let attempts_before = cq_gpu_runtime::retry_readback_count();
        let started = Instant::now();
        let answer = triangles(&edges);
        runtime.pool().stream().synchronize()?;
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        let attempts = cq_gpu_runtime::retry_readback_count() - attempts_before;
        count = answer.checked_len()?;
        if check && iteration == 0 {
            let mut actual = answer.to_host()?;
            actual.sort_unstable();
            let expected = reference(&rows);
            if actual != expected {
                return Err(
                    format!("mismatch: {} vs {} expected", actual.len(), expected.len()).into(),
                );
            }
            println!("exact=true");
        }
        println!(
            "iteration={iteration} rows={count} query_ms={ms:.3} retry_attempts={attempts} output_capacity={} output_bytes={}",
            answer.capacity(),
            answer.encoded_bytes()
        );
        drop(answer);
        if iteration > 0 {
            times.push(ms);
        }
    }
    times.sort_by(f64::total_cmp);
    println!(
        "answer_rows={count} median_query_ms={:.3}",
        times[times.len() / 2]
    );
    let mut certificate_times = Vec::new();
    for sample in 0..6 {
        let mut execution = runtime.execution();
        let input = execution.bind_host(&edges, "E(src:i64,dst:i64)", 2)?;
        let started = Instant::now();
        execution.certify_host_set(input, "E")?;
        runtime.pool().stream().synchronize()?;
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        println!("certificate_sample={sample} certificate_ms={ms:.3}");
        if sample > 0 {
            certificate_times.push(ms);
        }
    }
    certificate_times.sort_by(f64::total_cmp);
    println!(
        "median_certificate_ms={:.3}",
        certificate_times[certificate_times.len() / 2]
    );
    Ok(())
}
