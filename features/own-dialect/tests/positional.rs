//! Both emitters execute identical checked decisions; runtime counters expose duplicate construction.
#![recursion_limit = "512"]
use cq_load::{MinMax, ZoneRows};
use std::collections::BTreeSet;
use std::num::NonZeroUsize;

macro_rules! reduced {
    ($($printer:ident)::+, $left:expr, $right:expr) => { reduced!(@typed $($printer)::+, i64, $left, $right) };
    (@typed $($printer:ident)::+, $ty:ty, $left:expr, $right:expr) => {
        mini_linq::passes::select! {
            arguments { positional(L, R) };
            pipeline { $($printer)::+ };
            physical {
                relation L(a:$ty, b:$ty) = ($left);
                relation R(c:$ty, flag:$ty) = ($right);
                l = pipe rename L@0 { a -> x, b -> y };
                window = pipe range select l if (y >= 10, y < 40);
                r = pipe rename R@1 { c -> x, flag -> f };
                selected = let point select r if (f = 1);
                projected = pipe project window keep {x,y};
                reduced = let semi eager [x] projected, selected;
                back = pipe semi eager [x] selected, reduced;
                joined = pipe probe first [x] back, reduced;
                result = pipe canon project joined keep {x,y};
                return result as answer(x,y);
            }
        }
        .map(|(x, y)| (*x, *y))
        .collect::<BTreeSet<_>>()
    };
}
macro_rules! compare_reduced {
    ($left:expr, $right:expr) => {{
        cq_storage::counters::reset();
        let pull = reduced!(mini_linq::passes::print_pull, $left, $right);
        let pull_cost = cq_storage::counters::snapshot();
        cq_storage::counters::reset();
        let push = reduced!(own_push::push, $left, $right);
        let push_cost = cq_storage::counters::snapshot();
        assert_eq!(pull, push);
        assert_eq!(
            pull_cost, push_cost,
            "push must reuse the same snapshots/builds"
        );
        push
    }};
}
#[test]
fn retained_masks_reuse_payloads_on_vec_slice_array_and_partial_zone_blocks() {
    let left = vec![(1_i64, 10_i64), (1, 10), (1, 20), (2, 30), (3, 40), (5, 15)];
    let right = vec![(1_i64, 1_i64), (1, 1), (2, 0), (3, 1), (9, 1)];
    let expected = BTreeSet::from([(1, 10), (1, 20)]);
    assert_eq!(compare_reduced!(left, right), expected);
    assert_eq!(
        cq_storage::counters::snapshot().rows_copied,
        3,
        "only the three surviving occurrences enter the payload index; no retained tuple buffer"
    );
    assert_eq!(
        compare_reduced!(left.as_slice(), right.as_slice()),
        expected
    );
    assert_eq!(
        compare_reduced!([(1_i64, 10_i64), (1, 20)], [(1_i64, 1_i64)]),
        expected
    );
    for block_rows in [1, 2, 4, 2048] {
        let mut long_left = left.clone();
        long_left.extend((0..2045).map(|i| (100 + i, 50)));
        long_left.push((1, 15));
        let left = ZoneRows::from_rows::<(MinMax<i64>, MinMax<i64>)>(
            long_left,
            NonZeroUsize::new(block_rows).unwrap(),
        );
        let right = ZoneRows::from_rows::<(MinMax<i64>, MinMax<i64>)>(
            right.clone(),
            NonZeroUsize::new(block_rows).unwrap(),
        );
        assert_eq!(
            compare_reduced!(left, right),
            BTreeSet::from([(1, 10), (1, 15), (1, 20)])
        );
        let costs = cq_storage::counters::snapshot();
        assert_eq!(
            costs.blocks_considered,
            costs.blocks_visited + costs.blocks_skipped
        );
        assert!(costs.keyset_staging_bytes_peak > 0);
        if block_rows < 2048 {
            assert!(costs.blocks_skipped > 0);
        }
    }
    let empty: Vec<(i64, i64)> = vec![];
    assert!(compare_reduced!(empty, right).is_empty());
    assert!(compare_reduced!(left, empty).is_empty());
    assert!(compare_reduced!(empty, empty).is_empty());
}
#[test]
fn conflicting_points_keep_all_residuals_before_the_next_read() {
    let rows = [(1_i64, 10_i64), (2, 10), (3, 20)];
    macro_rules! run {
        ($($printer:ident)::+) => {
            mini_linq::compile! {
                pipeline { print = $($printer)::+; select = mini_linq::passes::select(positional(R)); }
                struct Conflicting;
                relation R(a:i64,b:i64) = (rows);
                answer(a) :- R(a,b), if *a == 1 && *a == 2 && *b == 10;
            }
            .collect()
        };
    }
    let result: Vec<_> = run!(mini_linq::passes::print_pull);
    let pushed: Vec<_> = run!(own_push::push);
    assert_eq!(result, pushed);
    assert!(result.is_empty());
}
#[test]
fn nonleading_filtered_index_keeps_payload_and_constant_reconstruction() {
    let left = [(1_i64,), (2,), (3,)];
    let right = [
        (1_i64, "yes".to_owned(), 10_i64),
        (2, "no".to_owned(), 20),
        (3, "yes".to_owned(), 30),
    ];
    macro_rules! run {
        ($($printer:ident)::+) => {
            mini_linq::compile! {
                pipeline { print = $($printer)::+; select = mini_linq::passes::select(positional(L,R)); }
                struct Filtered;
                relation L(a:i64) = (left);
                relation R(a:i64,label:String,value:i64) = (right);
                answer(a,label,value) :- L(a), R(a,label,value), if label == "yes";
            }
            .map(|(a, l, v)| (*a, l.clone(), *v))
            .collect()
        };
    }
    let result: BTreeSet<_> = run!(mini_linq::passes::print_pull);
    let pushed: BTreeSet<_> = run!(own_push::push);
    assert_eq!(result, pushed);
    assert_eq!(
        result,
        BTreeSet::from([(1, "yes".to_owned(), 10), (3, "yes".to_owned(), 30)])
    );
}

#[derive(Clone, Copy)]
struct UnusedRight;
impl cq_storage::rows::Rows for UnusedRight {
    type Row = (&'static i64,);
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        std::iter::from_fn(|| panic!("an empty left must not force its RHS"))
    }
}
#[test]
fn empty_positional_left_does_not_force_lazy_semi_rhs() {
    let left = [(1_i64,), (2,)];
    let right = UnusedRight;
    macro_rules! run {
        ($($printer:ident)::+) => {
            mini_linq::passes::select! {
                arguments { positional(L) };
                pipeline { $($printer)::+ };
                physical {
                    relation L(a:i64) = (left); relation R(b:i64) = (right);
                    l = pipe rename L@0 {a -> x}; empty = pipe range select l if (x > 100);
                    r = pipe rename R@1 {b -> x};
                    s = pipe semi lazy [x] empty,r;
                    out = pipe canon project s keep {x}; return out as answer(x);
                }
            }
            .collect()
        };
    }
    let result: Vec<_> = run!(mini_linq::passes::print_pull);
    let pushed: Vec<_> = run!(own_push::push);
    assert_eq!(result, pushed);
    assert!(result.is_empty());
}

struct PayloadOnly<T>(T);
impl<T: cq_storage::IndexRead> Clone for PayloadOnly<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<I: Iterator, T: cq_storage::Build<I>> cq_storage::Build<I> for PayloadOnly<T> {
    fn build(source: I) -> Self {
        Self(T::build(source))
    }
}
impl<T: cq_storage::IndexRead> cq_storage::IndexRead for PayloadOnly<T> {
    type Key = T::Key;
    type Item = T::Item;
    type Cursor = T::Cursor;
    fn lookup(&self, key: &Self::Key) -> Self::Cursor {
        self.0.lookup(key)
    }
}

#[test]
fn custom_aliases_and_bare_build_provider_keep_selected_source_contracts() {
    type Mask<H> = cq_storage::Selection<H>;
    type Keys<K> = cq_storage::AdaptiveKeySet<K, cq_storage::I64Codec, 64>;
    type Payload<'a, K, V> = PayloadOnly<cq_storage::HashIndex<'a, K, V>>;
    let left = [(1_i64, 10_i64), (1, 20), (2, 30)];
    let right = [(1_i64, 1_i64), (2, 0)];
    macro_rules! run { ($($printer:ident)::+) => { $($printer)::+! {
        pipeline {};
        physical {
            relation L(a:i64,b:i64) = (left); relation R(c:i64,d:i64) = (right);
            l = pipe rename L@0 {a -> x,b -> y}; r = pipe rename R@1 {c -> x,d -> z};
            s = let point select r if (z = 1); t = let semi eager [x] l,s;
            u = pipe semi eager [x] s,t; j = pipe probe first [x] u,t;
            out = pipe canon project j keep {x,y}; return out as answer(x,y);
        }
        structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            s selection = Mask<_> base R lineage{x -> c,z -> d} summary none;
            s keyset = Keys<_> [x] owned base R lineage{x -> c,z -> d};
            t selection = Mask<_> base L lineage{x -> a,y -> b} summary per_block;
            t = Payload [x];
            t keyset = Keys<_> [x] owned base L lineage{x -> a,y -> b};
            s read = positional base R lineage{x -> c,z -> d} column d marker ::cq_load::ColumnAt<1> predicate physical;
            t read = positional base L lineage{x -> a,y -> b} column a marker ::cq_load::ColumnAt<0> predicate membership s[x] summary exact;
            u read = positional base R lineage{x -> c,z -> d} column c marker ::cq_load::ColumnAt<0> predicate membership t[x] summary exact;
            out canon = ::std::collections::HashSet<_>;
        }
    }.map(|(x,y)| (*x,*y)).collect() }; }
    let actual: BTreeSet<_> = run!(mini_linq::passes::print_pull);
    let pushed: BTreeSet<_> = run!(own_push::push);
    assert_eq!(actual, pushed);
    assert_eq!(actual, BTreeSet::from([(1, 10), (1, 20)]));
}

#[test]
fn positional_bindings_do_not_shadow_captured_parameters() {
    let __cq_positional_r = 2_i64;
    let rows = [(1_i64,), (2,), (3,)];
    macro_rules! run {
        ($($printer:ident)::+) => {
            mini_linq::compile! {
                pipeline { print = $($printer)::+; select = mini_linq::passes::select(positional(r)); }
                struct Hygiene;
                relation r(x:i64) = (rows);
                answer(x) :- r(x), if *x < __cq_positional_r;
            }
            .map(|(x,)| *x)
            .collect()
        };
    }
    let values: Vec<_> = run!(mini_linq::passes::print_pull);
    let pushed: Vec<_> = run!(own_push::push);
    assert_eq!(values, pushed);
    assert_eq!(values, vec![1]);
}

#[test]
fn positional_predicate_does_not_shadow_an_opaque_guard_capture() {
    fn keep(x: &i64, limit: i64) -> bool {
        *x < limit
    }
    let __cq_r1_predicate = 2_i64;
    let __push_position_r1 = 2_i64;
    let rows = [(1_i64,), (2,), (3,)];
    macro_rules! run {
        ($($printer:ident)::+) => {
            mini_linq::compile! {
                pipeline { print = $($printer)::+; select = mini_linq::passes::select(positional(r)); }
                struct GuardHygiene;
                relation r(x:i64) = (rows);
                answer(x) :- r(x), if *x >= 1, if keep(x, __cq_r1_predicate), if keep(x, __push_position_r1);
            }
            .map(|(x,)| *x)
            .collect()
        };
    }
    let values: Vec<_> = run!(mini_linq::passes::print_pull);
    let pushed: Vec<_> = run!(own_push::push);
    assert_eq!(values, pushed);
    assert_eq!(values, vec![1]);
}

#[test]
fn a_computed_left_key_can_test_an_owned_set_without_becoming_a_selection() {
    let left = [(1_i64,), (7,)];
    let right = [(2_i64, 1_i64), (8, 0)];
    macro_rules! run {
        ($($printer:ident)::+) => {
            mini_linq::passes::select! {
                arguments { positional(L,R) };
                pipeline { $($printer)::+ };
                physical {
                    relation L(a:i64) = (left); relation R(b:i64,c:i64) = (right);
                    l = pipe rename L@0 {a -> a};
                    computed = pipe bind l with x:i64 = (*a + 1);
                    r = pipe rename R@1 {b -> x,c -> z};
                    s = let point select r if (z = 1);
                    t = let semi eager [x] computed,s;
                    back = pipe semi eager [x] s,t;
                    out = pipe canon project back keep {x}; return out as answer(x);
                }
            }
            .map(|(x,)| *x)
            .collect()
        };
    }
    let values: Vec<_> = run!(mini_linq::passes::print_pull);
    let pushed: Vec<_> = run!(own_push::push);
    assert_eq!(values, pushed);
    assert_eq!(values, vec![2]);
}

#[test]
fn raw_identifiers_survive_recorded_base_and_column_lineage() {
    let rows = [(1_i64,), (2,)];
    macro_rules! run { ($($printer:ident)::+) => { mini_linq::compile! {
        pipeline { print = $($printer)::+; select = mini_linq::passes::select(positional(r#type)); }
        struct Raw;
        relation r#type(r#match:i64) = (rows);
        answer(r#loop) :- r#type(r#loop), if *r#loop > 1;
    }
    .map(|(x,)| *x)
    .collect() }; }
    let values: Vec<_> = run!(mini_linq::passes::print_pull);
    let pushed: Vec<_> = run!(own_push::push);
    assert_eq!(values, pushed);
    assert_eq!(values, vec![2]);
}

#[test]
fn borrowed_keysets_preserve_positive_and_negative_membership() {
    macro_rules! run {
        ($($printer:ident)::+, $operator:ident, $budget:literal, $left:expr, $right:expr) => {
            $($printer)::+! {
                pipeline {};
                physical {
                    relation L(a:i64) = ($left); relation R(b:i64) = ($right);
                    l = pipe rename L@0 {a -> x}; r = pipe rename R@1 {b -> x};
                    s = let project r keep {x};
                    out = pipe canon $operator eager [x] l,s;
                    return out as answer(x);
                }
                structures {
                    l host scan = ::core::iter::Map<_,_> [];
                    r host scan = ::core::iter::Map<_,_> [];
                    s rows = ::std::vec::Vec<_>;
                    s keyset = ::cq_storage::AdaptiveKeySet<_,::cq_storage::BorrowedI64TupleCodec,$budget> [x] borrowed base R lineage{x -> b};
                    out canon = ::std::collections::HashSet<_>;
                }
            }.map(|(x,)| *x).collect::<BTreeSet<_>>()
        };
    }
    macro_rules! compare {
        ($operator:ident, $budget:literal, $left:expr, $right:expr, $expected:expr) => {{
            let pull = run!(
                mini_linq::passes::print_pull,
                $operator,
                $budget,
                $left,
                $right
            );
            let push = run!(own_push::push, $operator, $budget, $left, $right);
            assert_eq!(pull, push);
            assert_eq!(push, $expected);
        }};
    }
    let left = [(1_i64,), (1,), (2,), (3,), (i64::MAX,)];
    let right = [(1_i64,), (1,), (3,)];
    let empty: [(i64,); 0] = [];
    compare!(semi, 64, left, right, BTreeSet::from([1, 3]));
    compare!(anti, 64, left, right, BTreeSet::from([2, i64::MAX]));
    compare!(semi, 0, left, right, BTreeSet::from([1, 3]));
    compare!(anti, 0, left, empty, BTreeSet::from([1, 2, 3, i64::MAX]));
    compare!(semi, 64, empty, right, BTreeSet::new());
}

#[test]
fn borrowed_scalar_point_and_range_parameters_agree() {
    let rows = [(1_i64,), (2,), (3,)];
    let cutoff = &&2_i64;
    macro_rules! run {
        ($printer:path, $comparison:tt) => {
            mini_linq::compile! {
                pipeline {
                    select = mini_linq::passes::select(positional(R));
                    print = $printer;
                }
                struct BorrowedParameter;
                relation R(x:i64) = (rows);
                answer(x) :- R(x), if *x $comparison cutoff;
            }
            .map(|(x,)| *x)
            .collect::<Vec<_>>()
        };
    }
    assert_eq!(run!(mini_linq::passes::print_pull, ==), [2]);
    assert_eq!(run!(own_push::push, ==), [2]);
    assert_eq!(run!(mini_linq::passes::print_pull, >=), [2, 3]);
    assert_eq!(run!(own_push::push, >=), [2, 3]);
}
