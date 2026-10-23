#![recursion_limit = "1024"]
//! Directed i64 triangles over the authored Sequence range providers.
//!
//! IDs remain native i64 values through upload, range payload packing and
//! output decoding. The recorded dense directory still requires nonnegative
//! u32-range keys, and canonical output fields must fit one packed u64.
//! `--check` compares the compiled GPU tuples with the independent CPU reference.
//!
//! Supported scheduling advice is recorded below: 256 lanes × 4 items becomes
//! `tile 1024 items 4`, and range enumeration is `flatten lookup`. Canonical
//! publication flattens the remaining tile partition before using PackedCanon.
//! The old frontier(8), directory(rank), and output(retry) choices have no corresponding
//! Sequence advice; they are not silently reinterpreted as another policy.
use cq_gpu_runtime::DeviceColumns;
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

// Preserve the written Smallest triangle and its signed vertex types.
fn triangles<'a>(edges: &'a DeviceColumns<'_, (i64, i64)>) -> DeviceColumns<'a, (i64, i64, i64)> {
    cq_sequence_rewrites::grade! {
        arguments { gpu, Edges at host, ByB at host, ByA at host, result at device };
        pipeline { ::cq_gpu_sequence::machine_grade, ::cq_gpu_sequence::translate };
        physical {
            relation E(src:i64,dst:i64) = (edges);
            l = pipe rename E@0 {src -> a,dst -> b};
            r = let rename E@1 {src -> b,dst -> c};
            s = let rename E@2 {src -> a,dst -> c};
            candidate = pipe probe smallest [b] l,r;
            checked = pipe canon probe first [a,c] candidate,s;
            return checked as answer(a,b,c);
        }
        structures {
            l host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            r host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            s host scan = ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            r = ::cq_gpu_sequence_runtime::RangeProvider [b][c];
            s = ::cq_gpu_sequence_runtime::RangeProvider [a][c];
            r rows = ::cq_gpu_sequence_runtime::RangeProvider;
            s rows = ::cq_gpu_sequence_runtime::RangeProvider;
            checked canon = ::cq_gpu_sequence_runtime::canon::PackedCanon;
        }
        sequence {
            Edges = store rows l;
            ByB = store rows r;
            ByA = store rows s;
            result = store canon flatten map checked flatten lookup s [a,c]
                flatten lookup r [b] tile 1024 items 4 Edges;
            output count;
        }
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
    let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
    let resident = DeviceColumns::from_rows(
        pool,
        &rows,
        &mut cq_gpu_runtime::Dictionary::default(),
        &[cq_gpu_runtime::Codec::For; 2],
    )?;
    let expected = if check {
        let started = Instant::now();
        let mut expected = reference(&rows);
        let cpu_reference_ms = started.elapsed().as_secs_f64() * 1000.0;
        expected.sort_unstable();
        println!(
            "cpu_reference_rows={} cpu_reference_ms={cpu_reference_ms:.6}",
            expected.len()
        );
        Some(expected)
    } else {
        None
    };
    // Match the H05 timing protocol: one discarded warmup, five measured
    // resident calls. Upload, answer download, and CPU checking stay outside.
    let mut samples = [[0.0f64; 5]; 5];
    for iteration in 0..6 {
        let started = Instant::now();
        let answer = triangles(&resident);
        let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
        let timing = cq_gpu_runtime::take_last_timings()
            .ok_or("triangle translator supplied no execution timings")?;
        let count = answer.checked_len()?;
        if let Some(expected) = &expected {
            let mut actual = answer.to_host()?;
            actual.sort_unstable();
            assert_eq!(
                &actual, expected,
                "compiled i64 triangle differs from CPU reference"
            );
        }
        println!(
            "iteration={iteration} warmup={} checked={check} gpu_answer_rows={count} build_ms={:.6} kernel_ms={:.6} dedup_ms={:.6} total_ms={:.6} wall_ms={wall_ms:.6}",
            iteration == 0, timing.build_ms, timing.kernel_ms, timing.dedup_ms, timing.total_ms,
        );
        if iteration > 0 {
            samples[iteration - 1] = [
                timing.build_ms as f64,
                timing.kernel_ms as f64,
                timing.dedup_ms as f64,
                timing.total_ms as f64,
                wall_ms,
            ];
        }
    }
    let medians: [f64; 5] = std::array::from_fn(|field| {
        let mut values = samples.map(|sample| sample[field]);
        values.sort_unstable_by(f64::total_cmp);
        values[2]
    });
    println!(
        "warmups=1 runs=5 checked={check} median_build_ms={:.6} median_kernel_ms={:.6} median_dedup_ms={:.6} median_total_ms={:.6} median_wall_ms={:.6}",
        medians[0], medians[1], medians[2], medians[3], medians[4],
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_i64_triangle_preserves_vertex_ids() -> Result<(), Box<dyn Error>> {
        let pool = cq_gpu_runtime::Pool::new(0, 0, 0)?;
        for mut rows in [
            vec![],
            vec![
                (0i64, 0),
                (1, 2),
                (2, 3),
                (1, 3),
                (1, 3),
                (1_000_000, 1_000_000),
            ],
        ] {
            rows.sort_unstable();
            rows.dedup();
            let resident = DeviceColumns::from_rows(
                pool.clone(),
                &rows,
                &mut cq_gpu_runtime::Dictionary::default(),
                &[cq_gpu_runtime::Codec::For; 2],
            )?;
            let answer = triangles(&resident);
            let mut actual = answer.to_host()?;
            actual.sort_unstable();
            let mut expected = reference(&rows);
            expected.sort_unstable();
            assert_eq!(answer.checked_len()?, expected.len());
            assert_eq!(actual, expected);
        }
        Ok(())
    }
}
