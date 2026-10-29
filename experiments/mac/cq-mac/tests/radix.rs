#![cfg(target_os = "macos")]
use cq_mac::{IndexSpec, MacContext, MacError, MacRadixRun, Payload};
use cq_storage::{IndexRead, RadixRun};

const SPEC: IndexSpec = IndexSpec {
    key_column: 0,
    payload: Payload::Column(1),
};

fn check(run: &MacRadixRun, rows: &[(u32, u32)]) {
    let native = RadixRun::from_pairs(rows.iter().copied());
    let mut sorted = rows.to_vec();
    sorted.sort_by_key(|r| r.0);
    assert_eq!(run.len(), rows.len());
    assert_eq!(run.items(), sorted.iter().map(|r| r.1).collect::<Vec<_>>());
    assert_eq!(run.keys(), native.keys().copied().collect::<Vec<_>>());
    assert_eq!(run.offsets().len(), run.keys().len() + 1);
    let mut queries = vec![0, 1, u32::MAX - 1, u32::MAX];
    for (i, &key) in run.keys().iter().enumerate() {
        queries.extend([key, key.saturating_sub(1), key.saturating_add(1)]);
        if let Some(&next) = run.keys().get(i + 1) {
            queries.push(key + (next - key) / 2);
        }
    }
    queries.sort_unstable();
    queries.dedup();
    for key in queries {
        let lo = sorted.partition_point(|r| r.0 < key);
        let hi = sorted.partition_point(|r| r.0 <= key);
        let expected = sorted[lo..hi].iter().map(|r| r.1).collect::<Vec<_>>();
        assert_eq!(run.lookup(&key).collect::<Vec<_>>(), expected, "key={key}");
        assert_eq!((&native).lookup(&key).collect::<Vec<_>>(), expected);
        assert_eq!(run.contains(&key), lo != hi);
        let cursor = run.lookup(&key);
        assert_eq!(cursor.len(), expected.len());
        assert_eq!(cursor.collect::<Vec<_>>(), expected);
    }
}

#[test]
fn gpu_spline_reader_covers_chunk_seams_gaps_and_duplicate_bags() {
    let context = MacContext::new().unwrap();
    for n in [0, 1, 2, 255, 256, 257, 511, 512, 513, 4097, 65539] {
        for mode in 0..3 {
            let mut random = 0x831a53d7u32;
            let rows = (0..n)
                .map(|i| {
                    random ^= random << 13;
                    random ^= random >> 17;
                    random ^= random << 5;
                    let key = match mode {
                        0 => (i as u32 / 3) * 2,
                        1 => random,
                        _ => match i % 5 {
                            0 => 0,
                            1 => u32::MAX,
                            _ => random % 97,
                        },
                    };
                    (key, (i as u32 / 2).wrapping_mul(104729))
                })
                .collect::<Vec<_>>();
            let source = context.relation_u32(n, |i| [rows[i].0, rows[i].1]).unwrap();
            let run = context.build_radix(&source, SPEC, None).unwrap();
            check(&run, &rows);
            assert_eq!(
                source.u32_values().unwrap(),
                rows.iter().flat_map(|r| [r.0, r.1]).collect::<Vec<_>>()
            );
        }
    }
}

#[test]
fn merge_preserves_chronology_and_pinned_native_cursors() {
    let context = MacContext::new().unwrap();
    let older_rows = [(u32::MAX, 8), (7, 3), (7, 3), (0, 9)];
    let newer_rows = [(7, 1), (u32::MAX, 2), (0, 4), (8, 6), (7, 0)];
    let old_source = context
        .relation_u32(older_rows.len(), |i| [older_rows[i].0, older_rows[i].1])
        .unwrap();
    let new_source = context
        .relation_u32(newer_rows.len(), |i| [newer_rows[i].0, newer_rows[i].1])
        .unwrap();
    let old = context.build_radix(&old_source, SPEC, None).unwrap();
    let new = context.build_radix(&new_source, SPEC, None).unwrap();
    let pinned = old.clone();
    let mut cursor = pinned.lookup(&7);
    assert_eq!(cursor.next(), Some(3));
    let merged = context.merge_radix(&old, &new).unwrap();
    check(
        &merged,
        &older_rows.into_iter().chain(newer_rows).collect::<Vec<_>>(),
    );
    let empty_source = context.relation_u32(0, |_| [0u32, 0]).unwrap();
    let empty = context.build_radix(&empty_source, SPEC, None).unwrap();
    check(&context.merge_radix(&old, &empty).unwrap(), &older_rows);
    check(&context.merge_radix(&empty, &new).unwrap(), &newer_rows);
    check(&context.merge_radix(&empty, &empty).unwrap(), &[]);
    let foreign = MacContext::new().unwrap();
    assert!(matches!(
        foreign.merge_radix(&old, &new),
        Err(MacError::ContextMismatch)
    ));
    drop(old);
    drop(new);
    drop(context);
    cq_mac::memory::clear_cache().unwrap();
    assert_eq!(cursor.collect::<Vec<_>>(), [3]);
    assert_eq!(merged.lookup(&7).collect::<Vec<_>>(), [3, 3, 1, 0]);
}

#[test]
fn selected_wide_sources_validate_only_selected_fields_and_keep_original_ids() {
    let context = MacContext::new().unwrap();
    let rows = [[8i64, -9], [-1, i64::MAX], [8, 99], [0, i64::MIN]];
    let source = context.relation_i64(rows.len(), |i| rows[i]).unwrap();
    let mask = context
        .relation_u32(rows.len(), |i| [u32::from(i != 1)])
        .unwrap();
    let spec = IndexSpec {
        key_column: 0,
        payload: Payload::RowId,
    };
    let run = context.build_radix(&source, spec, Some(&mask)).unwrap();
    check(&run, &[(8, 0), (8, 2), (0, 3)]);
    assert!(context.build_radix(&source, spec, None).is_err());
    assert_eq!(source.i64_values().unwrap(), rows.as_flattened());
    assert_eq!(run.lookup(&8).collect::<Vec<_>>(), [0, 2]);
}
