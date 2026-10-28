//! Deterministic direct-vs-built evidence: scans initialize fallback indexes,
//! while native lookup/cursor calls identify direct reads.
use cq_storage::rows::Rows;
use cq_storage::{ExactMembership, IndexRead, OrderedRead};
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};

struct Observed<T: ?Sized> {
    scans: Cell<usize>,
    rows: Cell<usize>,
    reads: Cell<usize>,
    value: T,
}
impl<T> Observed<T> {
    fn new(value: T) -> Self {
        Self {
            scans: Cell::new(0),
            rows: Cell::new(0),
            reads: Cell::new(0),
            value,
        }
    }
}
impl<'a, T: ?Sized> Rows for &'a Observed<T>
where
    &'a T: Rows,
{
    type Row = <&'a T as Rows>::Row;
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        self.scans.set(self.scans.get() + 1);
        (&self.value)
            .rows()
            .inspect(|_| self.rows.set(self.rows.get() + 1))
    }
}
impl<'a, T: ?Sized> IndexRead for &'a Observed<T>
where
    &'a T: IndexRead,
{
    type Key = <&'a T as IndexRead>::Key;
    type Item = <&'a T as IndexRead>::Item;
    type Cursor = <&'a T as IndexRead>::Cursor;
    fn lookup(&self, key: &Self::Key) -> Self::Cursor {
        self.reads.set(self.reads.get() + 1);
        (&self.value).lookup(key)
    }
}
impl<'a, T: ?Sized> cq_storage::PointLookup for &'a Observed<T>
where
    &'a T: cq_storage::PointLookup,
{
    type Key = <&'a T as cq_storage::PointLookup>::Key;
    type Row = <&'a T as cq_storage::PointLookup>::Row;
    type Cursor = <&'a T as cq_storage::PointLookup>::Cursor;
    fn point_lookup(&self, key: &Self::Key) -> Self::Cursor {
        self.reads.set(self.reads.get() + 1);
        (&self.value).point_lookup(key)
    }
}
impl<T: ExactMembership<K>, K> ExactMembership<K> for Observed<T> {
    fn contains(&self, key: &K) -> bool {
        self.reads.set(self.reads.get() + 1);
        self.value.contains(key)
    }
}
impl<'h> OrderedRead for &'h Observed<BTreeMap<(i32,), Vec<(i32,)>>> {
    type Cursor<'a>
        = cq_storage::GroupedCursor<'h, (i32,), (i32,)>
    where
        Self: 'a;
    fn ordered(&self) -> Self::Cursor<'_> {
        self.reads.set(self.reads.get() + 1);
        (&self.value).ordered()
    }
}
// These cost contracts isolate unreduced probe dispatch, not reduction materialization.
macro_rules! probe {
    ($l:ident,$r:ident) => {
        ::mini_linq::compile! { pipeline { plan = ::mini_linq::passes::plan(no_reduce); structures = [::core_pass_fixtures::ordered_dedup]; }
            struct Q; relation $l(a:i32); relation $r(a:i32,b:i32);
            answer(a,b) :- $l(a),$r(a,b);
        }
    };
}
macro_rules! nonleading {
    ($l:ident,$r:ident) => {
        ::mini_linq::compile! { pipeline { plan = ::mini_linq::passes::plan(no_reduce); structures = [::core_pass_fixtures::ordered_dedup]; }
            struct Q; relation $l(b:i32); relation $r(a:i32,b:i32);
            answer(a,b) :- $l(b),$r(a,b);
        }
    };
}
macro_rules! window {
    ($r:ident) => {
        ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; }
            struct Q; relation $r(a:i32,b:i32);
            answer(a,b) :- $r(a,b), if *a >= 2 && *a < 9;
        }
    };
}

#[test]
fn leading_hash_reads_build_zero_indexes_and_nonleading_reads_build_once() {
    let l = [(1,), (1,), (2,), (9,)];
    let rows = [(1, 7), (1, 8), (2, 7)];
    let r = Observed::new(HashMap::from([
        ((1,), vec![(7,), (8,)]),
        ((2,), vec![(7,)]),
    ]));
    let got: Vec<_> = probe!(l, r).collect();
    assert_eq!(got.len(), 3);
    assert_eq!((r.scans.get(), r.reads.get()), (0, l.len()));
    let r = Observed::new(rows.as_slice());
    let got: Vec<_> = probe!(l, r).collect();
    assert_eq!(got.len(), 3);
    assert_eq!((r.scans.get(), r.reads.get()), (1, 0));
    let l = [(7,), (7,), (8,)];
    let r = Observed::new(HashMap::from([
        ((1,), vec![(7,), (8,)]),
        ((2,), vec![(7,)]),
    ]));
    let got: Vec<_> = nonleading!(l, r).collect();
    assert_eq!(got.len(), 3);
    assert_eq!((r.scans.get(), r.reads.get()), (1, 0));
    println!(
        "hash leading: builds=0 lookups=4; slice: builds=1; nonleading same-type map: builds=1 direct-lookups=0"
    );
}

#[test]
fn fallback_is_lazy_and_shared_for_membership() {
    let l: [(i32,); 0] = [];
    let r = Observed::new([(1, 7)]);
    assert_eq!(probe!(l, r).count(), 0);
    assert_eq!(r.scans.get(), 0);
    let l = [(1,), (1,), (2,)];
    let r = Observed::new(HashMap::from([
        ((1,), vec![(7,), (8,)]),
        ((2,), vec![(7,)]),
    ]));
    // Isolate membership dispatch from default semijoin materialization.
    let got: Vec<_> = ::mini_linq::compile! { pipeline { plan = ::mini_linq::passes::plan(no_reduce); structures = [::core_pass_fixtures::ordered_dedup]; } struct Q; relation l(a:i32); relation r(a:i32,b:i32); answer(a) :- l(a),r(a,_); }.collect();
    assert_eq!(got.len(), 2);
    assert_eq!(r.scans.get(), 0);
    assert_eq!(r.reads.get(), 3);
}

#[test]
fn negation_reuses_native_owned_and_borrowed_key_sets_without_builds() {
    macro_rules! negative {
        ($left:ident, $keys:ident) => {
            ::mini_linq::compile! {
                pipeline { plan = ::mini_linq::passes::plan(no_reduce); }
                struct Q;
                relation $left(a: i32);
                relation $keys(a: i32);
                answer(a) :- $left(a), !$keys(a);
            }
        };
    }
    let left = [(1,), (2,), (3,), (4,)];
    let keys = Observed::new(std::collections::HashSet::from([(1,), (3,)]));
    for _ in 0..3 {
        assert_eq!(
            negative!(left, keys).map(|(a,)| *a).collect::<Vec<_>>(),
            [2, 4]
        );
    }
    assert_eq!((keys.scans.get(), keys.reads.get()), (0, 12));

    let values = [1, 3];
    let keys = Observed::new(std::collections::HashSet::from([
        (&values[0],),
        (&values[1],),
    ]));
    for _ in 0..3 {
        assert_eq!(
            negative!(left, keys).map(|(a,)| *a).collect::<Vec<_>>(),
            [2, 4]
        );
    }
    assert_eq!((keys.scans.get(), keys.reads.get()), (0, 12));

    let keys = Observed::new([(1,), (3,)]);
    assert_eq!(
        negative!(left, keys).map(|(a,)| *a).collect::<Vec<_>>(),
        [2, 4]
    );
    assert_eq!((keys.scans.get(), keys.reads.get()), (1, 0));
}

#[test]
fn lazy_negation_uses_native_membership_and_builds_scan_fallback_once() {
    macro_rules! negative {
        ($left:ident, $keys:ident) => {
            ::mini_linq::compile! {
                pipeline {
                    plan = ::mini_linq::passes::plan(no_reduce);
                    physical = [::core_pass_fixtures::lazy_anti];
                }
                struct Q;
                relation $left(a: i32);
                relation $keys(a: i32);
                answer(a) :- $left(a), !$keys(a);
            }
        };
    }
    let left = [(1,), (2,), (3,), (4,)];
    let values = [1, 3];
    let keys = Observed::new(std::collections::HashSet::from([
        (&values[0],),
        (&values[1],),
    ]));
    for _ in 0..3 {
        assert_eq!(
            negative!(left, keys).map(|(a,)| *a).collect::<Vec<_>>(),
            [2, 4]
        );
    }
    assert_eq!((keys.scans.get(), keys.reads.get()), (0, 12));
    let keys = Observed::new([(1,), (3,)]);
    assert_eq!(
        negative!(left, keys).map(|(a,)| *a).collect::<Vec<_>>(),
        [2, 4]
    );
    assert_eq!((keys.scans.get(), keys.reads.get()), (1, 0));
}

#[test]
fn computed_negation_keys_still_probe_owned_and_borrowed_hosts_in_place() {
    macro_rules! negative {
        ($left:ident, $keys:ident) => {
            ::mini_linq::compile! {
                pipeline { plan = ::mini_linq::passes::plan(no_reduce); }
                struct Q;
                relation $left(a: i32);
                relation $keys(y: String);
                answer(a) :- $left(a), let y: String = a.to_string(), !$keys(y);
            }
        };
    }
    let left = [(1,), (2,), (3,)];
    let values = [String::from("1"), String::from("3")];
    let keys = Observed::new(std::collections::HashSet::from([
        (values[0].clone(),),
        (values[1].clone(),),
    ]));
    cq_storage::counters::reset();
    assert_eq!(
        negative!(left, keys).map(|(a,)| *a).collect::<Vec<_>>(),
        [2]
    );
    assert_eq!((keys.scans.get(), keys.reads.get()), (0, 3));
    assert_eq!(cq_storage::counters::snapshot().index_builds, 0);
    let keys = Observed::new(std::collections::HashSet::from([
        (&values[0],),
        (&values[1],),
    ]));
    cq_storage::counters::reset();
    assert_eq!(
        negative!(left, keys).map(|(a,)| *a).collect::<Vec<_>>(),
        [2]
    );
    assert_eq!((keys.scans.get(), keys.reads.get()), (0, 3));
    assert_eq!(cq_storage::counters::snapshot().index_builds, 0);
    let keys = Observed::new([(String::from("1"),), (String::from("3"),)]);
    cq_storage::counters::reset();
    assert_eq!(
        negative!(left, keys).map(|(a,)| *a).collect::<Vec<_>>(),
        [2]
    );
    assert_eq!((keys.scans.get(), keys.reads.get()), (1, 0));
    assert_eq!(cq_storage::counters::snapshot().index_builds, 1);
}

#[test]
fn generator_negation_keys_use_native_borrowed_membership_without_builds() {
    let left = [(1,), (2,), (3,)];
    let values = [String::from("1"), String::from("3")];
    let keys = Observed::new(std::collections::HashSet::from([
        (&values[0],),
        (&values[1],),
    ]));
    cq_storage::counters::reset();
    let answers: Vec<_> = ::mini_linq::compile! {
        pipeline { plan = ::mini_linq::passes::plan(no_reduce); }
        struct Q;
        relation left(a:i32); relation keys(y:String);
        answer(a) :- left(a), for y in Some(a.to_string()), !keys(y);
    }
    .map(|(a,)| *a)
    .collect();
    assert_eq!(answers, [2]);
    assert_eq!((keys.scans.get(), keys.reads.get()), (0, 3));
    let cost = cq_storage::counters::snapshot();
    assert_eq!(cost.index_builds, 0);
    assert_eq!(cost.lookups, 3);
}

#[test]
fn first_host_answer_streams_and_owns_unused_dispatch_state() {
    let values: Vec<_> = (0..100_000).map(|a| (a,)).collect();
    let l = Observed::new(values.as_slice());
    let r = Observed::new(
        (0..100_000)
            .map(|a| ((a,), vec![(a,)]))
            .collect::<HashMap<_, _>>(),
    );
    let mut answers = probe!(l, r);
    assert_eq!((l.rows.get(), r.scans.get(), r.reads.get()), (0, 0, 0));
    assert_eq!(answers.next(), Some((&0, &0)));
    assert_eq!((l.rows.get(), r.scans.get(), r.reads.get()), (1, 0, 1));
    assert!(l.rows.get() * 1000 < values.len());
    println!("host first answer: rows=1, lookups=1, builds=0; full result=100000");
}

#[test]
fn first_answer_owns_and_initializes_the_fallback_only_on_demand() {
    let values: Vec<_> = (0..20_000).map(|v| (0, v)).collect();
    let l = Observed::new(values.as_slice());
    let r = Observed::new(values.as_slice());
    let visits = Cell::new(0);
    let observe = |_: &i32, _: &i32| {
        visits.set(visits.get() + 1);
        true
    };
    // This test pins lazy fallback initialization in the unreduced probe plan.
    let mut answers = ::mini_linq::compile! { pipeline { plan = ::mini_linq::passes::plan(no_reduce); structures = [::core_pass_fixtures::ordered_dedup]; }
        struct Q;
        relation l(a:i32,tag:i32); relation r(a:i32,b:i32);
        answer(tag,b) :- l(a,tag),r(a,b),if observe(tag,b);
    };
    assert_eq!((l.rows.get(), r.rows.get(), visits.get()), (0, 0, 0));
    assert_eq!(answers.next(), Some((&0, &0)));
    assert_eq!((l.rows.get(), r.rows.get(), visits.get()), (1, 20_000, 1));
    assert_eq!(answers.next(), Some((&0, &1)));
    assert_eq!((r.scans.get(), r.rows.get(), visits.get()), (1, 20_000, 2));
    println!(
        "lazy fallback first answer: outer rows=1, build rows=20000, answers visited=1; full result=400000000"
    );
}

#[test]
fn relational_range_iterators_survive_the_generated_block() {
    let rows: Vec<_> = (0..100_000).map(|a| (a, a)).collect();
    let r = Observed::new(
        rows.iter()
            .map(|&(a, b)| ((a,), vec![(b,)]))
            .collect::<BTreeMap<_, _>>(),
    );
    let mut answers = window!(r);
    assert_eq!((r.scans.get(), r.reads.get()), (0, 1));
    assert_eq!(answers.next(), Some((&2, &2)));
    assert_eq!((r.scans.get(), r.reads.get()), (0, 1));
    let r = Observed::new(rows.as_slice());
    let mut answers = window!(r);
    assert_eq!(r.rows.get(), 0);
    assert_eq!(answers.next(), Some((&2, &2)));
    assert_eq!(answers.next(), Some((&3, &3)));
    assert_eq!(r.scans.get(), 1);
}

#[test]
fn rust_comparison_windows_seek_ordered_hosts_and_scan_slices() {
    let r = Observed::new(BTreeMap::from([
        ((1,), vec![(0,)]),
        ((2,), vec![(5,), (6,)]),
        ((8,), vec![(1,)]),
        ((9,), vec![(1,)]),
    ]));
    let got: Vec<_> = window!(r).collect();
    assert_eq!(got.len(), 3);
    assert_eq!((r.scans.get(), r.reads.get()), (0, 1));
    let rows = [(1, 0), (2, 5), (2, 6), (8, 1), (9, 1)];
    let r = Observed::new(rows.as_slice());
    let got: Vec<_> = window!(r).collect();
    assert_eq!(got.len(), 3);
    assert_eq!((r.scans.get(), r.reads.get()), (1, 0));
    println!("relational windows: maps seek and slices scan; no indexes built");
}

#[test]
fn a_different_prefix_length_falls_back_even_when_the_host_has_a_key() {
    let l = [(1, 7)];
    let r = Observed::new(HashMap::from([((1,), vec![(7,), (8,)])]));
    let got: Vec<_> = ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; } struct Q; relation l(a:i32,b:i32); relation r(a:i32,b:i32); answer(a,b) :- l(a,b),r(a,b); }.collect();
    assert_eq!(got.len(), 1);
    assert_eq!((r.scans.get(), r.reads.get()), (1, 0));
}

#[test]
fn ordered_key_types_need_no_hash_implementation_on_direct_reads() {
    #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
    struct Key(i32);
    let l = [(Key(1),), (Key(2),)];
    let r = BTreeMap::from([((Key(1),), vec![(7,)]), ((Key(2),), Vec::<(i32,)>::new())]);
    let got: Vec<_> = ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; } struct Q; relation l(a:Key); relation r(a:Key,b:i32); answer(a,b) :- l(a),r(a,b); }.collect();
    assert_eq!(got.len(), 1);
    let got: Vec<_> = ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; } struct Q; relation l(a:Key); relation r(a:Key,b:i32); answer(a) :- l(a),r(a,_); }.collect();
    assert_eq!(got.len(), 1, "an empty map group has no witness");
}

#[test]
fn relational_comparisons_follow_the_declared_numeric_type() {
    let r = BTreeMap::from([
        ((1_u64,), vec![(0,)]),
        ((2,), vec![(5,)]),
        ((9,), vec![(1,)]),
    ]);
    let got: Vec<_> = ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; } struct Q; relation r(a:u64,b:i32); answer(a,b) :- r(a,b),if *a < 9; }.collect();
    assert_eq!(got.len(), 2);
    let r = [(1_u64, 0), (2, 5), (9, 1)];
    let got: Vec<_> = ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; } struct Q; relation r(a:u64,b:i32); answer(a,b) :- r(a,b),if *a >= 2 && *a < 9; }.collect();
    assert_eq!(got, [(&2, &5)]);
}

#[test]
fn ordinary_hashmap_counters_separate_native_lookups_from_build_hashes() {
    use std::hash::{Hash, Hasher};
    thread_local! { static HASHES: Cell<(usize,usize)> = const { Cell::new((0,0)) }; }
    #[derive(Clone)]
    struct Key(i32, bool);
    impl PartialEq for Key {
        fn eq(&self, other: &Self) -> bool {
            self.0 == other.0
        }
    }
    impl Eq for Key {}
    impl PartialOrd for Key {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for Key {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.0.cmp(&other.0)
        }
    }
    impl Hash for Key {
        fn hash<H: Hasher>(&self, h: &mut H) {
            HASHES.with(|c| {
                let (build, lookup) = c.get();
                c.set((build + usize::from(self.1), lookup + usize::from(!self.1)));
            });
            self.0.hash(h);
        }
    }
    macro_rules! run { ($l:ident,$r:ident) => { ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; } struct Q; relation $l(a:Key); relation $r(a:Key,b:i32); answer(a,b) :- $l(a),$r(a,b); } }; }
    let l = [(Key(1, false),), (Key(1, false),), (Key(2, false),)];
    let r = HashMap::from([((Key(1, true),), vec![(7,)])]);
    HASHES.with(|h| h.set((0, 0)));
    assert_eq!(run!(l, r).count(), 1);
    // Three payload probes; unfiltered membership reducers are omitted.
    assert_eq!(HASHES.with(Cell::get), (0, 3));
    let rows = [(Key(1, true), 7)];
    let r = rows.as_slice();
    HASHES.with(|h| h.set((0, 0)));
    assert_eq!(run!(l, r).count(), 1);
    // The source row enters one bucket map; publishing it does not rehash keys.
    // The fallback supplies payload probes without a redundant membership pass.
    assert_eq!(HASHES.with(Cell::get), (1, 3));
}

#[test]
fn composite_prefix_uses_declaration_order_and_shorter_keys_build() {
    let l = [(1, 2)];
    let r = Observed::new(HashMap::from([((2, 1), vec![(3,)])]));
    let got: Vec<_> = ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; } struct Q; relation l(a:i32,b:i32); relation r(b:i32,a:i32,c:i32); answer(a,b,c) :- l(a,b),r(b,a,c); }.collect();
    assert_eq!(got, [(&1, &2, &3)]);
    // One native payload probe uses the full declared key.
    assert_eq!((r.scans.get(), r.reads.get()), (0, 1));
    let l = [(2,)];
    let got: Vec<_> = ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; } struct Q; relation l(b:i32); relation r(b:i32,a:i32,c:i32); answer(a,b,c) :- l(b),r(b,a,c); }.collect();
    assert_eq!(got, [(&1, &2, &3)]);
    // The shorter key scans once; prior full-key native reads remain counted.
    assert_eq!((r.scans.get(), r.reads.get()), (1, 1));
}

#[test]
fn direct_merge_requires_ordered_read_without_requiring_payload_lookup() {
    let l = std::collections::BTreeSet::from([(1,), (2,)]);
    let r = std::collections::BTreeSet::from([(2,), (3,)]);
    let got: Vec<_> = ::mini_linq::passes::select! { pipeline { ::core_pass_fixtures::ordered_dedup, ::mini_linq::passes::sequence, ::mini_linq::passes::sequential, ::mini_linq::passes::translate_pull };
        physical {
            relation l(a:i32); relation r(a:i32);
            left = pipe rename l@0 {a -> a}; right = pipe rename r@1 {a -> a};
            j = pipe canon merge[a] left,right; return j as answer(a);
        }
    }
    .collect();
    assert_eq!(got, [(&2,)]);
}

#[test]
fn ordered_key_sets_support_relational_comparison_windows() {
    let r = std::collections::BTreeSet::from([(1,), (2,), (9,)]);
    let got: Vec<_> = ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; } struct Q; relation r(a:i32); answer(a) :- r(a), if *a >= 2 && *a < 9; }.collect();
    assert_eq!(got, [(&2,)]);
}

#[test]
fn retained_opaque_guards_preserve_unsigned_comparisons() {
    let r = [(1_u64, 0), (2, 5), (9, 1)];
    let got: Vec<_> = ::mini_linq::passes::select! { pipeline { ::core_pass_fixtures::ordered_dedup, ::mini_linq::passes::sequence, ::mini_linq::passes::sequential, ::mini_linq::passes::translate_pull };
        physical {
            relation r(a:u64,b:i32); rows = let rename r@0 {a -> a,b -> b};
            w = pipe canon guard rows if (*a < 9); return w as answer(a,b);
        }
    }
    .collect();
    assert_eq!(got, [(&1, &0), (&2, &5)]);
}

#[test]
#[allow(non_snake_case)]
fn literal_point_reads_scan_slices_and_directly_read_leading_map_keys() {
    let Customer = Observed::new(vec![
        (1_i32, "BUILDING"),
        (2, "AUTOMOBILE"),
        (3, "BUILDING"),
    ]);
    cq_storage::counters::reset();
    let rows: std::collections::BTreeSet<_> =
        ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; }
            struct LiteralSlice;
            relation Customer(id:i32, segment:&str);
            answer(c) :- Customer(c, "BUILDING");
        }
        .map(|(c,)| *c)
        .collect();
    assert_eq!(rows, [1, 3].into());
    assert_eq!(cq_storage::counters::snapshot().index_builds, 0);
    assert_eq!(Customer.scans.get(), 1);
    assert_eq!(Customer.reads.get(), 0);
    assert_eq!(Customer.rows.get(), 3);

    let Customer = Observed::new(HashMap::from([
        (("BUILDING",), vec![(1_i32,), (3,)]),
        (("AUTOMOBILE",), vec![(2,)]),
    ]));
    cq_storage::counters::reset();
    let rows: std::collections::BTreeSet<_> =
        ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; }
            struct LiteralMap;
            relation Customer(segment:&str, id:i32);
            answer(c) :- Customer("BUILDING", c);
        }
        .map(|(c,)| *c)
        .collect();
    assert_eq!(rows, [1, 3].into());
    assert_eq!(cq_storage::counters::snapshot().index_builds, 0);
    assert_eq!(Customer.scans.get(), 0);
    assert_eq!(Customer.reads.get(), 1);
}

#[test]
fn relational_range_reads_seek_ordered_hosts_and_scan_slices_without_builds() {
    let map: BTreeMap<(i32,), Vec<(i32,)>> = (0..10_000).map(|x| ((x,), vec![(x + 1,)])).collect();
    let low = 9_995;
    let high = 9_998;
    cq_storage::counters::reset();
    let rows: Vec<_> =
        ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; }
            struct Range; relation map(key:i32,value:i32);
            answer(x,y) :- map(x,y), if * x >= low, if * x < high;
        }
        .map(|(x, y)| (*x, *y))
        .collect();
    assert_eq!(rows, [(9_995, 9_996), (9_996, 9_997), (9_997, 9_998)]);
    let counts = cq_storage::counters::snapshot();
    assert_eq!(counts.index_builds, 0);
    assert!(
        counts.rows_enumerated < 10 && counts.cursor_steps < 12,
        "{counts:?}"
    );

    let slice: Vec<_> = (0..10_000).map(|x| (x, x + 1)).collect();
    cq_storage::counters::reset();
    let scanned: Vec<_> =
        ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; }
            struct SliceRange; relation slice(key:i32,value:i32);
            answer(x,y) :- slice(x,y), if * x >= low, if * x < high;
        }
        .map(|(x, y)| (*x, *y))
        .collect();
    assert_eq!(scanned, rows);
    let counts = cq_storage::counters::snapshot();
    assert_eq!(counts.index_builds, 0);
    assert!(counts.rows_enumerated >= 10_000, "{counts:?}");

    // Empty windows are total, including the bounds that BTreeMap::range rejects.
    for (low, high) in [(5, 5), (9, 2)] {
        let rows: Vec<_> = ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; }
            struct EmptyRange; relation map(key:i32,value:i32);
            answer(x) :- map(x,_), if * x > low, if * x < high;
        }
        .collect();
        assert!(rows.is_empty());
    }
}

#[test]
fn relational_disjunction_membership_and_host_parameters_compute_classical_filters() {
    let rows = [
        (0, "a".to_owned()),
        (1, "b".into()),
        (2, "a".into()),
        (3, "c".into()),
        (4, "b".into()),
    ];
    let low = 2;
    let got: Vec<_> = ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; }
        struct Disjunction; relation rows(key:i32,tag:String);
        answer(x) :- rows(x,tag),
            if ((((* x >= 0 && * x <= 1)) && ((* tag == "a" || * tag == "b"))) || ((* x >= low) && (* tag == "c")));
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(got, [0, 1, 3]);
    let tag_param = "b".to_owned();
    let got: Vec<_> =
        ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; }
            struct Parameters; relation rows(key:i32,tag:String);
            answer(x) :- rows(x,tag), if * tag == tag_param, if 1 < * x, if * x <= 4;
        }
        .map(|(x,)| *x)
        .collect();
    assert_eq!(got, [4]);
    assert_eq!(tag_param, "b");
}

#[test]
fn an_ordered_host_and_non_copy_parameter_need_no_cloning_or_scan_fallback() {
    let r = Observed::new(BTreeMap::from([
        ((1,), vec![(11,)]),
        ((2,), vec![(22,)]),
        ((3,), vec![(33,)]),
    ]));
    let cutoff = 2;
    let got: Vec<_> =
        ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; }
            struct OrderedHost; relation r(k:i32,v:i32);
            answer(x,y) :- r(x,y), if * x >= cutoff;
        }
        .map(|(x, y)| (*x, *y))
        .collect();
    assert_eq!(got, [(2, 22), (3, 33)]);
    assert_eq!(r.scans.get(), 0);
    assert_eq!(r.reads.get(), 1);

    // Neither Eq/Ord nor borrowed captures imply Clone. A panicking Clone
    // implementation must never be called by a relational constant window.
    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct Value(i32);
    impl Clone for Value {
        fn clone(&self) -> Self {
            panic!("opaque Clone is not relational")
        }
    }
    let map = BTreeMap::from([((Value(1),), vec![(10,)]), ((Value(3),), vec![(30,)])]);
    let cutoff = Value(2);
    let got: Vec<_> =
        ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; }
            struct BorrowedBounds; relation map(k:Value,v:i32);
            answer(y) :- map(x,y), if * x > cutoff;
        }
        .map(|(y,)| *y)
        .collect();
    assert_eq!(got, [30]);
}

#[test]
fn constraints_on_prefix_values_precede_partial_guards_and_later_atoms() {
    let r = [(0,), (2,)];
    let s = [(2, 20)];
    let cutoff = 1;
    let got: Vec<_> =
        ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; }
            struct LetPrefix; relation r(x:i32); relation s(m:i32,y:i32);
            answer(y) :- r(x), let m:i32 = *x, if * m > cutoff, if 10 / *m > 0, s(m,y);
        }
        .map(|(y,)| *y)
        .collect();
    assert_eq!(got, [20]);
    let other = [(0,)];
    let t = [(2, 0, 99)];
    let got: Vec<_> = ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; }
        struct PrefixTuple; relation r(x:i32); relation other(y:i32); relation t(x:i32,y:i32,z:i32);
        answer(z) :- r(x), other(y), if * x > * y, if 10 / (*x - *y) > 0, t(x,y,z);
    }
    .map(|(z,)| *z)
    .collect();
    assert_eq!(got, [99]);
    let got: Vec<_> =
        ::mini_linq::compile! { pipeline { structures = [::core_pass_fixtures::ordered_dedup]; }
            struct BoundIdentity; relation r(x:i32); relation s(m:i32,y:i32);
            answer(y) :- r(x), let m:i32 = 2, if * m == * x, s(m,y);
        }
        .map(|(y,)| *y)
        .collect();
    assert_eq!(got, [20]);
}

/// An explicitly source-bound, non-leading index. Payloads borrow the original
/// rows rather than a reconstructed tuple buffer.
struct IndexedRows {
    source: Vec<(i32, i32, i32)>,
    index: cq_storage::RadixRun<(i32,), usize>,
    scans: Cell<usize>,
    reads: Cell<usize>,
}

impl IndexedRows {
    fn new(source: Vec<(i32, i32, i32)>) -> Self {
        let index = cq_storage::RadixRun::from_pairs(
            source
                .iter()
                .enumerate()
                .map(|(row, value)| ((value.1,), row)),
        );
        Self {
            source,
            index,
            scans: Cell::new(0),
            reads: Cell::new(0),
        }
    }
}

impl<'a> Rows for &'a IndexedRows {
    type Row = (&'a i32, &'a i32, &'a i32);

    fn rows(self) -> impl Iterator<Item = Self::Row> {
        self.scans.set(self.scans.get() + 1);
        self.source.iter().map(|(a, b, c)| (a, b, c))
    }
}

#[derive(Clone, Copy)]
struct IndexedPayload<'a>(&'a (i32, i32, i32));

impl<'a> cq_storage::rows::Columns for IndexedPayload<'a> {
    type Row = (&'a i32, &'a i32);

    fn columns(self) -> Self::Row {
        (&self.0.0, &self.0.2)
    }
}

#[derive(Clone, Copy)]
struct IndexedCursor<'a> {
    source: &'a [(i32, i32, i32)],
    rows: cq_storage::RunCursor<'a, usize>,
}

impl<'a> Iterator for IndexedCursor<'a> {
    type Item = IndexedPayload<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.rows
            .next()
            .map(|row| IndexedPayload(&self.source[row]))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.rows.size_hint()
    }
}

impl ExactSizeIterator for IndexedCursor<'_> {}

#[derive(Clone, Copy)]
struct IndexedRead<'a>(&'a IndexedRows);

impl<'a> IndexRead for IndexedRead<'a> {
    type Key = (i32,);
    type Item = IndexedPayload<'a>;
    type Cursor = IndexedCursor<'a>;

    fn lookup(&self, key: &Self::Key) -> Self::Cursor {
        self.0.reads.set(self.0.reads.get() + 1);
        IndexedCursor {
            source: &self.0.source,
            rows: (&self.0.index).lookup(key),
        }
    }
}

impl ExactMembership<(i32,)> for IndexedRead<'_> {
    fn contains(&self, key: &(i32,)) -> bool {
        self.0.reads.set(self.0.reads.get() + 1);
        self.0.index.contains(key)
    }
}

impl<'a> cq_storage::dispatch::IndexedColumn<1> for &'a IndexedRows {
    type Read = IndexedRead<'a>;

    fn indexed_column(&self) -> Self::Read {
        IndexedRead(self)
    }
}

#[test]
fn compiled_declared_column_reads_native_source_rows_without_fallback() {
    let left = [(7,), (99,), (8,), (7,)];
    let owner = IndexedRows::new(vec![(10, 7, 100), (20, 8, 200), (11, 7, 101), (10, 7, 100)]);
    // Exercise the extra borrowed binding used by generated workload hosts.
    let right = &owner;
    let visits = &std::cell::RefCell::new(Vec::new());
    let result: Vec<_> = ::mini_linq::compile! {
        pipeline { plan = ::mini_linq::passes::plan(no_reduce); structures = [::core_pass_fixtures::ordered_dedup]; }
        struct ColumnProbe;
        relation left(q:i32); relation right(a:i32,b:i32,c:i32);
        answer(x,k,z) :- left(k), right(x,k,z),
            if { visits.borrow_mut().push((*x,*k,*z)); true };
    }.map(|(x,k,z)| (*x,*k,*z)).collect();
    assert_eq!(result, [(10, 7, 100), (11, 7, 101), (20, 8, 200)]);
    assert_eq!(
        *visits.borrow(),
        [
            (10, 7, 100),
            (11, 7, 101),
            (10, 7, 100),
            (20, 8, 200),
            (10, 7, 100),
            (11, 7, 101),
            (10, 7, 100),
        ]
    );
    assert_eq!((owner.scans.get(), owner.reads.get()), (0, left.len()));
}

#[test]
fn compiled_same_typed_wrong_slot_and_computed_keys_keep_fallback() {
    let left = [(100,), (999,), (200,)];
    let right = IndexedRows::new(vec![(10, 7, 100), (20, 8, 200), (11, 7, 101), (10, 7, 100)]);
    let result: Vec<_> = ::mini_linq::compile! {
        pipeline { plan = ::mini_linq::passes::plan(no_reduce); structures = [::core_pass_fixtures::ordered_dedup]; }
        struct WrongSlot;
        relation left(q:i32); relation right(a:i32,b:i32,c:i32);
        answer(x,y,k) :- left(k), right(x,y,k);
    }.map(|(x,y,k)| (*x,*y,*k)).collect();
    assert_eq!(result, [(10, 7, 100), (20, 8, 200)]);
    assert_eq!((right.scans.get(), right.reads.get()), (1, 0));

    let left = [(7,), (99,), (8,)];
    let right = IndexedRows::new(vec![(10, 7, 100), (20, 8, 200)]);
    let result: Vec<_> = ::mini_linq::compile! {
        pipeline { plan = ::mini_linq::passes::plan(no_reduce); structures = [::core_pass_fixtures::ordered_dedup]; }
        struct ComputedColumn;
        relation left(q:i32); relation right(a:i32,b:i32,c:i32);
        answer(x,k,z) :- left(q), let k:i32 = *q, right(x,k,z);
    }.map(|(x,k,z)| (*x,k,*z)).collect();
    assert_eq!(result, [(10, 7, 100), (20, 8, 200)]);
    assert_eq!((right.scans.get(), right.reads.get()), (1, 0));
}

#[test]
fn compiled_nonprefix_multikey_does_not_use_a_single_column_index() {
    let left = [(7, 100), (7, 200), (8, 200)];
    let right = IndexedRows::new(vec![(10, 7, 100), (20, 8, 200), (11, 7, 101)]);
    let result: Vec<_> = ::mini_linq::compile! {
        pipeline { plan = ::mini_linq::passes::plan(no_reduce); structures = [::core_pass_fixtures::ordered_dedup]; }
        struct MultiColumn;
        relation left(q:i32,r:i32); relation right(a:i32,b:i32,c:i32);
        answer(x,k,z) :- left(k,z), right(x,k,z);
    }.map(|(x,k,z)| (*x,*k,*z)).collect();
    assert_eq!(result, [(10, 7, 100), (20, 8, 200)]);
    assert_eq!((right.scans.get(), right.reads.get()), (1, 0));
}

#[test]
fn compiled_explicit_column_membership_uses_only_the_matching_slot() {
    let left = [(7,), (99,), (8,)];
    let right = IndexedRows::new(vec![(10, 7, 100), (20, 8, 200), (11, 7, 101)]);
    let result: Vec<_> = ::mini_linq::compile! {
        pipeline { plan = ::mini_linq::passes::plan(no_reduce); structures = [::core_pass_fixtures::ordered_dedup]; }
        struct ColumnMembership;
        relation left(q:i32); relation right(a:i32,b:i32,c:i32);
        answer(k) :- left(k), !right(_,k,_);
    }.map(|(k,)| *k).collect();
    assert_eq!(result, [99]);
    assert_eq!((right.scans.get(), right.reads.get()), (0, left.len()));
}

#[test]
fn compiled_retained_column_cursor_expands_once_in_source_chronology() {
    let left = [(7,), (99,), (8,), (7,)];
    let right = IndexedRows::new(vec![(10, 7, 100), (20, 8, 200), (11, 7, 101), (10, 7, 100)]);
    let visits = &std::cell::RefCell::new(Vec::new());
    let result: Vec<_> = ::mini_linq::passes::sequence! {
        pipeline { ::mini_linq::passes::sequential, ::mini_linq::passes::translate_pull };
        physical {
            relation L(q:i32) = (&left); relation R(a:i32,b:i32,c:i32) = (&right);
            l = pipe rename L@0 {q -> k};
            r = pipe rename R@1 {a -> x,b -> k,c -> z};
            s = pipe semi eager [k] l,r;
            p = pipe probe first [k] s,r;
            observed = pipe guard p if ({ visits.borrow_mut().push((*x,*k,*z)); true });
            out = pipe canon project observed keep {x,k,z};
            return out as answer(x,k,z);
        }
        structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            r host s = ::cq_storage::RadixRun<_,_> [k] supports [expand];
            r host p = ::cq_storage::RadixRun<_,_> [k] supports [expand];
            s share = lookup r [k] first s force p;
            s repr = tuples witness { r [k] at s as _ };
            p stage = batch 2 expand;
            out canon = ::std::collections::HashSet<_>;
        }
    }
    .map(|(x, k, z)| (*x, *k, *z))
    .collect();
    assert_eq!(result, [(10, 7, 100), (11, 7, 101), (20, 8, 200)]);
    assert_eq!(
        *visits.borrow(),
        [
            (10, 7, 100),
            (11, 7, 101),
            (10, 7, 100),
            (20, 8, 200),
            (10, 7, 100),
            (11, 7, 101),
            (10, 7, 100),
        ]
    );
    assert_eq!((right.scans.get(), right.reads.get()), (0, left.len()));
}
