//! End-to-end GPU candidates, compared as bags rather than deduplicated sets.
//! Kernel modules use the same CUDA macro / merged-PTX build as other Sequence tests.
use gpu_datalog_runtime::{Buffer, Column, DeviceLayout, Packing, Pool, Run};
use std::sync::Arc;
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn runs(pool: &Arc<Pool>, packing: &Packing, rows: &[Vec<Vec<i128>>]) -> Result<Vec<Run<u64>>> {
    rows.iter().map(|rows| {
        let mut words = rows.iter().map(|row| packing.pack::<u64>(row)).collect::<Result<Vec<_>>>()?;
        words.sort_unstable();
        // Keep duplicates between runs, but each immutable run is sorted/unique.
        words.dedup();
        let len = words.len();
        let buffer = Buffer::from_host(pool.clone(), &words)?;
        // SAFETY: this upload is initialized, sorted and unique in the packing's order.
        unsafe { Run::from_sorted(buffer, len) }
    }).collect()
}

fn bag(buffer: &Buffer<u64>, len: usize, packing: &Packing) -> Result<Vec<Vec<i128>>> {
    let mut rows = buffer.to_host_prefix(len)?.into_iter()
        .map(|word| packing.unpack(word)).collect::<Result<Vec<_>>>()?;
    rows.sort();
    Ok(rows)
}

fn join(left: &[Vec<Vec<i128>>], right: &[Vec<Vec<i128>>]) -> Vec<Vec<i128>> {
    let mut expected = Vec::new();
    for l in left.iter().flatten() {
        for r in right.iter().flatten() {
            if l[1] == r[0] { expected.push(vec![l[0], r[1]]); }
        }
    }
    expected.sort();
    expected
}

#[test]
fn delta_edge_runs_empty_delta_and_retry() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let dp = Packing::with_columns(&[Column::I32, Column::I32])?;
    let ep = Packing::with_columns(&[Column::I32, Column::U32])?;
    let edge_rows = vec![vec![], (0..4097).map(|z| vec![-7, z]).collect(),
        vec![vec![-7, 0], vec![9, u32::MAX as i128]], vec![]];
    let edge = DeviceLayout::<(i32,), (u32,)>::from_runs(pool.clone(), ep.clone(), runs(&pool, &ep, &edge_rows)?)?;
    let mut arena = gpu_datalog_runtime::CandidateArena::new(pool.clone());
    for delta_rows in [vec![vec![vec![-13, -7], vec![i32::MIN as i128, 9]]], vec![vec![]]] {
        let delta = DeviceLayout::<(i32,), (i32,)>::from_runs(pool.clone(), dp.clone(), runs(&pool, &dp, &delta_rows)?)?;
        let result = cq_gpu_sequence_test_macros::planned_arena_hole!(linear);
        assert_eq!(bag(result.buffer(), result.len(), result.packing())?, join(&delta_rows, &edge_rows));
    }
    // Opt-in artifact location avoids writing into the source tree during tests.
    if let Some(path) = std::env::var_os("CQ_RUN_HOLE_SOURCE") {
        std::fs::write(path, cq_gpu_sequence_test_macros::arena_hole_source!(linear))?;
    }
    Ok(())
}

#[test]
fn nonlinear_delta_full_visits_every_full_run() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::with_columns(&[Column::I32, Column::I32])?;
    let rows = vec![vec![vec![-3, -2], vec![-2, 5]], vec![],
        vec![vec![5, -3], vec![-3, -2]], vec![vec![-2, 8], vec![8, -3]]];
    let delta_rows = vec![vec![vec![-11, -3], vec![-12, -2], vec![17, 5], vec![19, 8]]];
    let delta = DeviceLayout::<(i32,), (i32,)>::from_runs(pool.clone(), packing.clone(), runs(&pool, &packing, &delta_rows)?)?;
    let full = DeviceLayout::<(i32,), (i32,)>::from_runs(pool.clone(), packing.clone(), runs(&pool, &packing, &rows)?)?;
    let mut arena = gpu_datalog_runtime::CandidateArena::new(pool);
    let result = cq_gpu_sequence_test_macros::planned_arena_hole!(nonlinear);
    assert_eq!(bag(result.buffer(), result.len(), result.packing())?, join(&delta_rows, &rows));
    Ok(())
}

#[test]
fn two_edge_occurrences_keep_independent_lookup_positions() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let dp = Packing::with_columns(&[Column::I32, Column::I32])?;
    let ep = Packing::with_columns(&[Column::I32, Column::U32])?;
    let delta_rows = vec![vec![vec![-2, 7], vec![7, -2], vec![-2, -2]]];
    let edge_rows = vec![vec![vec![-2, 0], vec![7, u32::MAX as i128]], vec![],
        vec![vec![-2, 11], vec![7, 12]], vec![vec![-2, 0]]];
    let delta = DeviceLayout::<(i32,), (i32,)>::from_runs(pool.clone(), dp.clone(), runs(&pool, &dp, &delta_rows)?)?;
    let edge = DeviceLayout::<(i32,), (u32,)>::from_runs(pool.clone(), ep.clone(), runs(&pool, &ep, &edge_rows)?)?;
    let mut expected = Vec::new();
    for d in delta_rows.iter().flatten() {
        for l in edge_rows.iter().flatten().filter(|e| e[0] == d[0]) {
            for r in edge_rows.iter().flatten().filter(|e| e[0] == d[1]) {
                expected.push(vec![l[1], r[1]]);
            }
        }
    }
    expected.sort();
    let mut arena = gpu_datalog_runtime::CandidateArena::new(pool);
    let result = cq_gpu_sequence_test_macros::planned_arena_hole!(double);
    assert_eq!(bag(result.buffer(), result.len(), result.packing())?, expected);
    Ok(())
}

#[test]
fn anti_membership_checks_all_blocked_runs_without_multiplying_rows() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let dp = Packing::with_columns(&[Column::I32, Column::I32])?;
    let bp = Packing::with_columns(&[Column::I32])?;
    let delta_rows = vec![vec![vec![-5, 1], vec![-2, 2], vec![8, -9]], vec![vec![-5, 1]]];
    let blocked_rows = vec![vec![], vec![vec![-2]], vec![vec![8]], vec![vec![-2]]];
    let delta = DeviceLayout::<(i32,), (i32,)>::from_runs(pool.clone(), dp.clone(), runs(&pool, &dp, &delta_rows)?)?;
    let blocked = DeviceLayout::<(i32,), ()>::from_runs(pool.clone(), bp.clone(), runs(&pool, &bp, &blocked_rows)?)?;
    let mut expected: Vec<_> = delta_rows.iter().flatten()
        .filter(|d| !blocked_rows.iter().flatten().any(|b| b[0] == d[0])).cloned().collect();
    expected.sort();
    let mut arena = gpu_datalog_runtime::CandidateArena::new(pool);
    let result = cq_gpu_sequence_test_macros::planned_arena_hole!(anti);
    assert_eq!(bag(result.buffer(), result.len(), result.packing())?, expected);
    Ok(())
}

#[test]
fn multi_run_scan_keeps_run_boundary_duplicates_and_unsigned_symbols() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::with_columns(&[Column::I32, Column::U32])?;
    let rows = vec![vec![], vec![vec![i32::MIN as i128, u32::MAX as i128]],
        vec![vec![-1, 0], vec![4, 19]], vec![], vec![vec![-1, 0]], vec![]];
    let edge = DeviceLayout::<(i32,), (u32,)>::from_runs(pool.clone(), packing.clone(), runs(&pool, &packing, &rows)?)?;
    let mut expected: Vec<_> = rows.iter().flatten().cloned().collect();
    expected.sort();
    let mut arena = gpu_datalog_runtime::CandidateArena::new(pool);
    let result = cq_gpu_sequence_test_macros::planned_arena_hole!(scan);
    assert_eq!(bag(result.buffer(), result.len(), result.packing())?, expected);
    Ok(())
}

#[test]
fn wide_multi_run_scan_preserves_both_word_halves() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::with_columns(&[Column::I64, Column::I64])?;
    let rows = vec![
        vec![],
        vec![vec![i64::MIN as i128, i64::MAX as i128], vec![-1, 0]],
        vec![vec![i64::MAX as i128, i64::MIN as i128]],
        vec![],
        vec![vec![-1, 0], vec![1i128 << 40, -(1i128 << 48)]],
    ];
    let mut uploaded = Vec::new();
    for run in &rows {
        let mut words = run.iter().map(|row| packing.pack::<u128>(row))
            .collect::<Result<Vec<_>>>()?;
        words.sort_unstable();
        words.dedup();
        let len = words.len();
        let buffer = Buffer::from_host(pool.clone(), &words)?;
        // SAFETY: every uploaded word is initialized and the run is sorted/unique.
        uploaded.push(unsafe { Run::from_sorted(buffer, len)? });
    }
    let edge = DeviceLayout::<(i64,), (i64,)>::from_runs(pool.clone(), packing, uploaded)?;
    let mut arena = gpu_datalog_runtime::CandidateArena::new(pool);
    let result = cq_gpu_sequence_test_macros::planned_arena_hole!(wide_scan);
    assert_eq!(result.packing().word_bits(), 128);
    let lanes = result.buffer().to_host_prefix(result.len() * 2)?;
    let mut actual = lanes.chunks_exact(2).map(|lanes| {
        let word = lanes[0] as u128 | ((lanes[1] as u128) << 64);
        result.packing().unpack(word)
    }).collect::<Result<Vec<_>>>()?;
    let mut expected: Vec<_> = rows.into_iter().flatten().collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
    Ok(())
}

#[path = "support/run_holes.rs"]
mod fixtures;

#[test]
fn run_binding_rejects_nonleading_lookup_key() {
    let mut source = fixtures::fixture("linear");
    source.structures.entries[2] =
        syn::parse_quote!(e = ::cq_gpu_sequence_runtime::RangeProvider [z][y];);
    assert!(cq_gpu_sequence_core::runs::bind(&source).is_err());
}

#[test]
fn run_binding_rejects_lookup_key_outside_recorded_layout() {
    let mut source = fixtures::fixture("linear");
    source.structures.entries[2] =
        syn::parse_quote!(e = ::cq_gpu_sequence_runtime::RangeProvider [missing][z];);
    assert!(cq_gpu_sequence_core::runs::bind(&source).is_err());
}
