//! Directed i64 triangles: an explicit unsupported Sequence translation fixture.
//!
//! The resident range provider requires u32/i32 payload codes. Do not silently
//! narrow arbitrary signed vertex IDs or substitute the handwritten GPU oracle.
//! `--check` runs the independent CPU reference, not a compiled GPU query.
//! Enable `triangle-sequence-rejection` to exercise the authored translation.
//!
//! Supported scheduling advice is recorded below: 256 lanes × 4 items becomes
//! `tile 1024 items 4`, and range enumeration is `flatten lookup`. The old
//! frontier(8), directory(rank), and output(retry) choices have no corresponding
//! Sequence advice; they are not silently reinterpreted as another policy.
#[cfg(feature = "triangle-sequence-rejection")]
use cq_gpu_runtime::DeviceColumns;
use std::{
    error::Error,
    io::{BufRead, BufReader},
};

const REJECTION: &str = "T-Gpu: range payload needs a recorded u32/i32 dense code";

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

// TODO: the existing Sequence range provider cannot retain i64 adjacency payloads.
// This fixture preserves the Smallest triangle and its signed vertex types.
// The runtime rejection below is sourced from sequence/core/src/rows.rs, not
// from adapter argument validation. The feature intentionally fails translation.
#[cfg(feature = "triangle-sequence-rejection")]
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
            result = store canon map checked flatten lookup s [a,c]
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
    if check {
        let expected = reference(&rows);
        println!("cpu_reference_rows={}", expected.len());
    }
    Err(REJECTION.into())
}
