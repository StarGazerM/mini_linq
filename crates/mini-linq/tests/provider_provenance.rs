//! Block handles retain storage identity even when payload records are zero sized.
use cq_storage::{BlockDescriptor, BlockRead, BlockStore, Counters};
use mini_linq::block_rows;
use std::collections::HashSet;
use std::num::NonZeroUsize;

fn size() -> NonZeroUsize {
    NonZeroUsize::new(2).unwrap()
}

#[test]
fn zero_column_handles_select_their_own_store() {
    let store = BlockStore::<0>::from_rows([[]], size());
    let work = Counters::default();
    let input = block_rows! { storage cq_storage; input (&store) };
    let handles: HashSet<_> =
        block_rows! { storage cq_storage; work (&work); rows (&input) }.collect();
    let selected = block_rows! { storage cq_storage; select (&input) (handles) };
    assert_eq!(block_rows! { storage cq_storage; len (&selected) }, 1);
    let output: Vec<_> = block_rows! { storage cq_storage; work (&work); scan (&selected) }
        .copied()
        .collect();
    assert_eq!(output, vec![[]]);
}

#[test]
#[should_panic(expected = "selected row belongs to another block store")]
fn zero_column_foreign_handles_are_rejected() {
    let left = BlockStore::<0>::from_rows([[]], size());
    let right = BlockStore::<0>::from_rows([[]], size());
    // Zero-sized payload addresses cannot establish storage identity.
    assert!(std::ptr::eq(
        left.blocks().next().unwrap().rows().next().unwrap().1,
        right.blocks().next().unwrap().rows().next().unwrap().1,
    ));
    let work = Counters::default();
    let left = block_rows! { storage cq_storage; input (&left) };
    let right = block_rows! { storage cq_storage; input (&right) };
    let foreign: HashSet<_> =
        block_rows! { storage cq_storage; work (&work); rows (&right) }.collect();
    let _ = block_rows! { storage cq_storage; select (&left) (foreign) };
}

#[test]
#[should_panic(expected = "block descriptor belongs to another block store")]
fn zero_column_foreign_descriptor_is_rejected_before_payload_access() {
    let left = BlockStore::<0>::from_rows([[]], size());
    let right = BlockStore::<0>::from_rows([[]], size());
    let work = Counters::default();
    let left = block_rows! { storage cq_storage; input (&left) };
    let right = block_rows! { storage cq_storage; input (&right) };
    let foreign = block_rows! { storage cq_storage; work (&work); blocks (&right) }
        .next()
        .unwrap();
    // Merely requesting this mismatched descriptor's payload must reject it.
    let _ = block_rows! { storage cq_storage; work (&work); block_rows (foreign) (&left) };
}

#[test]
#[should_panic(expected = "selected coordinates are absent from the current mask")]
fn valid_store_identity_does_not_allow_removed_rows_to_return() {
    let store = BlockStore::from_rows([[1], [2]], size());
    let work = Counters::default();
    let input = block_rows! { storage cq_storage; input (&store) };
    let mut handles = block_rows! { storage cq_storage; work (&work); rows (&input) };
    let retained = handles.next().unwrap();
    let removed = handles.next().unwrap();
    let reduced = block_rows! { storage cq_storage; select (&input) (HashSet::from([retained])) };
    let _ = block_rows! { storage cq_storage; select (&reduced) (HashSet::from([removed])) };
}

#[test]
fn optional_instrumentation_preserves_access_and_evaluates_its_expression_once() {
    let store = BlockStore::from_rows([[1], [2], [3], [4], [5]], size());
    let input = block_rows! { storage cq_storage; input (&store) };
    let plain: HashSet<_> = block_rows! { storage cq_storage; scan (&input) }
        .copied()
        .collect();
    let work = Counters::default();
    let evaluations = std::cell::Cell::new(0);
    let observed = block_rows! {
        storage cq_storage;
        work ({ evaluations.set(evaluations.get() + 1); &work });
        scan (&input)
    };
    assert_eq!(evaluations.get(), 1);
    assert_eq!(work.take(), Default::default());
    assert_eq!(observed.copied().collect::<HashSet<_>>(), plain);
    assert_eq!(evaluations.get(), 1);
    let counts = work.take();
    assert_eq!(counts.rows_visited, 5);
    assert_eq!(counts.blocks_opened, 3);

    let rows: HashSet<_> = [[1], [2], [3]].into_iter().collect();
    let plain: HashSet<_> = mini_linq::hash_rows! { storage cq_storage; scan (&rows) }
        .copied()
        .collect();
    let observed = mini_linq::hash_rows! {
        storage cq_storage;
        work ({ evaluations.set(evaluations.get() + 1); &work });
        scan (&rows)
    };
    assert_eq!(evaluations.get(), 2);
    assert_eq!(observed.copied().collect::<HashSet<_>>(), plain);
    assert_eq!(evaluations.get(), 2);
    assert_eq!(work.take().rows_visited, 3);
}
