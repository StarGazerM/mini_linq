use gpu_device_columns::{
    unique::unique_rows, Codec, ColumnView, EncodedColumn, Pool,
};
use std::{collections::HashSet, sync::Arc};

fn view(column: &EncodedColumn) -> ColumnView {
    unsafe { column.view() }
}

fn oracle<const N: usize>(rows: &[[i64; N]]) -> Option<Vec<u32>> {
    let mut seen = HashSet::new();
    let positions: Vec<_> = rows.iter().enumerate()
        .filter_map(|(row, tuple)| seen.insert(*tuple).then_some(row as u32))
        .collect();
    (positions.len() != rows.len()).then_some(positions)
}

fn check(pool: &Arc<Pool>, columns: &[ColumnView], rows: usize, expected: Option<Vec<u32>>) {
    // Callers keep all input buffers alive through this call and readback.
    let selected = unsafe { unique_rows(pool.clone(), columns, rows) }.unwrap();
    assert_eq!(selected.as_ref().map(|buffer| buffer.to_host().unwrap()), expected);
    if let Some(selected) = selected {
        assert!(Arc::ptr_eq(selected.pool(), pool));
    }
}

#[test]
fn projected_duplicates_keep_first_positions_in_input_order() {
    let pool = Pool::new(0, 1 << 20, 128 << 20).unwrap();
    let a = [7_i32, -2, 7, 7, -2, 8, 7, 8, -2];
    let b = [i64::MIN, i64::MAX, i64::MIN, 4, i64::MAX, 4, 4, 4, 0];
    let da = EncodedColumn::from_host(pool.clone(), &a, Codec::For).unwrap();
    let db = EncodedColumn::from_host(pool.clone(), &b, Codec::Plain).unwrap();
    check(&pool, &[view(&da), view(&db)], a.len(), Some(vec![0, 1, 3, 5, 8]));
    // Projecting away the second column changes which occurrences survive.
    check(&pool, &[view(&da)], a.len(), Some(vec![0, 1, 5]));
    gpu_device_columns::unique::release_cached_kernels();
}

#[test]
fn mixed_original_widths_and_codecs_match_exact_host_oracle() {
    let pool = Pool::new(0, 1 << 20, 128 << 20).unwrap();
    for rows in [2, 255, 256, 257, 1023, 1024, 1025, 4099] {
        let tuples: Vec<_> = (0..rows).map(|i| {
            let key = (i * 97 + i / 7) % 211;
            [
                match key % 4 { 0 => i32::MIN as i64, 1 => i32::MAX as i64, _ => -(key as i64) },
                match key % 5 { 0 => 0, 1 => u32::MAX as i64, _ => key as i64 },
                match key % 3 { 0 => i64::MIN, 1 => i64::MAX, _ => key as i64 },
            ]
        }).collect();
        let a: Vec<_> = tuples.iter().map(|row| row[0] as i32).collect();
        let b: Vec<_> = tuples.iter().map(|row| row[1] as u32).collect();
        let c: Vec<_> = tuples.iter().map(|row| row[2]).collect();
        let da = EncodedColumn::from_host(pool.clone(), &a, Codec::For).unwrap();
        let db = EncodedColumn::from_host(pool.clone(), &b, Codec::Delta).unwrap();
        let dc = EncodedColumn::from_host(pool.clone(), &c, Codec::Plain).unwrap();
        let native = EncodedColumn::from_host_native(pool.clone(), &c).unwrap();
        check(&pool, &[view(&da), view(&db), view(&dc), view(&native)], rows, oracle(&tuples));
        check(&pool, &[view(&da)], rows, oracle(&tuples.iter().map(|r| [r[0]]).collect::<Vec<_>>()));
    }
    gpu_device_columns::unique::release_cached_kernels();
}

#[test]
fn duplicate_free_and_identical_rows() {
    let pool = Pool::new(0, 1 << 20, 128 << 20).unwrap();
    let a: Vec<u32> = (0..4099).map(|i| u32::MAX - i).collect();
    let da = EncodedColumn::from_host(pool.clone(), &a, Codec::Delta).unwrap();
    check(&pool, &[view(&da)], a.len(), None);
    let repeated = EncodedColumn::from_host(pool.clone(), &vec![i64::MIN; 8193], Codec::For).unwrap();
    check(&pool, &[view(&repeated)], 8193, Some(vec![0]));
    gpu_device_columns::unique::release_cached_kernels();
}

// Test-only algebraic collision construction, not a production hash knob.
// The production recurrence is h_next = avalanche(h_previous XOR value).
// Choosing the last field as avalanche(seed XOR first) XOR target gives the
// exact same full 64-bit hash for every tuple, even though tuples differ.
fn avalanche(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[test]
fn arbitrary_full_hash_collisions_do_not_merge_distinct_tuples() {
    let pool = Pool::new(0, 1 << 20, 128 << 20).unwrap();
    for duplicate in [false, true] {
        let rows = if duplicate { 1025 } else { 257 };
        let a: Vec<i32> = (0..rows).map(|i| {
            let id = if duplicate { (i * 97) % 257 } else { i };
            id as i32 - 128
        }).collect();
        let b: Vec<i64> = a.iter().map(|&first| {
            (avalanche(0x9e37_79b9_7f4a_7c15 ^ first as i64 as u64)
                ^ 0x0123_4567_89ab_cdef) as i64
        }).collect();
        let tuples: Vec<_> = a.iter().zip(&b).map(|(&a, &b)| [a as i64, b]).collect();
        for codec in [Codec::For, Codec::Plain, Codec::Delta] {
            let da = EncodedColumn::from_host(pool.clone(), &a, codec).unwrap();
            let db = EncodedColumn::from_host(pool.clone(), &b, codec).unwrap();
            check(&pool, &[view(&da), view(&db)], rows, oracle(&tuples));
        }
        let da = EncodedColumn::from_host(pool.clone(), &a, Codec::For).unwrap();
        let db = EncodedColumn::from_host_native(pool.clone(), &b).unwrap();
        check(&pool, &[view(&da), view(&db)], rows, oracle(&tuples));
    }
    gpu_device_columns::unique::release_cached_kernels();
}

#[test]
fn empty_and_zero_column_relations_preserve_set_semantics() {
    let pool = Pool::new(0, 1 << 20, 128 << 20).unwrap();
    check(&pool, &[], 0, None);
    check(&pool, &[], 1, None);
    check(&pool, &[], 2, Some(vec![0]));
    check(&pool, &[], 4099, Some(vec![0]));
    let empty = EncodedColumn::from_host(pool.clone(), &[] as &[i32], Codec::For).unwrap();
    check(&pool, &[view(&empty)], 0, None);
    let singleton = EncodedColumn::from_host(pool.clone(), &[i64::MIN], Codec::Plain).unwrap();
    check(&pool, &[view(&singleton)], 1, None);
    assert!(unsafe { unique_rows(pool, &[], u32::MAX as usize) }.is_err());
}

#[test]
fn cached_modules_follow_context_and_nondefault_stream() {
    let first = Pool::new(0, 1 << 20, 128 << 20).unwrap();
    let second = Pool::new(0, 1 << 20, 128 << 20).unwrap();
    let lane = first.lane().unwrap();
    for pool in [&first, &second, &lane, &first] {
        let values = EncodedColumn::from_host(pool.clone(), &[3_i32, 1, 3, 2, 1], Codec::For).unwrap();
        check(pool, &[view(&values)], 5, Some(vec![0, 1, 3]));
    }
    gpu_device_columns::unique::release_cached_kernels();
}
