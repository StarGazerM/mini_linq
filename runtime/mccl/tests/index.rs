use mccl::{
    index::Index,
    memory,
    relation::{IndexSpec, Payload, Relation},
    Gpu, Result,
};

const PAIRS: IndexSpec = IndexSpec {
    key_column: 0,
    payload: Payload::Column(1),
};
const ROW_IDS: IndexSpec = IndexSpec {
    key_column: 0,
    payload: Payload::RowId,
};
type Builder = fn(&Gpu, &Relation, IndexSpec, Option<&Relation>) -> Result<Index>;
const BUILDERS: [Builder; 3] = [
    Gpu::build_index,
    Gpu::build_index_on_cpu,
    Gpu::build_index_on_cpu_radix,
];

fn ingest(gpu: &Gpu, rows: &[(u32, u32)]) -> Result<Relation> {
    gpu.relation_u32(rows.len(), |i| [rows[i].0, rows[i].1])
}

fn batch(len: usize, mask: u32) -> Vec<(u32, u32)> {
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
            (
                (key & mask) | (0xa537c912 & !mask),
                if i % 9 <= 2 { 42 } else { state.rotate_left(7) },
            )
        })
        .collect()
}

fn check(index: &Index, input: &[(u32, u32)]) {
    let mut expected = input.to_vec();
    expected.sort_by_key(|row| row.0);
    assert_eq!(index.len(), input.len());
    assert_eq!(index.is_empty(), input.is_empty());
    assert_eq!(
        index
            .rows()
            .iter()
            .map(|row| (row.key, row.value))
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(index.directory().len(), 4097);
    for prefix in 0..4097usize {
        let want = expected.partition_point(|row| ((row.0 >> 20) as usize) < prefix);
        assert_eq!(index.directory()[prefix], want as u32, "prefix {prefix}");
    }
    let distinct_keys = expected
        .iter()
        .enumerate()
        .filter(|(i, row)| *i == 0 || expected[*i - 1].0 != row.0)
        .map(|(_, row)| row.0);
    for key in distinct_keys.chain([
        0,
        1,
        0x000f_ffff,
        0x0010_0000,
        0x8000_0000,
        u32::MAX - 1,
        u32::MAX,
    ]) {
        let begin = expected.partition_point(|row| row.0 < key);
        let end = begin + expected[begin..].partition_point(|row| row.0 == key);
        let wanted = &expected[begin..end];
        assert_eq!(index.contains(&key), !wanted.is_empty());
        let mut cursor = index.lookup(&key);
        assert_eq!(cursor.len(), wanted.len());
        for (i, row) in wanted.iter().enumerate() {
            assert_eq!(cursor.next(), Some(row.1));
            assert_eq!(
                cursor.size_hint(),
                (wanted.len() - i - 1, Some(wanted.len() - i - 1))
            );
        }
        assert_eq!(cursor.next(), None);
        assert_eq!(cursor.next(), None);
    }
}

#[test]
fn stable_full_width_builds_and_exact_directory() -> Result<()> {
    let gpu = Gpu::new()?;
    for len in [0, 1, 31, 255, 256, 2047, 2048, 2049, 65539] {
        let rows = batch(len, u32::MAX);
        let relation = ingest(&gpu, &rows)?;
        for build in BUILDERS {
            check(&build(&gpu, &relation, PAIRS, None)?, &rows);
        }
    }
    let boundaries = (0..4096u32)
        .rev()
        .flat_map(|prefix| {
            [
                ((prefix << 20) | 0x000f_ffff, prefix ^ u32::MAX),
                (prefix << 20, prefix),
                (prefix << 20, prefix),
            ]
        })
        .collect::<Vec<_>>();
    let relation = ingest(&gpu, &boundaries)?;
    for build in BUILDERS {
        check(&build(&gpu, &relation, PAIRS, None)?, &boundaries);
    }
    Ok(())
}

#[test]
fn radix_skips_constant_bytes_without_changing_stability() -> Result<()> {
    let gpu = Gpu::new()?;
    for mask in [
        0,
        0xff,
        0xff00,
        0xff0000,
        0xff000000,
        0xffff,
        0xff00ff,
        0xffffff,
        u32::MAX,
    ] {
        let rows = batch(4097, mask);
        let relation = ingest(&gpu, &rows)?;
        check(
            &gpu.build_index_on_cpu_radix(&relation, PAIRS, None)?,
            &rows,
        );
    }
    Ok(())
}

#[test]
fn stable_merge_and_context_identity() -> Result<()> {
    let gpu = Gpu::new()?;
    let clone = gpu.clone();
    let foreign = Gpu::new()?;
    let a = [(u32::MAX, 3), (0, 9), (17, 4), (17, 4), (17, 1)];
    let b = [(17, 0), (u32::MAX, 2), (0, 8), (17, 7)];
    let big_a = batch(2049, u32::MAX);
    let big_b = batch(3073, u32::MAX);
    for (a, b) in [
        (&a[..], &b[..]),
        (&a[..], &[][..]),
        (&[][..], &b[..]),
        (&[][..], &[][..]),
        (&big_a[..], &big_b[..]),
    ] {
        let source_a = ingest(&gpu, a)?;
        let source_b = ingest(&gpu, b)?;
        let older = gpu.build_index_on_cpu_radix(&source_a, PAIRS, None)?;
        let newer = gpu.build_index(&source_b, PAIRS, None)?;
        assert!(older.belongs_to(&clone));
        assert!(!older.belongs_to(&foreign));
        assert!(foreign.merge_indexes(&older, &newer).is_err());
        let empty_source = ingest(&foreign, &[])?;
        let foreign_empty = foreign.build_index_on_cpu(&empty_source, PAIRS, None)?;
        assert!(gpu.merge_indexes(&older, &foreign_empty).is_err());
        assert!(gpu.merge_indexes(&foreign_empty, &newer).is_err());
        for (first, second, input) in [
            (
                &older,
                &newer,
                a.iter().chain(b).copied().collect::<Vec<_>>(),
            ),
            (
                &newer,
                &older,
                b.iter().chain(a).copied().collect::<Vec<_>>(),
            ),
        ] {
            check(&clone.merge_indexes(first, second)?, &input);
        }
        check(&older, a);
        check(&newer, b);
    }
    Ok(())
}

#[test]
fn ready_borrows_survive_source_drop_cache_churn_and_context_drop() -> Result<()> {
    let gpu = Gpu::new()?;
    let mut source = batch(4097, u32::MAX);
    let original = source.clone();
    let relation = ingest(&gpu, &source)?;
    let indexes = [
        gpu.build_index(&relation, PAIRS, None)?,
        gpu.build_index_on_cpu(&relation, PAIRS, None)?,
        gpu.build_index_on_cpu_radix(&relation, PAIRS, None)?,
    ];
    drop(relation);
    source.fill((123, 456));
    drop(source);
    for index in &indexes {
        let clone = index.clone();
        let rows = clone.rows();
        let directory = clone.directory();
        let saved_rows = rows.to_vec();
        let saved_directory = directory.to_vec();
        let mut cursor = clone.lookup(&u32::MAX);
        let expected = original
            .iter()
            .filter(|row| row.0 == u32::MAX)
            .map(|row| row.1)
            .collect::<Vec<_>>();
        assert_eq!(cursor.next(), expected.first().copied());
        let copied = cursor;
        for len in [0, 17, 8193, 2049, 1, 4097] {
            let input = batch(len, u32::MAX);
            let relation = ingest(&gpu, &input)?;
            let temporary = gpu.build_index(&relation, PAIRS, None)?;
            let merged = gpu.merge_indexes(index, &temporary)?;
            drop(temporary);
            drop(merged);
            memory::clear_cache()?;
            assert_eq!(rows, saved_rows);
            assert_eq!(directory, saved_directory);
        }
        assert_eq!(cursor.collect::<Vec<_>>(), expected[1..]);
        assert_eq!(copied.collect::<Vec<_>>(), expected[1..]);
    }
    drop(gpu);
    memory::clear_cache()?;
    for index in indexes {
        check(&index, &original);
    }
    Ok(())
}

#[test]
fn shared_source_repeated_columns_payloads_and_compaction() -> Result<()> {
    let gpu = Gpu::new()?;
    // Cross both radix tile and scan boundaries with duplicate keys/payloads.
    let rows = (0..8197usize)
        .map(|i| [(i % 17) as u32, ((8197 - i) % 31) as u32, (i % 7) as u32])
        .collect::<Vec<_>>();
    let source = gpu.relation_u32(rows.len(), |i| rows[i])?;
    let saved = source.u32_values()?.to_vec();
    let borrowed = source.u32_values()?;
    let mask = gpu.relation_u32(rows.len(), |i| [if i % 5 == 1 { 0 } else { 7 }])?;
    let all = gpu.relation_u32(rows.len(), |_| [u32::MAX])?;
    let none = gpu.relation_u32(rows.len(), |_| [0])?;
    for round in 0..2 {
        for key_column in 0..3 {
            for payload in [Payload::RowId, Payload::Column(0), Payload::Column(2)] {
                let spec = IndexSpec {
                    key_column,
                    payload,
                };
                for selection in [None, Some(&mask), Some(&all), Some(&none)] {
                    let selected = selection.map(Relation::u32_values).transpose()?;
                    let expected = rows
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| selected.is_none_or(|mask| mask[*i] != 0))
                        .map(|(i, row)| {
                            (
                                row[key_column],
                                match payload {
                                    Payload::RowId => i as u32,
                                    Payload::Column(column) => row[column],
                                },
                            )
                        })
                        .collect::<Vec<_>>();
                    for build in BUILDERS {
                        let index = build(&gpu, &source, spec, selection)?;
                        check(&index, &expected);
                        let doubled = gpu.merge_indexes(&index, &index)?;
                        check(
                            &doubled,
                            &expected
                                .iter()
                                .chain(&expected)
                                .copied()
                                .collect::<Vec<_>>(),
                        );
                        assert_eq!(borrowed, saved, "round {round}");
                    }
                }
            }
        }
        memory::clear_cache()?;
        assert_eq!(source.u32_values()?, saved);
    }
    Ok(())
}

#[test]
fn signed_wide_source_checks_only_selected_index_fields() -> Result<()> {
    let gpu = Gpu::new()?;
    let rows = [
        [7, 41, i64::MIN],
        [-1, 52, i64::MAX],
        [u32::MAX as i64, u32::MAX as i64, -999],
        [7, 43, i64::MAX],
        [u32::MAX as i64 + 1, 54, -1],
        [9, -1, 0],
        [10, u32::MAX as i64 + 1, 1],
    ];
    let source = gpu.relation_i64(rows.len(), |i| rows[i])?;
    assert_eq!(source.i64_values()?, rows.as_flattened());
    assert!(source.u32_values().is_err());
    let good = gpu.relation_u32(rows.len(), |i| [u32::from(matches!(i, 0 | 2 | 3))])?;
    let none = gpu.relation_u32(rows.len(), |_| [0])?;
    for build in BUILDERS {
        check(
            &build(&gpu, &source, PAIRS, Some(&good))?,
            &[(7, 41), (u32::MAX, u32::MAX), (7, 43)],
        );
        check(
            &build(&gpu, &source, ROW_IDS, Some(&good))?,
            &[(7, 0), (u32::MAX, 2), (7, 3)],
        );
        check(&build(&gpu, &source, PAIRS, Some(&none))?, &[]);
        for invalid in [1, 4, 5, 6] {
            let single = gpu.relation_u32(rows.len(), |i| [u32::from(i == invalid)])?;
            assert!(build(&gpu, &source, PAIRS, Some(&single)).is_err());
            if invalid >= 5 {
                // Invalid payload columns are irrelevant to row-ID construction.
                check(
                    &build(&gpu, &source, ROW_IDS, Some(&single))?,
                    &[(rows[invalid][0] as u32, invalid as u32)],
                );
            } else {
                assert!(build(&gpu, &source, ROW_IDS, Some(&single)).is_err());
            }
        }
        assert!(build(&gpu, &source, PAIRS, None).is_err());
        // Failed builds do not invalidate the source or a subsequent valid build.
        check(
            &build(&gpu, &source, PAIRS, Some(&good))?,
            &[(7, 41), (u32::MAX, u32::MAX), (7, 43)],
        );
        assert_eq!(source.i64_values()?, rows.as_flattened());
    }
    Ok(())
}

#[test]
fn lazy_gpu_array_relation_retains_inputs_and_context() -> Result<()> {
    let gpu = Gpu::new()?;
    let retained_context = gpu.clone();
    let host = [8, 30, 2, 31, 8, 32, 0, 33];
    let uploaded = gpu.upload(&host)?;
    let produced = gpu.copy(&uploaded)?;
    // No eval/as_slice/to_vec before the relation build. Reshaping retains the
    // lazy GPU-produced dependency rather than materializing it on the host.
    let source = gpu.relation_from_u32(&produced, 2)?;
    let clone = source.clone();
    drop(produced);
    drop(uploaded);
    drop(gpu);
    memory::clear_cache()?;
    let index = retained_context.build_index(&source, PAIRS, None)?;
    check(&index, &[(8, 30), (2, 31), (8, 32), (0, 33)]);
    let borrowed = source.u32_values()?;
    for _ in 0..4 {
        let churn = retained_context.relation_u32(16389, |i| [i as u32, 99])?;
        drop(retained_context.build_index(&churn, ROW_IDS, None)?);
        drop(churn);
        memory::clear_cache()?;
        assert_eq!(borrowed, host);
    }
    check(
        &retained_context.build_index(
            &clone,
            IndexSpec {
                key_column: 1,
                payload: Payload::RowId,
            },
            None,
        )?,
        &[(30, 0), (31, 1), (32, 2), (33, 3)],
    );
    drop(retained_context);
    memory::clear_cache()?;
    assert_eq!(source.u32_values()?, host);
    drop(source);
    assert_eq!(clone.u32_values()?, host);
    check(&index, &[(8, 30), (2, 31), (8, 32), (0, 33)]);
    Ok(())
}

#[test]
fn relation_shapes_contexts_empty_inputs_and_ingestion_panic() -> Result<()> {
    let gpu = Gpu::new()?;
    let foreign = Gpu::new()?;
    let source = gpu.relation_u32(3, |i| [i as u32, 99])?;
    assert!(source.belongs_to(&gpu.clone()));
    assert!(!source.belongs_to(&foreign));
    assert_eq!(source.columns(), 2);
    assert_eq!(source.len(), 3);
    assert!(!source.is_empty());
    assert!(source.i64_values().is_err());
    assert!(gpu.relation_u32::<0>(1, |_| []).is_err());
    assert!(gpu.relation_i64::<0>(0, |_| []).is_err());
    assert!(gpu
        .relation_u32::<1>(mccl::index::MAX_ROWS + 1, |_| [0])
        .is_err());
    let array = gpu.upload(&[1, 2, 3])?;
    assert!(gpu.relation_from_u32(&array, 0).is_err());
    assert!(gpu.relation_from_u32(&array, 2).is_err());
    assert!(foreign.relation_from_u32(&array, 1).is_err());
    let masks = [
        gpu.relation_u32(2, |_| [1])?,
        gpu.relation_u32(3, |_| [1, 1])?,
        gpu.relation_i64(3, |_| [1])?,
        foreign.relation_u32(3, |_| [1])?,
    ];
    for build in BUILDERS {
        assert!(build(&foreign, &source, PAIRS, None).is_err());
        for mask in &masks {
            assert!(build(&gpu, &source, PAIRS, Some(mask)).is_err());
        }
        assert!(build(
            &gpu,
            &source,
            IndexSpec {
                key_column: 2,
                payload: Payload::RowId,
            },
            None
        )
        .is_err());
        assert!(build(
            &gpu,
            &source,
            IndexSpec {
                key_column: 0,
                payload: Payload::Column(2),
            },
            None
        )
        .is_err());
        let empty = gpu.relation_i64::<2>(0, |_| unreachable!())?;
        let mask = gpu.relation_u32::<1>(0, |_| unreachable!())?;
        assert!(empty.is_empty());
        assert_eq!(empty.i64_values()?, &[]);
        check(&build(&gpu, &empty, PAIRS, None)?, &[]);
        check(&build(&gpu, &empty, PAIRS, Some(&mask))?, &[]);
    }
    for signed in [false, true] {
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if signed {
                let _ = gpu.relation_i64(100, |i| {
                    assert_ne!(i, 43, "deliberate loader failure");
                    [i as i64, i64::MAX]
                });
            } else {
                let _ = gpu.relation_u32(100, |i| {
                    assert_ne!(i, 43, "deliberate loader failure");
                    [i as u32, 0]
                });
            }
        }));
        assert!(panic.is_err());
        memory::clear_cache()?;
        check(
            &gpu.build_index(&source, PAIRS, None)?,
            &[(0, 99), (1, 99), (2, 99)],
        );
    }
    Ok(())
}
