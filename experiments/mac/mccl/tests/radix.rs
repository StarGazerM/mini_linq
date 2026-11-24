use mccl::{
    Gpu, Result, memory,
    radix::RadixIndex,
    relation::{IndexSpec, Payload, Relation},
};

const PAIRS: IndexSpec = IndexSpec {
    key_column: 0,
    payload: Payload::Column(1),
};
const ROW_IDS: IndexSpec = IndexSpec {
    key_column: 0,
    payload: Payload::RowId,
};

fn ingest(gpu: &Gpu, rows: &[(u32, u32)]) -> Result<Relation> {
    gpu.relation_u32(rows.len(), |i| [rows[i].0, rows[i].1])
}

fn batch(len: usize) -> Vec<(u32, u32)> {
    let mut state = 0x831a53d7u32;
    (0..len)
        .map(|i| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let key = match i % 9 {
                0 => 0,
                1 | 2 => u32::MAX,
                3 => 0x000f_ffff,
                4 => 0x0010_0000,
                _ => state,
            };
            (key, if i % 7 == 0 { 42 } else { i as u32 })
        })
        .collect()
}

// Independent test oracle: exact rational interpolation over a full endpoint
// search, not a second production spline reader. It checks both present ranks
// and absent-key insertion positions against the native search-window contract.
fn check_bound(index: &RadixIndex, key: u32) {
    let keys = index.keys();
    if keys.is_empty() || key <= keys[0] || key >= keys[keys.len() - 1] {
        return;
    }
    let points = index.points();
    let key = u64::from(key);
    let upper = points.partition_point(|point| point[0] < key);
    assert!(upper > 0 && upper < points.len());
    let [left_key, left_rank] = points[upper - 1];
    let [right_key, right_rank] = points[upper];
    let numerator = u128::from(key - left_key) * u128::from(right_rank - left_rank);
    let estimate = left_rank as usize + (numerator / u128::from(right_key - left_key)) as usize;
    let actual = keys.partition_point(|candidate| u64::from(*candidate) < key);
    let begin = estimate.saturating_sub(index.search_error());
    let end = estimate
        .saturating_add(index.search_error() + 2)
        .min(keys.len());
    assert!(
        begin <= actual && actual <= end,
        "key {key}: estimate {estimate}, rank {actual}"
    );
    if keys.get(actual).copied().map(u64::from) == Some(key) {
        assert!(actual < end);
    }
    assert!(end - begin <= 2 * index.search_error() + 2);
}

fn check(index: &RadixIndex, input: &[(u32, u32)]) {
    let mut expected = input.to_vec();
    expected.sort_by_key(|row| row.0);
    let mut keys = Vec::new();
    let mut offsets = Vec::new();
    let mut items = Vec::new();
    for (key, payload) in expected {
        if keys.last() != Some(&key) {
            keys.push(key);
            offsets.push(items.len());
        }
        items.push(payload);
    }
    offsets.push(items.len());
    assert_eq!(index.len(), input.len());
    assert_eq!(index.is_empty(), input.is_empty());
    assert_eq!(index.keys(), keys);
    assert_eq!(index.offsets(), offsets);
    assert_eq!(index.items(), items);
    assert_eq!(index.radix().len(), 4098);
    assert_eq!(index.search_error(), if input.is_empty() { 32 } else { 33 });
    if keys.is_empty() {
        assert!(index.points().is_empty());
        assert!(index.radix().iter().all(|&rank| rank == 0));
        assert_eq!((index.min_key(), index.max_key(), index.shift()), (0, 0, 0));
        return;
    }
    assert_eq!(index.min_key(), u64::from(keys[0]));
    assert_eq!(index.max_key(), u64::from(keys[keys.len() - 1]));
    let shift = (64 - (index.max_key() - index.min_key()).leading_zeros()).saturating_sub(12);
    assert_eq!(index.shift(), shift);
    assert_eq!(index.points().first(), Some(&[index.min_key(), 0]));
    assert_eq!(
        index.points().last(),
        Some(&[index.max_key(), keys.len() as u64 - 1])
    );
    assert!(
        index
            .points()
            .windows(2)
            .all(|p| p[0][0] < p[1][0] && p[0][1] < p[1][1])
    );
    assert!(index.points().len() <= keys.len());
    for &[key, rank] in index.points() {
        assert_eq!(keys.get(rank as usize).copied().map(u64::from), Some(key));
    }
    for prefix in 0..4098 {
        let expected = index.points().partition_point(|point| {
            ((point[0] - index.min_key()) >> index.shift()) < prefix as u64
        });
        assert_eq!(index.radix()[prefix], expected, "prefix {prefix}");
    }
    for (rank, &key) in keys.iter().enumerate() {
        check_bound(index, key);
        check_bound(index, key.saturating_sub(1));
        check_bound(index, key.saturating_add(1));
        if let Some(&next) = keys.get(rank + 1) {
            check_bound(index, key + (next - key) / 2);
        }
    }
    for key in [
        0,
        1,
        255,
        256,
        4095,
        4096,
        0x7fff_ffff,
        u32::MAX - 1,
        u32::MAX,
    ] {
        check_bound(index, key);
    }
}

#[test]
fn gpu_grouping_full_width_and_chunk_boundaries() -> Result<()> {
    let gpu = Gpu::new()?;
    for len in [0, 1, 2, 31, 255, 256, 257, 511, 512, 513, 2049, 65539] {
        let rows = batch(len);
        check(
            &gpu.build_radix_index(&ingest(&gpu, &rows)?, PAIRS, None)?,
            &rows,
        );
    }
    for len in [1, 257, 4097] {
        let rows = (0..len).map(|i| (u32::MAX, i as u32)).collect::<Vec<_>>();
        check(
            &gpu.build_radix_index(&ingest(&gpu, &rows)?, PAIRS, None)?,
            &rows,
        );
    }
    Ok(())
}

#[test]
fn parallel_spline_bounds_for_dense_sparse_and_curved_keys() -> Result<()> {
    let gpu = Gpu::new()?;
    for mode in 0..5 {
        let rows = (0..8193u32)
            .rev()
            .map(|i| {
                let key = match mode {
                    0 => i,
                    1 => u32::MAX - 8192 + i,
                    2 => i * i,
                    3 => (i / 256) * 1_000_000 + (i % 256).pow(2),
                    _ => {
                        if i < 4096 {
                            i * 3
                        } else {
                            3_000_000_000 + (i - 4096) * 127
                        }
                    }
                };
                (key, i)
            })
            .collect::<Vec<_>>();
        check(
            &gpu.build_radix_index(&ingest(&gpu, &rows)?, PAIRS, None)?,
            &rows,
        );
    }
    Ok(())
}

#[test]
fn masks_original_row_ids_and_selected_signed_domain() -> Result<()> {
    let gpu = Gpu::new()?;
    let rows = batch(4099);
    let source = ingest(&gpu, &rows)?;
    for mode in 0..3 {
        let selected = |i: usize| match mode {
            0 => 0,
            1 => u32::MAX,
            _ => (i % 3) as u32,
        };
        let mask = gpu.relation_u32(rows.len(), |i| [selected(i)])?;
        for spec in [PAIRS, ROW_IDS] {
            let expected = rows
                .iter()
                .enumerate()
                .filter(|(i, _)| selected(*i) != 0)
                .map(|(i, &(key, value))| (key, if spec == ROW_IDS { i as u32 } else { value }))
                .collect::<Vec<_>>();
            check(
                &gpu.build_radix_index(&source, spec, Some(&mask))?,
                &expected,
            );
        }
    }
    let signed = [
        [9, 44],
        [-1, 99],
        [u32::MAX as i64, u32::MAX as i64],
        [5, i64::MAX],
    ];
    let relation = gpu.relation_i64(signed.len(), |i| signed[i])?;
    let mask = gpu.relation_u32(signed.len(), |i| [u32::from(i == 0 || i == 2)])?;
    check(
        &gpu.build_radix_index(&relation, PAIRS, Some(&mask))?,
        &[(9, 44), (u32::MAX, u32::MAX)],
    );
    assert!(gpu.build_radix_index(&relation, PAIRS, None).is_err());
    assert!(gpu.build_radix_index(&relation, ROW_IDS, None).is_err());
    let payload_only = gpu.relation_i64(1, |_| [5, i64::MAX])?;
    assert!(gpu.build_radix_index(&payload_only, PAIRS, None).is_err());
    check(
        &gpu.build_radix_index(&payload_only, ROW_IDS, None)?,
        &[(5, 0)],
    );
    // A failed native build must not poison subsequent output ownership.
    check(&gpu.build_radix_index(&source, PAIRS, None)?, &rows);
    Ok(())
}

#[test]
fn stable_grouped_merges_preserve_inputs_and_occurrence_chronology() -> Result<()> {
    let gpu = Gpu::new()?;
    let a = [(u32::MAX, 3), (0, 9), (17, 4), (17, 4), (17, 1)];
    let b = [(17, 0), (u32::MAX, 2), (0, 8), (17, 7)];
    let big_a = batch(2049);
    let big_b = batch(4097);
    for (a, b) in [
        (&a[..], &b[..]),
        (&a[..], &[][..]),
        (&[][..], &b[..]),
        (&[][..], &[][..]),
        (&big_a[..], &big_b[..]),
    ] {
        let older = gpu.build_radix_index(&ingest(&gpu, a)?, PAIRS, None)?;
        let newer = gpu.build_radix_index(&ingest(&gpu, b)?, PAIRS, None)?;
        let ab = a.iter().chain(b).copied().collect::<Vec<_>>();
        let ba = b.iter().chain(a).copied().collect::<Vec<_>>();
        let merged = gpu.merge_radix_indexes(&older, &newer)?;
        check(&merged, &ab);
        check(&gpu.merge_radix_indexes(&newer, &older)?, &ba);
        check(
            &gpu.merge_radix_indexes(&merged, &older)?,
            &ab.iter().chain(a).copied().collect::<Vec<_>>(),
        );
        check(
            &gpu.merge_radix_indexes(&older, &older)?,
            &a.iter().chain(a).copied().collect::<Vec<_>>(),
        );
        check(&older, a);
        check(&newer, b);
    }
    Ok(())
}

#[test]
fn context_and_input_validation() -> Result<()> {
    let gpu = Gpu::new()?;
    let clone = gpu.clone();
    let foreign = Gpu::new()?;
    let source = ingest(&gpu, &[(2, 9), (1, 8)])?;
    let foreign_source = ingest(&foreign, &[(2, 9), (1, 8)])?;
    let index = clone.build_radix_index(&source, PAIRS, None)?;
    let other = foreign.build_radix_index(&foreign_source, PAIRS, None)?;
    assert!(index.belongs_to(&gpu));
    assert!(index.belongs_to(&clone));
    assert!(!index.belongs_to(&foreign));
    assert!(gpu.build_radix_index(&foreign_source, PAIRS, None).is_err());
    assert!(foreign.build_radix_index(&source, PAIRS, None).is_err());
    for (first, second) in [(&index, &other), (&other, &index), (&other, &other)] {
        assert!(gpu.merge_radix_indexes(first, second).is_err());
    }
    assert!(foreign.merge_radix_indexes(&index, &index).is_err());
    let foreign_mask = foreign.relation_u32(2, |_| [1])?;
    let short_mask = gpu.relation_u32(1, |_| [1])?;
    let wide_mask = gpu.relation_u32(2, |_| [1, 1])?;
    let signed_mask = gpu.relation_i64(2, |_| [1])?;
    for mask in [&foreign_mask, &short_mask, &wide_mask, &signed_mask] {
        assert!(gpu.build_radix_index(&source, PAIRS, Some(mask)).is_err());
    }
    for spec in [
        IndexSpec {
            key_column: 2,
            payload: Payload::RowId,
        },
        IndexSpec {
            key_column: 0,
            payload: Payload::Column(2),
        },
    ] {
        assert!(gpu.build_radix_index(&source, spec, None).is_err());
    }
    check(
        &clone.merge_radix_indexes(&index, &index)?,
        &[(2, 9), (1, 8), (2, 9), (1, 8)],
    );
    Ok(())
}

#[test]
fn retained_arrays_survive_sources_contexts_and_cache_churn() -> Result<()> {
    let gpu = Gpu::new()?;
    let rows = batch(4097);
    let source = ingest(&gpu, &rows)?;
    let original = gpu.build_radix_index(&source, PAIRS, None)?;
    let index = original.clone();
    let keys = index.keys();
    let offsets = index.offsets();
    let items = index.items();
    let points = index.points();
    let radix = index.radix();
    assert_eq!(keys.as_ptr(), original.keys().as_ptr());
    assert_eq!(offsets.as_ptr(), original.offsets().as_ptr());
    assert_eq!(items.as_ptr(), original.items().as_ptr());
    assert_eq!(points.as_ptr(), original.points().as_ptr());
    assert_eq!(radix.as_ptr(), original.radix().as_ptr());
    let merged = gpu.merge_radix_indexes(&index, &original)?;
    drop(source);
    drop(original);
    drop(gpu);
    let churn = Gpu::new()?;
    for _ in 0..4 {
        let temporary = churn.build_radix_index(&ingest(&churn, &batch(8193))?, PAIRS, None)?;
        drop(temporary);
        memory::clear_cache()?;
    }
    assert_eq!(keys, index.keys());
    assert_eq!(offsets, index.offsets());
    assert_eq!(items, index.items());
    assert_eq!(points, index.points());
    assert_eq!(radix, index.radix());
    check(&index, &rows);
    drop(index);
    check(
        &merged,
        &rows.iter().chain(&rows).copied().collect::<Vec<_>>(),
    );
    Ok(())
}

// Independent nested-loop oracle over the source rows in stable key order.
fn joined(rows: &[(u32, u32)], probes: &[u32]) -> Vec<[u32; 2]> {
    let mut sorted = rows.to_vec();
    sorted.sort_by_key(|row| row.0);
    probes
        .iter()
        .enumerate()
        .flat_map(|(ordinal, &key)| {
            sorted
                .iter()
                .filter(move |row| row.0 == key)
                .map(move |row| [ordinal as u32, row.1])
        })
        .collect()
}

fn check_join(gpu: &Gpu, rows: &[(u32, u32)], probes: &[u32]) -> Result<()> {
    let index = gpu.build_radix_index(&ingest(gpu, rows)?, PAIRS, None)?;
    let join = gpu.join_radix(&index, &gpu.upload(probes)?)?;
    let expected = joined(rows, probes);
    assert_eq!(join.len(), expected.len());
    assert_eq!(join.pairs(), expected.as_slice());
    Ok(())
}

#[test]
fn gpu_join_matches_nested_loops_with_misses_duplicates_and_extrema() -> Result<()> {
    let gpu = Gpu::new()?;
    let rows = batch(8193);
    let mut probes: Vec<u32> = rows.iter().step_by(3).map(|row| row.0).collect();
    probes.extend([
        0,
        1,
        0x000f_ffff,
        0x0010_0000,
        u32::MAX,
        u32::MAX - 1,
        12345,
    ]);
    probes.extend(probes.clone().iter().rev());
    check_join(&gpu, &rows, &probes)?;
    // Empty run, empty probes, and probes that all miss.
    check_join(&gpu, &[], &[0, u32::MAX])?;
    check_join(&gpu, &rows, &[])?;
    check_join(&gpu, &[(5, 1), (7, 2)], &[4, 6, 8])?;
    Ok(())
}

#[test]
fn gpu_join_spreads_skewed_fanout_across_output_tiles() -> Result<()> {
    let gpu = Gpu::new()?;
    // One hot key spans dozens of 2048-pair tiles; small keys border it so
    // tile owner searches cross probe and tile boundaries in both directions.
    let mut rows: Vec<(u32, u32)> = (0..100_003).map(|i| (42, i)).collect();
    rows.extend((0..5000).map(|i| (i % 50, 1_000_000 + i)));
    let probes = [41, 42, 43, 7, 42, 99, 0, 42];
    check_join(&gpu, &rows, &probes)?;
    // Exactly tile-sized and one-past-tile outputs.
    for len in [2047, 2048, 2049, 4096] {
        let rows: Vec<_> = (0..len).map(|i| (9, i as u32)).collect();
        check_join(&gpu, &rows, &[9])?;
    }
    Ok(())
}

#[test]
fn gpu_join_rejects_other_contexts() -> Result<()> {
    let gpu = Gpu::new()?;
    let other = Gpu::new()?;
    let index = gpu.build_radix_index(&ingest(&gpu, &batch(17))?, PAIRS, None)?;
    assert!(other.join_radix(&index, &other.upload(&[0])?).is_err());
    assert!(gpu.join_radix(&index, &other.upload(&[0])?).is_err());
    Ok(())
}

#[test]
fn gpu_semijoin_filters_join_pairs_and_row_sets_like_host_checks() -> Result<()> {
    use mccl::radix::SemiJoinRows;
    let gpu = Gpu::new()?;
    // Source rows [probe key, check key, item]; signed and wide values miss.
    let rows: Vec<[i64; 3]> = (0..9001i64)
        .map(|i| {
            let check = match i % 11 {
                0 => -1,
                1 => i64::from(u32::MAX) + 1,
                _ => i % 97,
            };
            [i % 13, check, i % 5]
        })
        .collect();
    let source = gpu.relation_i64(rows.len(), |i| rows[i])?;
    let by_probe = gpu.build_radix_index(&source, ROW_IDS, None)?;
    let probes: Vec<u32> = (0..15).collect();
    let join = gpu.join_radix(&by_probe, &gpu.upload(&probes)?)?;
    // Run over check keys 0..60: items {0,2} for even keys and {1} for odd.
    let run_rows: Vec<(u32, u32)> = (0..60u32)
        .flat_map(|k| {
            if k % 2 == 0 {
                vec![(k, 0), (k, 2)]
            } else {
                vec![(k, 1)]
            }
        })
        .collect();
    let run = gpu.build_radix_index(&ingest(&gpu, &run_rows)?, PAIRS, None)?;
    let joined: Vec<u32> = join.pairs().iter().map(|pair| pair[1]).collect();
    let present = |row: &[i64; 3]| (0..60).contains(&row[1]);
    let paired =
        |row: &[i64; 3]| present(row) && run_rows.contains(&(row[1] as u32, row[2] as u32));
    let keys = gpu.semijoin_rows(SemiJoinRows::Join(&join), &source, 1, None, &run)?;
    let expected: Vec<u32> = joined
        .iter()
        .copied()
        .filter(|&r| present(&rows[r as usize]))
        .collect();
    assert_eq!(keys.as_slice()?, expected.as_slice());
    let items = gpu.semijoin_rows(SemiJoinRows::Rows(&keys), &source, 1, Some(2), &run)?;
    let expected: Vec<u32> = expected
        .into_iter()
        .filter(|&r| paired(&rows[r as usize]))
        .collect();
    assert!(!expected.is_empty());
    assert_eq!(items.as_slice()?, expected.as_slice());
    // Empty inputs, an empty run, and invalid row IDs or columns.
    let none = gpu.upload(&[])?;
    assert!(
        gpu.semijoin_rows(SemiJoinRows::Rows(&none), &source, 1, None, &run)?
            .is_empty()
    );
    let empty = gpu.build_radix_index(&ingest(&gpu, &[])?, PAIRS, None)?;
    assert!(
        gpu.semijoin_rows(SemiJoinRows::Rows(&keys), &source, 1, None, &empty)?
            .is_empty()
    );
    let outside = gpu.upload(&[0, rows.len() as u32])?;
    assert!(
        gpu.semijoin_rows(SemiJoinRows::Rows(&outside), &source, 1, None, &run)
            .is_err()
    );
    assert!(
        gpu.semijoin_rows(SemiJoinRows::Rows(&keys), &source, 3, None, &run)
            .is_err()
    );
    Ok(())
}

#[test]
fn gpu_distinct_rows_match_host_sort_dedup() -> Result<()> {
    use mccl::radix::SemiJoinRows;
    use std::collections::BTreeSet;
    let gpu = Gpu::new()?;
    // Low-cardinality columns force many duplicate tuples; extrema exercise
    // every radix byte. Column 3 is wide and must never be projected.
    let rows: Vec<[i64; 4]> = (0..70001i64)
        .map(|i| {
            let a = [0, 1, u32::MAX as i64, 1 << 24][(i % 4) as usize];
            [a, (i * 7919) % 37, (i / 3) % 5, i64::MIN + i]
        })
        .collect();
    let source = gpu.relation_i64(rows.len(), |i| rows[i])?;
    let oracle = |ids: &[u32], columns: &[usize]| -> Vec<i64> {
        let set: BTreeSet<Vec<i64>> = ids
            .iter()
            .map(|&r| columns.iter().map(|&c| rows[r as usize][c]).collect())
            .collect();
        set.into_iter().flatten().collect()
    };
    // Unordered row set with repeated IDs; output order follows `columns`.
    let ids: Vec<u32> = (0..rows.len() as u32).rev().chain(0..5000).collect();
    let set = gpu.upload(&ids)?;
    for columns in [&[0usize, 1, 2][..], &[2, 0], &[1], &[2, 1, 0, 2]] {
        let distinct = gpu.distinct_rows(SemiJoinRows::Rows(&set), &source, columns)?;
        assert_eq!(distinct.width(), columns.len());
        assert_eq!(distinct.values(), oracle(&ids, columns).as_slice(), "{columns:?}");
    }
    // Join pairs contribute their row column.
    let by_b = gpu.build_radix_index(&source, IndexSpec { key_column: 1, payload: Payload::RowId }, None)?;
    let join = gpu.join_radix(&by_b, &gpu.upload(&[3, 36, 99])?)?;
    let joined: Vec<u32> = join.pairs().iter().map(|&[_, row]| row).collect();
    let distinct = gpu.distinct_rows(SemiJoinRows::Join(&join), &source, &[0, 2])?;
    assert_eq!(distinct.values(), oracle(&joined, &[0, 2]).as_slice());
    // Empty input, then errors: wide values, IDs outside, column arity/range.
    let none = gpu.upload(&[])?;
    assert!(gpu.distinct_rows(SemiJoinRows::Rows(&none), &source, &[0])?.is_empty());
    assert!(gpu.distinct_rows(SemiJoinRows::Rows(&set), &source, &[3]).is_err());
    let outside = gpu.upload(&[0, rows.len() as u32])?;
    assert!(gpu.distinct_rows(SemiJoinRows::Rows(&outside), &source, &[0]).is_err());
    assert!(gpu.distinct_rows(SemiJoinRows::Rows(&set), &source, &[]).is_err());
    assert!(gpu.distinct_rows(SemiJoinRows::Rows(&set), &source, &[0; 5]).is_err());
    assert!(gpu.distinct_rows(SemiJoinRows::Rows(&set), &source, &[4]).is_err());
    Ok(())
}

#[test]
fn fused_join_semijoin_distinct_matches_stepwise_calls() -> Result<()> {
    use mccl::radix::{SemiJoinRows, SemiJoinStep};
    let gpu = Gpu::new()?;
    // Source rows [probe key, check key, item, wide]: duplicates and misses.
    let rows: Vec<[i64; 4]> = (0..40009i64)
        .map(|i| [i % 97, (i * 31) % 211, i % 13, i64::MIN + i])
        .collect();
    let source = gpu.relation_i64(rows.len(), |i| rows[i])?;
    let by_probe = gpu.build_radix_index(&source, ROW_IDS, None)?;
    let checks: Vec<(u32, u32)> = (0..211u32).filter(|k| k % 3 != 0).map(|k| (k, k % 13)).collect();
    let check = gpu.build_radix_index(&ingest(&gpu, &checks)?, PAIRS, None)?;
    let odd: Vec<(u32, u32)> = (0..13u32).filter(|k| k % 2 == 1).map(|k| (k, 0)).collect();
    let items = gpu.build_radix_index(&ingest(&gpu, &odd)?, PAIRS, None)?;
    let probes = gpu.upload(&[5, 96, 500, 5, 0, 40])?;
    let steps = [
        SemiJoinStep { index: &check, key_column: 1, item_column: Some(2) },
        SemiJoinStep { index: &items, key_column: 2, item_column: None },
    ];
    // Stepwise reference through the separately tested primitives.
    let join = gpu.join_radix(&by_probe, &probes)?;
    let a = gpu.semijoin_rows(SemiJoinRows::Join(&join), &source, 1, Some(2), &check)?;
    let b = gpu.semijoin_rows(SemiJoinRows::Rows(&a), &source, 2, None, &items)?;
    assert!(!b.is_empty() && b.len() < join.len());
    for columns in [&[0usize, 1, 2][..], &[2, 0]] {
        let expected = gpu.distinct_rows(SemiJoinRows::Rows(&b), &source, columns)?;
        let fused = gpu.join_semijoin_distinct(&by_probe, &probes, &source, &steps, columns)?;
        assert_eq!(fused.values(), expected.values(), "{columns:?}");
    }
    // No steps: distinct over the whole join.
    let expected = gpu.distinct_rows(SemiJoinRows::Join(&join), &source, &[1])?;
    let fused = gpu.join_semijoin_distinct(&by_probe, &probes, &source, &[], &[1])?;
    assert_eq!(fused.values(), expected.values());
    // A step that removes everything, and probes that match nothing.
    let empty = gpu.build_radix_index(&ingest(&gpu, &[])?, PAIRS, None)?;
    let none = [SemiJoinStep { index: &empty, key_column: 1, item_column: None }];
    assert!(gpu.join_semijoin_distinct(&by_probe, &probes, &source, &none, &[0])?.is_empty());
    let misses = gpu.upload(&[1000, 2000])?;
    assert!(gpu.join_semijoin_distinct(&by_probe, &misses, &source, &steps, &[0])?.is_empty());
    // Deferred GPU errors still surface: a wide projected column.
    assert!(gpu.join_semijoin_distinct(&by_probe, &probes, &source, &steps, &[3]).is_err());
    Ok(())
}
