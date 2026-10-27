#![cfg(target_os = "macos")]

use cq_mac::{IndexSpec, MacContext, MaintainedIndex, MaintenancePolicy, Payload, Snapshot};

const PAIRS: IndexSpec = IndexSpec {
    key_column: 0,
    payload: Payload::Column(1),
};
use cq_storage::{Build, ExactMembership, HashIndex, IndexRead, RadixRun};
use std::collections::BTreeMap;

fn batch(n: usize, seed: u32) -> Vec<(u32, u32)> {
    let mut state = seed;
    (0..n)
        .map(|i| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let key = match i % 7 {
                0 => 0,
                1 => u32::MAX,
                2 => 17,
                3 => state % 97,
                _ => state,
            };
            (key, if i % 3 == 0 { 42 } else { state.rotate_left(7) })
        })
        .collect()
}

fn expected(rows: &[(u32, u32)]) -> BTreeMap<u32, Vec<u32>> {
    let mut result = BTreeMap::<u32, Vec<u32>>::new();
    for &(key, value) in rows {
        result.entry(key).or_default().push(value);
    }
    result
}

fn check(snapshot: &Snapshot, rows: &[(u32, u32)]) {
    assert_eq!(snapshot.len(), rows.len());
    assert_eq!(snapshot.is_empty(), rows.is_empty());
    let grouped = expected(rows);
    for key in grouped.keys().copied().chain([18, 100, u32::MAX - 1]) {
        let want = grouped.get(&key).map(Vec::as_slice).unwrap_or_default();
        let mut cursor = snapshot.lookup(&key);
        assert_eq!(cursor.len(), want.len());
        for (i, &value) in want.iter().enumerate() {
            assert_eq!(cursor.next(), Some(value));
            assert_eq!(
                cursor.size_hint(),
                (want.len() - i - 1, Some(want.len() - i - 1))
            );
        }
        assert_eq!(cursor.next(), None);
        assert_eq!(cursor.next(), None);
        assert_eq!(snapshot.contains(&key), !want.is_empty());
        assert_eq!(ExactMembership::contains(snapshot, &key), !want.is_empty());
    }
}

#[test]
fn shared_run_matches_native_providers_and_preserves_occurrences() {
    let context = MacContext::new().unwrap();
    let boundary_rows = (0..4096u32)
        .rev()
        .flat_map(|prefix| {
            [
                (prefix << 20, prefix),
                ((prefix << 20) | ((1 << 20) - 1), prefix),
            ]
        })
        .collect::<Vec<_>>();
    for rows in [0, 1, 2047, 2048, 2049, 65_539]
        .into_iter()
        .map(|n| batch(n, 0x1234abcd))
        .chain([boundary_rows])
    {
        let source = context
            .relation_u32(rows.len(), |i| [rows[i].0, rows[i].1])
            .unwrap();
        let run = context.build(&source, PAIRS, None).unwrap();
        let cpu_run = context.build_on_cpu(&source, PAIRS, None).unwrap();
        let radix = RadixRun::from_pairs(rows.iter().copied());
        let hash = HashIndex::build(rows.iter().copied());
        let grouped = expected(&rows);
        assert_eq!(run.len(), rows.len());
        assert_eq!(cpu_run.len(), run.len());
        assert_eq!(cpu_run.is_empty(), run.is_empty());
        for key in grouped
            .keys()
            .copied()
            .chain([0, 18, 100, u32::MAX - 1, u32::MAX])
            .chain((0..4096u32).flat_map(|prefix| [prefix << 20, (prefix << 20) | ((1 << 20) - 1)]))
        {
            let want = grouped.get(&key).map(Vec::as_slice).unwrap_or_default();
            let cursor = run.lookup(&key);
            assert_eq!(cursor.len(), want.len());
            assert_eq!(cursor.collect::<Vec<_>>(), want);
            let cpu_cursor = cpu_run.lookup(&key);
            assert_eq!(cpu_cursor.len(), want.len());
            assert_eq!(cpu_cursor.collect::<Vec<_>>(), want);
            assert_eq!(
                cpu_run.lookup(&key).collect::<Vec<_>>(),
                run.lookup(&key).collect::<Vec<_>>()
            );
            assert_eq!(cpu_run.contains(&key), !want.is_empty());
            assert_eq!(
                run.lookup(&key).collect::<Vec<_>>(),
                (&radix).lookup(&key).collect::<Vec<_>>()
            );
            assert_eq!(hash.lookup(&key).collect::<Vec<_>>(), want);
            assert_eq!(run.contains(&key), !want.is_empty());
        }
        // Exercise ordinary IndexRead probe dispatch, not just inherent lookup.
        let keys = [0, 17, u32::MAX, 0, 100];
        let actual = (&run)
            .probe::<4, _, _>(keys.into_iter(), |key| *key)
            .collect::<Vec<_>>();
        let want = keys
            .into_iter()
            .flat_map(|key| {
                grouped
                    .get(&key)
                    .into_iter()
                    .flatten()
                    .map(move |&value| (key, value))
            })
            .collect::<Vec<_>>();
        assert_eq!(actual, want);
    }
}

#[test]
fn online_histories_preserve_old_cursors_and_chronological_bags() {
    for policy in [MaintenancePolicy::Flat, MaintenancePolicy::BinaryCarry] {
        let context = MacContext::new().unwrap();
        let mut owner = MaintainedIndex::new(policy);
        let empty_source = context.relation_u32::<2>(0, |_| unreachable!()).unwrap();
        let empty = owner.snapshot();
        check(&empty, &[]);
        let mut all = Vec::new();
        let mut retained = Vec::new();
        for (i, n) in [5, 2049, 1, 3073, 3, 1025, 17, 4097]
            .into_iter()
            .enumerate()
        {
            let rows = batch(n, i as u32 + 1);
            all.extend_from_slice(&rows);
            let source = context
                .relation_u32(rows.len(), |i| [rows[i].0, rows[i].1])
                .unwrap();
            let snapshot = owner.append(&context, &source, PAIRS).unwrap();
            assert_eq!(snapshot.epoch(), i as u64 + 1);
            let count = match policy {
                MaintenancePolicy::Flat => 1,
                MaintenancePolicy::BinaryCarry => (i + 1).count_ones() as usize,
            };
            assert_eq!(snapshot.run_count(), count);
            check(&snapshot, &all);
            retained.push((snapshot, all.clone()));
            let before = owner.snapshot();
            let empty_update = owner.append(&context, &empty_source, PAIRS).unwrap();
            assert_eq!(empty_update.epoch(), before.epoch());
            assert_eq!(empty_update.run_count(), before.run_count());
            check(&empty_update, &all);
        }
        let pinned = &retained[2].0;
        let values = expected(&retained[2].1).remove(&0).unwrap();
        let mut cursor = pinned.lookup(&0);
        assert_eq!(cursor.next(), Some(values[0]));
        let copied = cursor;
        let rows = batch(8193, 123);
        let source = context
            .relation_u32(rows.len(), |i| [rows[i].0, rows[i].1])
            .unwrap();
        owner.append(&context, &source, PAIRS).unwrap();
        drop(owner);
        drop(context);
        assert_eq!(cursor.collect::<Vec<_>>(), values[1..]);
        assert_eq!(copied.collect::<Vec<_>>(), values[1..]);
        for (snapshot, rows) in retained {
            check(&snapshot, &rows);
        }
        check(&empty, &[]);
    }
}

#[test]
fn stable_merge_handles_empty_sides_and_nonmonotonic_payloads() {
    let context = MacContext::new().unwrap();
    let a = [(u32::MAX, 3), (0, 9), (17, 4), (17, 4), (17, 1)];
    let b = [(17, 0), (u32::MAX, 2), (0, 8), (17, 7)];
    let irregular_a = batch(2049, 0xabc123);
    let irregular_b = batch(3073, 0xdef456);
    let mut retained = Vec::new();
    for (older, newer) in [
        (&a[..], &b[..]),
        (&a[..], &[][..]),
        (&[][..], &b[..]),
        (&[][..], &[][..]),
        (&irregular_a[..], &irregular_b[..]),
    ] {
        let older_source = context
            .relation_u32(older.len(), |i| [older[i].0, older[i].1])
            .unwrap();
        let newer_source = context
            .relation_u32(newer.len(), |i| [newer[i].0, newer[i].1])
            .unwrap();
        for cpu_right in [false, true] {
            let left = context.build(&older_source, PAIRS, None).unwrap();
            let right = if cpu_right {
                context.build_on_cpu(&newer_source, PAIRS, None).unwrap()
            } else {
                context.build(&newer_source, PAIRS, None).unwrap()
            };
            // Merge the same CPU/GPU buffers in both chronological directions.
            for (first, second, first_rows, second_rows) in
                [(&left, &right, older, newer), (&right, &left, newer, older)]
            {
                let merged = context.merge(first, second).unwrap();
                let rows = first_rows
                    .iter()
                    .chain(second_rows)
                    .copied()
                    .collect::<Vec<_>>();
                retained.push((merged, rows));
            }
            retained.push((left, older.to_vec()));
            retained.push((right, newer.to_vec()));
        }
    }
    drop(context);
    // Both source construction modes and merged results remain immutable and
    // readable after all commands finish and the context is disposed.
    for (run, rows) in retained {
        assert_eq!(run.len(), rows.len());
        assert_eq!(run.is_empty(), rows.is_empty());
        let grouped = expected(&rows);
        for key in grouped
            .keys()
            .copied()
            .chain([0, 18, 100, u32::MAX - 1, u32::MAX])
        {
            let want = grouped.get(&key).map(Vec::as_slice).unwrap_or_default();
            assert_eq!(run.lookup(&key).collect::<Vec<_>>(), want);
            assert_eq!(run.contains(&key), !want.is_empty());
        }
    }
}

#[test]
fn context_mismatch_rejects_merge_and_append_without_publication() {
    use cq_mac::MacError;

    let context = MacContext::new().unwrap();
    let foreign = MacContext::new().unwrap();
    let local_source = context.relation_u32(2, |i| [[17, 4], [17, 1]][i]).unwrap();
    let foreign_source = foreign.relation_u32(1, |_| [17, 0]).unwrap();
    let local = context.build_on_cpu(&local_source, PAIRS, None).unwrap();
    let other = foreign
        .build_on_cpu_radix(&foreign_source, PAIRS, None)
        .unwrap();
    assert_eq!(
        context.build(&foreign_source, PAIRS, None).unwrap_err(),
        MacError::ContextMismatch
    );
    let foreign_mask = foreign.relation_u32(2, |_| [1]).unwrap();
    assert_eq!(
        context
            .build(&local_source, PAIRS, Some(&foreign_mask))
            .unwrap_err(),
        MacError::ContextMismatch
    );
    assert_eq!(
        context.merge(&local, &other).unwrap_err(),
        MacError::ContextMismatch
    );
    assert_eq!(
        context.merge(&other, &local).unwrap_err(),
        MacError::ContextMismatch
    );
    assert_eq!(local.lookup(&17).collect::<Vec<_>>(), [4, 1]);
    assert_eq!(other.lookup(&17).collect::<Vec<_>>(), [0]);

    for policy in [MaintenancePolicy::Flat, MaintenancePolicy::BinaryCarry] {
        let mut owner = MaintainedIndex::new(policy);
        let first = context.relation_u32(1, |_| [17, 9]).unwrap();
        let second = context.relation_u32(1, |_| [17, 8]).unwrap();
        owner.append(&context, &first, PAIRS).unwrap();
        let before = owner.append(&context, &second, PAIRS).unwrap();
        // A third binary-carry batch would not merge: reject it anyway instead
        // of publishing mixed-context runs that cannot be compacted later.
        let rejected = foreign.relation_u32(1, |_| [17, 7]).unwrap();
        let error = owner.append(&foreign, &rejected, PAIRS).err().unwrap();
        assert_eq!(error, MacError::ContextMismatch);
        assert_eq!(owner.snapshot().epoch(), before.epoch());
        assert_eq!(owner.snapshot().run_count(), before.run_count());
        check(&owner.snapshot(), &[(17, 9), (17, 8)]);
        let empty_source = foreign.relation_u32::<2>(0, |_| unreachable!()).unwrap();
        let empty = owner.append(&foreign, &empty_source, PAIRS).unwrap();
        assert_eq!(empty.epoch(), before.epoch());
        let last = context.relation_u32(1, |_| [17, 6]).unwrap();
        let after = owner.append(&context, &last, PAIRS).unwrap();
        check(&after, &[(17, 9), (17, 8), (17, 6)]);
        check(&before, &[(17, 9), (17, 8)]);
    }
}
