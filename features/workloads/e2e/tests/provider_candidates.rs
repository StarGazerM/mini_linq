use cq_storage::{Build, Candidates, IndexRead, Keys, Trie};
use cq_workloads::SortedIndex;

#[test]
fn sorted_representation_supplies_distinct_trie_candidates_with_duplicate_payloads() {
    let rows = [(2, (7, 70)), (2, (7, 71)), (2, (8, 80)), (2, (7, 70))];
    let trie: Trie<'_, _, SortedIndex<_, _>> = Build::build(rows.into_iter());
    assert_eq!(Candidates::count(&&trie, &2), 2);
    assert_eq!(
        Candidates::keys(&&trie, &2).copied().collect::<Vec<_>>(),
        [7, 8]
    );
    assert_eq!(Candidates::count(&&trie, &9), 0);
    assert!(Candidates::keys(&&trie, &9).next().is_none());

    let inner = (&trie).lookup(&2).next().unwrap();
    assert_eq!(inner.key_count(), inner.distinct_keys().count());
    assert_eq!(inner.lookup(&7).collect::<Vec<_>>(), [70, 71, 70]);
    let empty: SortedIndex<i32, i32> = Build::build(std::iter::empty());
    assert_eq!(empty.key_count(), 0);
    assert!(empty.distinct_keys().next().is_none());
}

#[test]
fn owned_lookup_cursors_are_independent_and_keep_buckets_alive() {
    let index: SortedIndex<_, _> = Build::build([(2, 70), (1, 10), (2, 71), (2, 70)].into_iter());
    let clone = index.clone();
    assert_eq!(Candidates::count(&index, &2), 3);
    assert_eq!(Candidates::keys(&index, &2).collect::<Vec<_>>(), [70, 71]);
    assert_eq!(Candidates::count(&index, &9), 0);
    assert!(Candidates::keys(&index, &9).next().is_none());
    let mut first = index.lookup(&2);
    let second = clone.lookup(&2);
    assert_eq!(first.next(), Some(70));
    assert!(index.lookup(&9).next().is_none());
    drop(index);
    drop(clone);
    assert_eq!(first.by_ref().collect::<Vec<_>>(), [71, 70]);
    assert!(first.next().is_none());
    assert_eq!(second.collect::<Vec<_>>(), [70, 71, 70]);
}
