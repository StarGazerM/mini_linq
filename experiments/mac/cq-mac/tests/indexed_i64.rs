#![cfg(target_os = "macos")]

use cq_mac::{IndexedI64Relation, MacContext, MacError, Relation};

fn prepare<const KEY_COLUMN: usize>(
    context: &MacContext,
    source: &Relation,
    on_cpu: bool,
) -> Result<IndexedI64Relation<KEY_COLUMN>, MacError> {
    if on_cpu {
        context.prepare_radix_i64_on_cpu(source)
    } else {
        context.prepare_radix_i64(source)
    }
}

fn require_copy<T: Copy>(_: T) {}

#[test]
fn cpu_and_gpu_bind_all_rows_without_narrowing_payloads() {
    let context = MacContext::new().unwrap();
    for n in [0, 1, 2, 257, 4097] {
        let rows = (0..n)
            .map(|i| {
                let key = match i % 7 {
                    0 => 0,
                    1 => i64::from(u32::MAX),
                    2 | 3 => 17,
                    _ => (i % 97) as i64 * 2,
                };
                [i64::MIN + i as i64, key, i64::MAX - i as i64]
            })
            .collect::<Vec<_>>();
        let source = context.relation_i64(n, |i| rows[i]).unwrap();
        let source_values = source.i64_values().unwrap();
        let gpu = context.prepare_radix_i64::<1>(&source).unwrap();
        let cpu = context.prepare_radix_i64_on_cpu::<1>(&source).unwrap();
        for bound in [&cpu, &gpu] {
            assert_eq!(bound.len(), n);
            assert_eq!(bound.is_empty(), n == 0);
            assert_eq!(bound.columns(), 3);
            assert_eq!(bound.values(), rows.as_flattened());
            assert_eq!(bound.values().as_ptr(), source_values.as_ptr());
            for (row_id, row) in rows.iter().enumerate() {
                assert_eq!(bound.row(row_id as u32), row);
            }
            for key in (-1..=195).chain([
                i64::MIN,
                i64::from(u32::MAX) - 1,
                i64::from(u32::MAX),
                i64::from(u32::MAX) + 1,
                i64::MAX,
            ]) {
                let expected = rows
                    .iter()
                    .enumerate()
                    .filter_map(|(row_id, row)| (row[1] == key).then_some(row_id as u32))
                    .collect::<Vec<_>>();
                let cursor = bound.row_ids(key);
                assert_eq!(cursor.is_none(), expected.is_empty(), "key={key}");
                if let Some(cursor) = cursor {
                    require_copy(cursor);
                    assert_eq!(cursor.len(), expected.len());
                    assert_eq!(cursor.collect::<Vec<_>>(), expected, "key={key}");
                    for row_id in cursor {
                        assert_eq!(bound.row(row_id), rows[row_id as usize]);
                    }
                }
            }
        }
    }
}

#[test]
fn source_key_domain_is_checked_but_other_columns_remain_i64() {
    let context = MacContext::new().unwrap();
    for bad in [i64::MIN, -1, i64::from(u32::MAX) + 1, i64::MAX] {
        let source = context
            .relation_i64(3, |i| [i64::MIN, if i == 2 { bad } else { 7 }, i64::MAX])
            .unwrap();
        for on_cpu in [false, true] {
            assert!(prepare::<1>(&context, &source, on_cpu).is_err());
        }
        assert_eq!(source.i64_values().unwrap()[7], bad);
    }
    let source = context
        .relation_i64(2, |i| [i64::MIN, i as i64, i64::MAX])
        .unwrap();
    for on_cpu in [false, true] {
        let bound = prepare::<1>(&context, &source, on_cpu).unwrap();
        assert_eq!(bound.row(0), [i64::MIN, 0, i64::MAX]);
        assert_eq!(bound.row(1), [i64::MIN, 1, i64::MAX]);
        // The same-shaped source is invalid when a different declared column
        // is indexed: key selection is not inferred from representable values.
        assert!(prepare::<0>(&context, &source, on_cpu).is_err());
        assert!(prepare::<2>(&context, &source, on_cpu).is_err());
    }
}

#[test]
fn preparation_rejects_wrong_dtype_slot_context_and_unconstructible_shapes() {
    let context = MacContext::new().unwrap();
    let foreign = MacContext::new().unwrap();
    for n in [0, 2] {
        let wide = context.relation_i64(n, |_| [0, 1]).unwrap();
        let narrow = context.relation_u32(n, |_| [0, 1]).unwrap();
        for on_cpu in [false, true] {
            assert!(prepare::<0>(&context, &narrow, on_cpu).is_err());
            assert!(prepare::<2>(&context, &wide, on_cpu).is_err());
            assert!(prepare::<{ usize::MAX }>(&context, &wide, on_cpu).is_err());
            assert!(matches!(
                prepare::<1>(&foreign, &wide, on_cpu),
                Err(MacError::ContextMismatch)
            ));
        }
    }
    // Relations cannot expose malformed shapes or excessive row counts to
    // preparation: ingestion rejects them before invoking a row initializer.
    assert!(context.relation_i64::<0>(0, |_| []).is_err());
    assert!(
        context
            .relation_i64::<1>(cq_mac::backend::MAX_ROWS + 1, |_| panic!(
                "must not initialize"
            ))
            .is_err()
    );
}

#[test]
fn independent_sources_keep_their_own_identity_and_declared_key_column() {
    let context = MacContext::new().unwrap();
    let left_rows = [[4, 8, i64::MIN], [8, 4, i64::MAX], [4, 8, -99]];
    let right_rows = [[40, 8, -100], [80, 4, 100], [40, 8, -200]];
    let left = context.relation_i64(3, |i| left_rows[i]).unwrap();
    let right = context.relation_i64(3, |i| right_rows[i]).unwrap();
    for on_cpu in [false, true] {
        let left_key1 = prepare::<1>(&context, &left, on_cpu).unwrap();
        let right_key1 = prepare::<1>(&context, &right, on_cpu).unwrap();
        let left_key0 = prepare::<0>(&context, &left, on_cpu).unwrap();
        assert_eq!(
            left_key1.values().as_ptr(),
            left.i64_values().unwrap().as_ptr()
        );
        assert_eq!(
            right_key1.values().as_ptr(),
            right.i64_values().unwrap().as_ptr()
        );
        assert_ne!(left_key1.values().as_ptr(), right_key1.values().as_ptr());
        assert_eq!(left_key1.row_ids(8).unwrap().collect::<Vec<_>>(), [0, 2]);
        assert_eq!(right_key1.row_ids(8).unwrap().collect::<Vec<_>>(), [0, 2]);
        assert_eq!(left_key0.row_ids(8).unwrap().collect::<Vec<_>>(), [1]);
        assert_eq!(left_key1.row(0), left_rows[0]);
        assert_eq!(right_key1.row(0), right_rows[0]);
    }
}

#[test]
fn borrowed_payloads_and_cursors_survive_owner_drop_and_allocator_churn() {
    for on_cpu in [false, true] {
        let context = MacContext::new().unwrap();
        let rows = [[i64::MIN, 7, i64::MAX], [-9, 3, 9], [-11, 7, 11]];
        let source = context.relation_i64(rows.len(), |i| rows[i]).unwrap();
        let original_pointer = source.i64_values().unwrap().as_ptr();
        let bound = prepare::<1>(&context, &source, on_cpu).unwrap();
        let retained = bound.clone();
        let values = retained.values();
        let payload = retained.row(2);
        let mut cursor = retained.row_ids(7).unwrap();
        assert_eq!(cursor.next(), Some(0));
        drop(bound);
        drop(source);
        drop(context);
        cq_mac::memory::clear_cache().unwrap();

        let fresh_context = MacContext::new().unwrap();
        for generation in 0..8 {
            let fresh_source = fresh_context
                .relation_i64(257, |i| [generation, (i % 5) as i64, -(i as i64)])
                .unwrap();
            let gpu = fresh_context.prepare_radix_i64::<1>(&fresh_source).unwrap();
            let cpu = fresh_context
                .prepare_radix_i64_on_cpu::<1>(&fresh_source)
                .unwrap();
            assert_eq!(gpu.row_ids(4).unwrap().len(), cpu.row_ids(4).unwrap().len());
            assert_eq!(values, rows.as_flattened());
            assert_eq!(payload, rows[2]);
        }
        drop(fresh_context);
        cq_mac::memory::clear_cache().unwrap();
        assert_eq!(retained.values().as_ptr(), original_pointer);
        assert_eq!(values, rows.as_flattened());
        assert_eq!(payload, rows[2]);
        assert_eq!(cursor.len(), 1);
        assert_eq!(cursor.collect::<Vec<_>>(), [2]);
        assert_eq!(retained.row_ids(7).unwrap().collect::<Vec<_>>(), [0, 2]);
    }
}
