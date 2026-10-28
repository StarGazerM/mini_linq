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
    cq_gpu_runtime::enable_arena_timing(false);
    for delta_rows in [vec![vec![vec![-13, -7], vec![i32::MIN as i128, 9]]], vec![vec![]]] {
        let delta = DeviceLayout::<(i32,), (i32,)>::from_runs(pool.clone(), dp.clone(), runs(&pool, &dp, &delta_rows)?)?;
        let result = cq_gpu_sequence_test_macros::planned_arena_hole!(linear);
        assert_eq!(bag(result.buffer(), result.len(), result.packing())?, join(&delta_rows, &edge_rows));
        assert!(cq_gpu_runtime::take_last_timings().is_none());
    }
    cq_gpu_runtime::enable_arena_timing(true);
    let delta_rows = vec![vec![vec![-13, -7]]];
    let delta = DeviceLayout::<(i32,), (i32,)>::from_runs(
        pool.clone(), dp.clone(), runs(&pool, &dp, &delta_rows)?)?;
    let result = cq_gpu_sequence_test_macros::planned_arena_hole!(linear);
    let timing = cq_gpu_runtime::take_last_timings();
    cq_gpu_runtime::enable_arena_timing(false);
    assert!(timing.is_some());
    assert_eq!(bag(result.buffer(), result.len(), result.packing())?, join(&delta_rows, &edge_rows));
    // Opt-in artifact location avoids writing into the source tree during tests.
    if let Some(path) = std::env::var_os("CQ_RUN_HOLE_SOURCE") {
        std::fs::write(path, cq_gpu_sequence_test_macros::arena_hole_source!(linear))?;
    }
    Ok(())
}

#[test]
fn previous_count_is_only_a_hint_and_retained_batches_stay_immutable() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let dp = Packing::with_columns(&[Column::I32, Column::I32])?;
    let ep = Packing::with_columns(&[Column::I32, Column::U32])?;
    let edge_rows = vec![vec![vec![7, 11], vec![7, 12]]];
    let edge = DeviceLayout::<(i32,), (u32,)>::from_runs(
        pool.clone(), ep.clone(), runs(&pool, &ep, &edge_rows)?)?;
    let mut arena = gpu_datalog_runtime::CandidateArena::new(pool.clone());
    let mut retained = Vec::new();
    for (rows, attempts) in [(1, 2), (2, 2), (1, 1), (0, 1), (1, 2)] {
        let delta_rows = vec![(0..rows).map(|x| vec![x, 7]).collect::<Vec<_>>()];
        let delta = DeviceLayout::<(i32,), (i32,)>::from_runs(
            pool.clone(), dp.clone(), runs(&pool, &dp, &delta_rows)?)?;
        let before = gpu_datalog_runtime::readback_count();
        let result = cq_gpu_sequence_test_macros::planned_arena_hole_previous!(linear);
        assert_eq!(gpu_datalog_runtime::readback_count() - before, attempts);
        assert_eq!(arena.previous_count(), result.len());
        let expected = join(&delta_rows, &edge_rows);
        assert_eq!(bag(result.buffer(), result.len(), result.packing())?, expected);
        retained.push((result, expected));
    }
    for (result, expected) in retained {
        assert_eq!(bag(result.buffer(), result.len(), result.packing())?, expected);
    }
    arena.clear();
    assert_eq!(arena.previous_count(), 0);
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
    let native = fixtures::fixture("native_anti");
    assert!(cq_lang_structures::contract::check(&native)?.built.is_empty());
    let result = cq_gpu_sequence_test_macros::planned_arena_hole!(native_anti);
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

#[test]
fn native_host_lookup_uses_supplied_runs_without_building() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let dp = Packing::with_columns(&[Column::I32, Column::I32])?;
    let ep = Packing::with_columns(&[Column::I32, Column::U32])?;
    let left = vec![vec![vec![-13, -7], vec![2, 9]]];
    let right = vec![vec![vec![-7, 0], vec![9, 4]], vec![vec![-7, 8]], vec![]];
    let delta = DeviceLayout::<(i32,), (i32,)>::from_runs(
        pool.clone(), dp.clone(), runs(&pool, &dp, &left)?)?;
    let edge = DeviceLayout::<(i32,), (u32,)>::from_runs(
        pool.clone(), ep.clone(), runs(&pool, &ep, &right)?)?;
    let mut arena = gpu_datalog_runtime::CandidateArena::new(pool);
    let result = cq_gpu_sequence_test_macros::planned_arena_hole!(native_linear);
    assert_eq!(bag(result.buffer(), result.len(), result.packing())?, join(&left, &right));
    Ok(())
}

#[test]
fn native_host_lookup_rejects_keys_outside_declared_prefix() {
    let mut source = fixtures::fixture("native_linear");
    source.structures.entries[1] =
        syn::parse_quote!(e host scan = ::gpu_datalog_runtime::DeviceLayout<(),(i32,u32)> [];);
    source.structures.entries[2] =
        syn::parse_quote!(e host result(joined) = ::gpu_datalog_runtime::DeviceLayout<(),(i32,u32)> [y];);
    let bound = cq_gpu_sequence_core::runs::bind(&source).unwrap();
    let sequence = cq_sequence_lowering::compile(&bound).unwrap();
    let machine = cq_gpu_sequence::machine();
    let planned = cq_sequence_plan::plan(&sequence, &machine, &cq_gpu_sequence::catalog()).unwrap();
    let analysis = cq_lang_sequence::check_grading(&planned.program, &machine).unwrap();
    let error = cq_gpu_sequence_core::translate_arena(
        &planned.program, &analysis, &syn::parse_quote!(&mut arena)).unwrap_err();
    assert!(error.to_string().contains("native lookup key is not a packed leading prefix"));
}

#[test]
fn host_captures_are_lazy_once_per_invocation_and_survive_retry() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::with_columns(&[Column::I32, Column::U32])?;
    let mut arena = gpu_datalog_runtime::CandidateArena::new(pool.clone());
    for length in [0usize, 4097, 0, 4097] {
        let calls = std::cell::Cell::new([0usize; 3]);
        let host_key = || { let mut counts = calls.get(); counts[0] += 1; calls.set(counts); -7i32 };
        let allow = || { let mut counts = calls.get(); counts[1] += 1; calls.set(counts); true };
        let host_value = || { let mut counts = calls.get(); counts[2] += 1; calls.set(counts); 23u32 };
        let rows = vec![(0..length).map(|value| vec![-7, value as i128]).collect()];
        let edge = DeviceLayout::<(i32,), (u32,)>::from_runs(
            pool.clone(), packing.clone(), runs(&pool, &packing, &rows)?)?;
        let result = cq_gpu_sequence_test_macros::planned_arena_hole!(capture);
        assert_eq!(bag(result.buffer(), result.len(), result.packing())?, vec![vec![-7, 23]; length]);
        assert_eq!(calls.get(), [usize::from(length != 0); 3]);
    }
    Ok(())
}

#[test]
fn constant_atom_filters_before_projection_exactly_like_cpu() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::with_columns(&[Column::I32, Column::U32])?;
    let rows = vec![vec![], vec![vec![-7, 0], vec![4, 19]], vec![vec![-7, u32::MAX as i128], vec![-7, 0]]];
    let edge = DeviceLayout::<(i32,), (u32,)>::from_runs(
        pool.clone(), packing.clone(), runs(&pool, &packing, &rows)?)?;
    let mut arena = gpu_datalog_runtime::CandidateArena::new(pool);
    let result = cq_gpu_sequence_test_macros::planned_arena_hole!(constant_atom);
    let mut expected = rows.iter().flatten().filter(|row| row[0] == -7).map(|row| vec![row[1]]).collect::<Vec<_>>();
    expected.sort();
    assert_eq!(bag(result.buffer(), result.len(), result.packing())?, expected);
    Ok(())
}

fn wide_runs<W: gpu_datalog_runtime::Word>(pool: &Arc<Pool>, packing: &Packing, rows: &[Vec<Vec<i128>>]) -> Result<Vec<Run<W>>> {
    rows.iter().map(|rows| {
        let mut words = rows.iter().map(|row| packing.pack::<W>(row)).collect::<Result<Vec<_>>>()?;
        words.sort_unstable();
        words.dedup();
        let length = words.len();
        let buffer = Buffer::from_host(pool.clone(), &words)?;
        // SAFETY: every word was initialized above, then sorted and deduplicated.
        unsafe { Run::from_sorted(buffer, length) }
    }).collect()
}

#[test]
fn three_lane_scan_decodes_cross_lane_fields_and_publishes_exact_bag() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::with_columns(&[Column::I64, Column::I32, Column::I64])?;
    let rows = vec![vec![], vec![vec![i64::MIN as i128, -7, i64::MAX as i128], vec![-1, 3, 0]],
        vec![vec![-1, 3, 0], vec![i64::MAX as i128, i32::MIN as i128, i64::MIN as i128]]];
    let edge = DeviceLayout::<(i64,), (i32,i64)>::from_runs(
        pool.clone(), packing.clone(), wide_runs::<[u64; 3]>(&pool, &packing, &rows)?)?;
    let mut arena = gpu_datalog_runtime::CandidateArena::new(pool);
    let result = cq_gpu_sequence_test_macros::planned_arena_hole!(wide3_scan);
    let words = result.buffer().to_host_prefix(result.len() * 3)?;
    let mut actual = words.chunks_exact(3).map(|word| result.packing().unpack([word[0], word[1], word[2]])).collect::<Result<Vec<_>>>()?;
    let mut expected: Vec<_> = rows.into_iter().flatten().collect();
    actual.sort(); expected.sort();
    assert_eq!(actual, expected);
    Ok(())
}

#[test]
fn four_lane_scan_decodes_cross_lane_fields_and_publishes_exact_bag() -> Result {
    let pool = Pool::new(0, 0, 0)?;
    let packing = Packing::with_columns(&[Column::I64, Column::I32, Column::I64, Column::I32, Column::I32])?;
    let rows = vec![vec![], vec![vec![i64::MIN as i128, -7, i64::MAX as i128, 3, -9]],
        vec![vec![i64::MIN as i128, -7, i64::MAX as i128, 3, -9], vec![-1, i32::MAX as i128, 0, i32::MIN as i128, 4]]];
    let edge = DeviceLayout::<(i64,), (i32,i64,i32,i32)>::from_runs(
        pool.clone(), packing.clone(), wide_runs::<[u64; 4]>(&pool, &packing, &rows)?)?;
    let mut arena = gpu_datalog_runtime::CandidateArena::new(pool);
    let result = cq_gpu_sequence_test_macros::planned_arena_hole!(wide4_scan);
    let words = result.buffer().to_host_prefix(result.len() * 4)?;
    let mut actual = words.chunks_exact(4).map(|word| result.packing().unpack([word[0], word[1], word[2], word[3]])).collect::<Result<Vec<_>>>()?;
    let mut expected: Vec<_> = rows.into_iter().flatten().collect();
    actual.sort(); expected.sort();
    assert_eq!(actual, expected);
    Ok(())
}
