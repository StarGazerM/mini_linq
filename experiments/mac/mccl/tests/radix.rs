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
