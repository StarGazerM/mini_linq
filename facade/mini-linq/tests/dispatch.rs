//! Deterministic direct-vs-built evidence: scans initialize fallback indexes,
//! while native lookup/cursor calls identify direct reads.
use cq_storage::rows::Rows;
use cq_storage::{ExactMembership, IndexRead, OrderedRead};
use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};

mod wiring {
    pub use cq_state_providers::{
        default_indexes as index_provider, ordered_distinct as distinct_state,
    };
    pub use mini_linq::{
        index_demands as choose_index_policy, select_structures, structures_to_rust,
    };
    macro_rules! policy { ($($t:tt)*) => { ::mini_linq::lower_physical! { $($t)* policy {} } }; }
    pub(crate) use policy as physical_policy;
}
use wiring as mini_linq;

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
macro_rules! probe {
    ($l:ident,$r:ident) => {
        ::mini_linq::cq_to_ra! {
            struct Q; relation $l(a:i32); relation $r(a:i32,b:i32);
            answer(a,b) :- $l(a),$r(a,b);
        }
    };
}
macro_rules! nonleading {
    ($l:ident,$r:ident) => {
        ::mini_linq::cq_to_ra! {
            struct Q; relation $l(b:i32); relation $r(a:i32,b:i32);
            answer(a,b) :- $l(b),$r(a,b);
        }
    };
}
macro_rules! window {
    ($r:ident) => {
        ::mini_linq::cq_to_ra! {
            struct Q; relation $r(a:i32,b:i32);
            answer(a,b) :- $r(a,b), if a >= 2 && a < 9;
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
    let got: Vec<_> = ::mini_linq::cq_to_ra! { struct Q; relation l(a:i32); relation r(a:i32,b:i32); answer(a) :- l(a),r(a,_); }.collect();
    assert_eq!(got.len(), 2);
    assert_eq!(r.scans.get(), 0);
    assert_eq!(r.reads.get(), 3);
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
    let mut answers = ::mini_linq::cq_to_ra! {
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
fn host_and_fallback_range_iterators_survive_the_generated_block() {
    let rows: Vec<_> = (0..100_000).map(|a| (a, a)).collect();
    let r = Observed::new(
        rows.iter()
            .map(|&(a, b)| ((a,), vec![(b,)]))
            .collect::<BTreeMap<_, _>>(),
    );
    let mut answers = window!(r);
    assert_eq!((r.scans.get(), r.reads.get()), (0, 0));
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
fn ordered_host_ranges_build_zero_and_slices_build_once() {
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
    println!("ordered range: builds=0 native-cursors=1; slice range: builds=1");
}

#[test]
fn a_different_prefix_length_falls_back_even_when_the_host_has_a_key() {
    let l = [(1, 7)];
    let r = Observed::new(HashMap::from([((1,), vec![(7,), (8,)])]));
    let got: Vec<_> = ::mini_linq::cq_to_ra! { struct Q; relation l(a:i32,b:i32); relation r(a:i32,b:i32); answer(a,b) :- l(a,b),r(a,b); }.collect();
    assert_eq!(got.len(), 1);
    assert_eq!((r.scans.get(), r.reads.get()), (1, 0));
}

#[test]
fn ordered_key_types_need_no_hash_implementation_on_direct_reads() {
    #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
    struct Key(i32);
    let l = [(Key(1),), (Key(2),)];
    let r = BTreeMap::from([((Key(1),), vec![(7,)]), ((Key(2),), Vec::<(i32,)>::new())]);
    let got: Vec<_> = ::mini_linq::cq_to_ra! { struct Q; relation l(a:Key); relation r(a:Key,b:i32); answer(a,b) :- l(a),r(a,b); }.collect();
    assert_eq!(got.len(), 1);
    let got: Vec<_> = ::mini_linq::cq_to_ra! { struct Q; relation l(a:Key); relation r(a:Key,b:i32); answer(a) :- l(a),r(a,_); }.collect();
    assert_eq!(got.len(), 1, "an empty map group has no witness");
}

#[test]
fn ordered_ranges_follow_the_declared_numeric_type() {
    let r = BTreeMap::from([
        ((1_u64,), vec![(0,)]),
        ((2,), vec![(5,)]),
        ((9,), vec![(1,)]),
    ]);
    let got: Vec<_> = ::mini_linq::cq_to_ra! { struct Q; relation r(a:u64,b:i32); answer(a,b) :- r(a,b),if a < 9; }.collect();
    assert_eq!(got.len(), 2);
    let r = [(1_u64, 0), (2, 5), (9, 1)];
    let got: Vec<_> = ::mini_linq::cq_to_ra! { struct Q; relation r(a:u64,b:i32); answer(a,b) :- r(a,b),if a >= 2 && a < 9; }.collect();
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
    macro_rules! run { ($l:ident,$r:ident) => { ::mini_linq::cq_to_ra! { struct Q; relation $l(a:Key); relation $r(a:Key,b:i32); answer(a,b) :- $l(a),$r(a,b); } }; }
    let l = [(Key(1, false),), (Key(1, false),), (Key(2, false),)];
    let r = HashMap::from([((Key(1, true),), vec![(7,)])]);
    HASHES.with(|h| h.set((0, 0)));
    assert_eq!(run!(l, r).count(), 1);
    assert_eq!(HASHES.with(Cell::get), (0, 3));
    let rows = [(Key(1, true), 7)];
    let r = rows.as_slice();
    HASHES.with(|h| h.set((0, 0)));
    assert_eq!(run!(l, r).count(), 1);
    // One source row is hashed once into each of HashIndex's two bucket maps.
    assert_eq!(HASHES.with(Cell::get), (2, 3));
}

#[test]
fn composite_prefix_uses_declaration_order_and_shorter_keys_build() {
    let l = [(1, 2)];
    let r = Observed::new(HashMap::from([((2, 1), vec![(3,)])]));
    let got: Vec<_> = ::mini_linq::cq_to_ra! { struct Q; relation l(a:i32,b:i32); relation r(b:i32,a:i32,c:i32); answer(a,b,c) :- l(a,b),r(b,a,c); }.collect();
    assert_eq!(got, [(&1, &2, &3)]);
    assert_eq!((r.scans.get(), r.reads.get()), (0, 1));
    let l = [(2,)];
    let got: Vec<_> = ::mini_linq::cq_to_ra! { struct Q; relation l(b:i32); relation r(b:i32,a:i32,c:i32); answer(a,b,c) :- l(b),r(b,a,c); }.collect();
    assert_eq!(got, [(&1, &2, &3)]);
    assert_eq!((r.scans.get(), r.reads.get()), (1, 1));
}

#[test]
fn direct_merge_requires_ordered_read_without_requiring_payload_lookup() {
    let l = std::collections::BTreeSet::from([(1,), (2,)]);
    let r = std::collections::BTreeSet::from([(2,), (3,)]);
    let got: Vec<_> = ::mini_linq::select_structures! {
        physical {
            relation l(a:i32); relation r(a:i32);
            left = pipe rename l {a -> a}; right = pipe rename r {a -> a};
            j = pipe canon merge[a] left,right; return j as answer(a);
        } policy { join = merge; }
    }
    .collect();
    assert_eq!(got, [(&2,)]);
}

#[test]
fn ordered_key_sets_range_without_payload_lookup() {
    let r = std::collections::BTreeSet::from([(1,), (2,), (9,)]);
    let got: Vec<_> = ::mini_linq::cq_to_ra! { struct Q; relation r(a:i32); answer(a) :- r(a), if a >= 2 && a < 9; }.collect();
    assert_eq!(got, [(&2,)]);
}

#[test]
fn retained_ordered_ranges_preserve_unsigned_bounds() {
    let r = [(1_u64, 0), (2, 5), (9, 1)];
    let got: Vec<_> = ::mini_linq::select_structures! {
        physical {
            relation r(a:u64,b:i32); rows = let rename r {a -> a,b -> b};
            w = pipe canon range[a] rows if (a < 9); return w as answer(a,b);
        } policy {}
    }
    .collect();
    assert_eq!(got, [(&1, &0), (&2, &5)]);
}
