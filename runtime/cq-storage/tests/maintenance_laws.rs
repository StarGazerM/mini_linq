//! Ordinary provider maintenance laws, observed only through public row/read APIs.
//! The fixed-seed generator needs no property-testing dependency. Oracles compare
//! complete rows, not merely grouped keys, and retain multiplicity when checking
//! reads so an accidentally duplicated publication cannot hide behind a set.

use cq_storage::rows::Rows;
use cq_storage::{
    Build, ExactMembership, Fresh, HashMap, IndexRead, Publish, RadixRun, SortedRun, Trie,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt::Debug;
use std::hash::BuildHasher;

const KEYS: [i64; 11] = [
    i64::MIN,
    -1_000_000,
    -65,
    -1,
    0,
    1,
    63,
    64,
    65,
    1_000_000,
    i64::MAX,
];
const SEED: u64 = 0x7b92_1d6c_4e03_a5f8;

fn next(state: &mut u64) -> u64 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *state
}

fn probes() -> impl Iterator<Item = i64> {
    KEYS.into_iter().chain([2, 66])
}

fn sorted<R: Ord>(mut rows: Vec<R>) -> Vec<R> {
    rows.sort();
    rows
}

fn assert_reads<P, R>(
    index: &P,
    expected: &BTreeSet<R>,
    observe: &impl Fn(&P) -> Vec<R>,
    check_reads: &impl Fn(&P, &BTreeSet<R>),
) where
    R: Clone + Ord + Debug,
{
    let expected_rows: Vec<_> = expected.iter().cloned().collect();
    // IX-Stable: repeated scans and point reads agree between publications.
    // Compare bags, not scan order: providers may choose their physical order.
    for _ in 0..2 {
        assert_eq!(
            sorted(observe(index)),
            expected_rows,
            "IX-Publish / IX-Stable rows"
        );
        check_reads(index, expected);
    }
}

fn admit<P, R>(
    index: &mut P,
    batch: Vec<R>,
    relation: &mut BTreeSet<R>,
    observe: &impl Fn(&P) -> Vec<R>,
    check_reads: &impl Fn(&P, &BTreeSet<R>),
) -> Vec<R>
where
    P: Fresh<Row = R> + Publish<Item = R>,
    R: Clone + Ord + Debug,
{
    let support: BTreeSet<_> = batch.iter().cloned().collect();
    let expected: Vec<_> = support.difference(relation).cloned().collect();
    let fresh = index.fresh(batch.clone());
    assert_eq!(
        sorted(fresh.clone()),
        expected,
        "IX-Fresh exactness and uniqueness"
    );
    assert_reads(index, relation, observe, check_reads);

    // Fresh is observational: repeating it before publishing cannot consume rows.
    assert_eq!(
        sorted(index.fresh(batch)),
        expected,
        "IX-Fresh is read-only"
    );
    assert_reads(index, relation, observe, check_reads);

    // Only distinct, disjoint rows are published, as the contract requires.
    index.publish(fresh.clone().into_iter());
    relation.extend(fresh.iter().cloned());
    assert_reads(index, relation, observe, check_reads);
    assert!(
        index.fresh(fresh.clone()).is_empty(),
        "published rows are no longer fresh"
    );
    fresh
}

fn maintenance_laws<P, R>(
    build: impl Fn(Vec<R>) -> P,
    row: impl Fn(i64, i64, i64) -> R,
    observe: impl Fn(&P) -> Vec<R>,
    check_reads: impl Fn(&P, &BTreeSet<R>),
) where
    P: Fresh<Row = R> + Publish<Item = R>,
    R: Clone + Ord + Debug,
{
    let mut state = SEED;
    for case in 0..12 {
        // Alternate compact domains with sparse/extreme signed keys, exercising
        // both dense and fallback directories without relying on their internals.
        let keys: &[i64] = if case % 2 == 0 { &KEYS[4..9] } else { &KEYS };
        let mut sample = || {
            let key = keys[(next(&mut state) % keys.len() as u64) as usize];
            let middle = (next(&mut state) % 5) as i64;
            let payload = (next(&mut state) % 7) as i64;
            row(key, middle, payload)
        };
        let initial: BTreeSet<_> = (0..case % 5 * 3).map(|_| sample()).collect();
        let count = [0, 1, 2, 7, 37, 96][case % 6];
        let mut candidates = Vec::new();
        for _ in 0..count {
            let candidate = sample();
            candidates.extend([candidate.clone(), candidate]);
        }
        if count != 0 {
            // Same prefix/different suffix, same suffix/different prefix, exact
            // duplicates, and rows already in the initial relation.
            candidates.extend([
                row(0, 0, 0),
                row(0, 0, 1),
                row(0, 1, 0),
                row(1, 0, 0),
                row(0, 0, 0),
            ]);
            candidates.extend(initial.iter().cloned());
        }
        let expected_admitted: BTreeSet<_> = candidates
            .iter()
            .filter(|candidate| !initial.contains(*candidate))
            .cloned()
            .collect();
        let expected_final: BTreeSet<_> =
            initial.iter().chain(candidates.iter()).cloned().collect();
        let mut outcomes = Vec::new();
        // ADM-Batch: singleton admission, bounded batches, and whole-input
        // admission produce the same relation and the same newly admitted rows.
        for width in [1, 7, candidates.len().max(1)] {
            let mut index = build(initial.iter().cloned().collect());
            let mut relation = initial.clone();
            assert_reads(&index, &relation, &observe, &check_reads);
            admit(
                &mut index,
                Vec::new(),
                &mut relation,
                &observe,
                &check_reads,
            );
            let mut admitted = Vec::new();
            for batch in candidates.chunks(width) {
                admitted.extend(admit(
                    &mut index,
                    batch.to_vec(),
                    &mut relation,
                    &observe,
                    &check_reads,
                ));
            }
            assert_eq!(
                sorted(admitted),
                expected_admitted.iter().cloned().collect::<Vec<_>>()
            );
            assert_eq!(relation, expected_final, "ADM-Batch reference relation");
            assert!(
                index.fresh(candidates.clone()).is_empty(),
                "replay admits no rows"
            );
            index.publish(Vec::<R>::new());
            assert_reads(&index, &relation, &observe, &check_reads);
            outcomes.push(sorted(observe(&index)));
        }
        assert_eq!(
            outcomes[0], outcomes[1],
            "ADM-Batch row versus bounded batches"
        );
        assert_eq!(
            outcomes[1], outcomes[2],
            "ADM-Batch bounded versus whole input"
        );
    }
}

fn set_laws<P>()
where
    P: FromIterator<(i64,)>
        + Fresh<Row = (i64,)>
        + Publish<Item = (i64,)>
        + ExactMembership<(i64,)>,
    for<'a> &'a P: Rows<Row = (&'a i64,)>,
{
    maintenance_laws(
        |rows: Vec<(i64,)>| rows.into_iter().collect::<P>(),
        |key, _, _| (key,),
        |index| index.rows().map(|(key,)| (*key,)).collect(),
        |index, expected| {
            for key in probes() {
                assert_eq!(
                    ExactMembership::contains(index, &(key,)),
                    expected.contains(&(key,))
                );
            }
        },
    );
}

#[test]
fn keyed_hash_set_maintenance_laws() {
    set_laws::<cq_storage::HashSet<(i64,)>>();
}

#[test]
fn standard_hash_set_maintenance_laws() {
    set_laws::<HashSet<(i64,)>>();
}

#[test]
fn ordered_set_maintenance_laws() {
    set_laws::<BTreeSet<(i64,)>>();
}

type GroupRow = ((i64,), (i64,));

fn grouped_map_laws<S: BuildHasher + Default>() {
    maintenance_laws(
        |rows: Vec<GroupRow>| {
            let mut index = HashMap::<(i64,), Vec<(i64,)>, S>::default();
            for (key, value) in rows {
                index.entry(key).or_default().push(value);
            }
            // An allocated but empty bucket represents no row or present key.
            index.entry((66,)).or_default();
            index
        },
        |key, _, payload| ((key,), (payload,)),
        |index| {
            index
                .rows()
                .map(|(key, value)| ((*key,), (*value,)))
                .collect()
        },
        |index, expected| {
            for key in probes() {
                let values: Vec<_> = expected
                    .iter()
                    .filter(|(stored, _)| *stored == (key,))
                    .map(|(_, value)| *value)
                    .collect();
                assert_eq!(
                    ExactMembership::contains(index, &(key,)),
                    !values.is_empty()
                );
                assert_eq!(sorted(index.lookup(&(key,)).copied().collect()), values);
            }
        },
    );
}

#[test]
fn standard_grouped_hash_map_maintenance_laws() {
    grouped_map_laws::<std::collections::hash_map::RandomState>();
}

#[test]
fn keyed_grouped_hash_map_maintenance_laws() {
    // The public cq_storage alias fixes HashState by default; the same map
    // implementation above also covers the explicitly standard hasher.
    grouped_map_laws::<cq_storage::HashState>();
}

#[cfg(feature = "counters")]
#[test]
fn counted_grouped_hash_map_maintenance_laws() {
    grouped_map_laws::<std::hash::BuildHasherDefault<cq_storage::counters::CountingHasher>>();
}

#[test]
fn ordered_grouped_map_maintenance_laws() {
    maintenance_laws(
        |rows: Vec<GroupRow>| {
            let mut index = BTreeMap::<(i64,), Vec<(i64,)>>::default();
            for (key, value) in rows {
                index.entry(key).or_default().push(value);
            }
            index.entry((66,)).or_default();
            index
        },
        |key, _, payload| ((key,), (payload,)),
        |index| {
            index
                .rows()
                .map(|(key, value)| ((*key,), (*value,)))
                .collect()
        },
        |index, expected| {
            for key in probes() {
                let values: Vec<_> = expected
                    .iter()
                    .filter(|(stored, _)| *stored == (key,))
                    .map(|(_, value)| *value)
                    .collect();
                assert_eq!(
                    ExactMembership::contains(index, &(key,)),
                    !values.is_empty()
                );
                assert_eq!(sorted(index.lookup(&(key,)).copied().collect()), values);
            }
        },
    );
}

fn set_bucket_laws<S: BuildHasher + Default, T: BuildHasher + Default>() {
    maintenance_laws(
        |rows: Vec<GroupRow>| {
            let mut index = HashMap::<(i64,), HashSet<(i64,), T>, S>::default();
            for (key, value) in rows {
                index.entry(key).or_default().insert(value);
            }
            index.entry((66,)).or_default();
            index
        },
        |key, _, payload| ((key,), (payload,)),
        |index| {
            index
                .rows()
                .map(|(key, value)| ((*key,), (*value,)))
                .collect()
        },
        |index, expected| {
            for key in probes() {
                let rows: Vec<_> = expected
                    .iter()
                    .filter(|(stored, _)| *stored == (key,))
                    .copied()
                    .collect();
                let values: Vec<_> = rows.iter().map(|(_, value)| *value).collect();
                assert_eq!(
                    ExactMembership::contains(index, &(key,)),
                    !values.is_empty()
                );
                assert_eq!(sorted(index.lookup(&(key,)).copied().collect()), values);
                assert_eq!(cq_storage::Candidates::count(&index, &(key,)), values.len());
                assert_eq!(
                    sorted(
                        cq_storage::Candidates::keys(&index, &(key,))
                            .copied()
                            .collect()
                    ),
                    values
                );
                let point = {
                    let temporary_key = (key,);
                    cq_storage::PointLookup::point_lookup(&index, &temporary_key)
                };
                assert_eq!(
                    sorted(point.map(|(key, value)| ((*key,), (*value,))).collect()),
                    rows
                );
            }
        },
    );
}

#[test]
fn set_bucket_map_maintenance_and_read_laws() {
    type Standard = std::collections::hash_map::RandomState;
    set_bucket_laws::<Standard, Standard>();
    set_bucket_laws::<cq_storage::HashState, Standard>();
    set_bucket_laws::<Standard, cq_storage::HashState>();
}

#[test]
fn set_bucket_admission_hashes_payloads_instead_of_scanning_the_group() {
    thread_local! {
        static COMPARISONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    }
    #[derive(Hash)]
    struct Counted(u64);
    impl PartialEq for Counted {
        fn eq(&self, other: &Self) -> bool {
            COMPARISONS.with(|count| count.set(count.get() + 1));
            self.0 == other.0
        }
    }
    impl Eq for Counted {}
    type Fixed = std::hash::BuildHasherDefault<std::collections::hash_map::DefaultHasher>;
    let mut index = HashMap::<u64, HashSet<Counted, Fixed>>::default();
    index.publish((0..4096).map(|value| (7, Counted(value))));
    COMPARISONS.with(|count| count.set(0));
    let fresh = index.fresh(vec![(7, Counted(2048)), (7, Counted(1 << 50))]);
    let comparisons = COMPARISONS.with(|count| count.get());
    assert_eq!(fresh.len(), 1);
    assert_eq!(fresh[0].1.0, 1 << 50);
    assert!(
        comparisons < 128,
        "payload hash membership must not scan 4096 rows"
    );
}

fn run_laws<P>()
where
    P: Build<std::vec::IntoIter<(i64, i64)>>
        + Fresh<Row = (i64, i64)>
        + Publish<Item = (i64, i64)>
        + ExactMembership<i64>,
    for<'a> &'a P: Rows<Row = (i64, i64)> + IndexRead<Key = i64, Item = i64>,
{
    maintenance_laws(
        |rows: Vec<(i64, i64)>| P::build(rows.into_iter()),
        |key, _, payload| (key, payload),
        |index| index.rows().collect(),
        |index, expected| {
            for key in probes() {
                let values: Vec<_> = expected
                    .iter()
                    .filter(|(stored, _)| *stored == key)
                    .map(|(_, value)| *value)
                    .collect();
                assert_eq!(ExactMembership::contains(index, &key), !values.is_empty());
                assert_eq!(sorted(index.lookup(&key).collect()), values);
            }
        },
    );
}

#[test]
fn sorted_run_binary_maintenance_laws() {
    run_laws::<SortedRun<i64, i64>>();
}

#[test]
fn sorted_run_rank_maintenance_laws() {
    run_laws::<SortedRun<i64, i64, cq_storage::I64Codec>>();
}

#[test]
fn radix_run_maintenance_laws() {
    run_laws::<RadixRun<i64, i64>>();
}

#[test]
fn columnar_radix_run_maintenance_laws() {
    type Columns = (Vec<i64>, Vec<i64>);
    type Index = RadixRun<i64, (i64, i64), Columns>;
    maintenance_laws(
        |rows| Index::build(rows.into_iter()),
        |key, first, second| (key, (first, second)),
        |index| index.rows().collect(),
        |index, expected| {
            for key in probes() {
                let values: Vec<_> = expected
                    .iter()
                    .filter_map(|&(k, value)| (k == key).then_some(value))
                    .collect();
                assert_eq!(sorted(index.lookup(&key).collect()), values);
                assert_eq!(index.count(&key), values.len());
                assert_eq!(index.contains(&key), !values.is_empty());
                let mut projected: Vec<_> = index
                    .column_groups(&key)
                    .flat_map(|(columns, range)| columns.0[range].iter().copied())
                    .collect();
                projected.sort_unstable();
                let mut expected_first: Vec<_> = values.iter().map(|value| value.0).collect();
                expected_first.sort_unstable();
                assert_eq!(projected, expected_first);
            }
        },
    );
}

type NestedRow = (i64, (i64, i64));
type NestedIndex = Trie<'static, i64, SortedRun<i64, i64>>;

fn trie_rows(index: &NestedIndex) -> Vec<NestedRow> {
    // Trie has no flat Rows adapter. Enumerate its entire generated outer-key
    // universe using IndexRead, then observe each inner provider through Rows.
    probes()
        .flat_map(|key| {
            index
                .lookup(&key)
                .flat_map(move |inner| inner.rows().map(move |rest| (key, rest)))
        })
        .collect()
}

fn check_trie_reads(index: &NestedIndex, expected: &BTreeSet<NestedRow>) {
    for key in probes() {
        let rows: Vec<_> = expected
            .iter()
            .filter(|(stored, _)| *stored == key)
            .map(|(_, rest)| *rest)
            .collect();
        assert_eq!(ExactMembership::contains(index, &key), !rows.is_empty());
        let children: Vec<_> = index.lookup(&key).collect();
        assert_eq!(children.len(), usize::from(!rows.is_empty()));
        for inner in children {
            for middle in -1..=5 {
                let values: Vec<_> = rows
                    .iter()
                    .filter(|(stored, _)| *stored == middle)
                    .map(|(_, value)| *value)
                    .collect();
                assert_eq!(
                    ExactMembership::contains(inner, &middle),
                    !values.is_empty()
                );
                assert_eq!(sorted(inner.lookup(&middle).collect()), values);
            }
        }
    }
}

#[test]
fn trie_maintenance_laws() {
    maintenance_laws(
        |rows: Vec<NestedRow>| NestedIndex::build(rows.into_iter()),
        |key, middle, payload| (key, (middle, payload)),
        trie_rows,
        check_trie_reads,
    );
}

#[test]
fn trie_maintenance_before_any_lazy_bucket_read() {
    let initial = [(0, (0, 0)), (0, (1, 0)), (1, (0, 0))];
    let mut index = NestedIndex::build(initial.into_iter());
    let candidates = vec![(0, (0, 0)), (0, (0, 1)), (0, (0, 1)), (63, (0, 0))];
    // Fresh reaches an existing lazy prefix and a missing prefix, but need not
    // touch prefix 1. Publish subsequently reaches that untouched lazy bucket.
    let fresh = index.fresh(candidates);
    assert_eq!(sorted(fresh.clone()), [(0, (0, 1)), (63, (0, 0))]);
    index.publish(fresh);
    index.publish([(1, (0, 1)), (64, (2, 3))]);
    let expected = initial
        .into_iter()
        .chain([(0, (0, 1)), (63, (0, 0)), (1, (0, 1)), (64, (2, 3))])
        .collect();
    assert_reads(&index, &expected, &trie_rows, &check_trie_reads);
}

#[test]
fn default_run_indices_start_empty_and_accept_first_publication() {
    fn check<P>()
    where
        P: Default + Fresh<Row = (i64, i64)> + Publish<Item = (i64, i64)>,
        for<'a> &'a P: Rows<Row = (i64, i64)>,
    {
        let mut index = P::default();
        assert_eq!((&index).rows().count(), 0);
        let fresh = index.fresh(vec![(1, 2), (1, 3), (1, 2)]);
        assert_eq!(sorted(fresh.clone()), [(1, 2), (1, 3)]);
        index.publish(fresh);
        assert_eq!(sorted((&index).rows().collect()), [(1, 2), (1, 3)]);
    }
    check::<SortedRun<i64, i64>>();
    check::<SortedRun<i64, i64, cq_storage::I64Codec>>();
    check::<RadixRun<i64, i64>>();

    let mut columns = RadixRun::<i64, (i64, i64), (Vec<i64>, Vec<i64>)>::default();
    assert!(columns.is_empty());
    columns.publish(columns.fresh(vec![(1, (2, 3)), (1, (2, 3))]));
    assert_eq!((&columns).rows().collect::<Vec<_>>(), [(1, (2, 3))]);

    let mut trie = NestedIndex::default();
    trie.publish(trie.fresh(vec![(1, (2, 3)), (1, (2, 3))]));
    assert_eq!(trie_rows(&trie), [(1, (2, 3))]);
}

#[test]
fn hash_admission_reuses_batches_without_cloning_rows() {
    #[derive(Eq, PartialEq, Hash)]
    struct NoClone(u32);
    let index = HashSet::from([NoClone(3)]);
    let batch = vec![NoClone(1), NoClone(1), NoClone(2), NoClone(3)];
    let allocation = batch.as_ptr();
    let fresh = index.fresh(batch);
    assert_eq!(fresh.as_ptr(), allocation);
    assert_eq!(
        fresh.into_iter().map(|row| row.0).collect::<BTreeSet<_>>(),
        BTreeSet::from([1, 2])
    );

    let index = HashMap::<_, _>::from_iter([(7, vec![NoClone(3)])]);
    let batch = vec![
        (7, NoClone(1)),
        (7, NoClone(1)),
        (7, NoClone(2)),
        (7, NoClone(3)),
    ];
    let allocation = batch.as_ptr();
    let fresh = index.fresh(batch);
    assert_eq!(fresh.as_ptr(), allocation);
    assert_eq!(
        fresh
            .into_iter()
            .map(|(key, row)| (key, row.0))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([(7, 1), (7, 2)])
    );
}
