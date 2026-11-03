#![recursion_limit = "1024"]
//! Directed i64 triangles: `probe smallest` over authored radix layouts. An
//! authored configuration, not the default: on DBLP it is 1.54× slower than
//! `triangle-compiled`'s `probe first` Range plan (README, paired timing).
//!
//! IDs remain native i64 values through upload, radix key packing, item lanes
//! and output decoding; full-domain signed IDs need no dense directory. The
//! two candidate sides `r [b][c]` and `s [a][c]` are `RadixLayout`s with
//! authored advice (u128 words for two i64 key fields, u32 offsets, ratio 4,
//! 16 directory bits). Per edge, GPU-Smallest counts both sides' distinct `c`
//! exactly, enumerates the first smaller one and keeps candidates the other
//! side contains. The full-domain `(a,b,c)` heading (192 bits) retains all
//! three lanes in the ordinary full-row set output.
//! `--check` compares the compiled GPU tuples with the independent CPU reference.
//!
//! Supported scheduling advice is recorded below: 256 lanes × 4 items becomes
//! `tile 1024 items 4`, and range enumeration is `flatten lookup`. Canonical
//! publication flattens the remaining tile partition before building its full-row set.
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

// The authored Smallest triangle with signed vertex types.
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
            checked = let canon probe first [a,c] candidate,s;
            return checked as answer(a,b,c);
        }
        structures { providers { device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns }; range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range }; radix_layout = ::gpu_datalog_runtime::RadixLayout<_, _, _, _, _> { reads [exact, range, membership, candidates], build typed, abi radix }; radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident, comparator_rows], build inferred }; }
            l host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            r host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            s host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
            r = @radix_layout radix { word u128; offset u32; ratio 4; radix_bits 16; merge_tile 1024 } as ::gpu_datalog_runtime::RadixLayout<(i64,i64),(i64,i64),u128,u32,(::gpu_datalog_runtime::Buffer<i64>,::gpu_datalog_runtime::Buffer<i64>)> [b][c];
            s = @radix_layout radix { word u128; offset u32; ratio 4; radix_bits 16; merge_tile 1024 } as ::gpu_datalog_runtime::RadixLayout<(i64,i64),(i64,i64),u128,u32,(::gpu_datalog_runtime::Buffer<i64>,::gpu_datalog_runtime::Buffer<i64>)> [a][c];
            r rows = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider;
            s rows = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider;
            checked rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
            checked canon = rows;
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
    let path = args.first().ok_or("triangle-smallest EDGES [--check]")?;
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
            // DECODE P: diagnostic triangle answer publication exposes logical tuples for CPU comparison.
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
            // Full-domain signed IDs: extremes, both signs, candidates shared
            // under several prefixes and both sides chosen as smallest.
            vec![
                (i64::MIN, -1),
                (-1, i64::MAX),
                (i64::MIN, i64::MAX),
                (i64::MIN, 0),
                (0, i64::MAX),
                (-1, 0),
                (i64::MAX, i64::MIN),
                (-7, -1),
                (-7, i64::MAX),
                (-7, 0),
                (-7, i64::MIN),
                (i64::MIN, -7),
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
            // DECODE P: diagnostic signed-boundary triangle publication exposes reconstructed result tuples.
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
