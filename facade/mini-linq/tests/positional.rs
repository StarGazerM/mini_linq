//! Generated execution checks, separate from runtime laws and IR contracts.
#![recursion_limit = "512"]
use cq_load::{MinMax, ZoneRows};
use std::collections::BTreeSet;
use std::num::NonZeroUsize;

macro_rules! reduced {
    ($left:expr, $right:expr) => { reduced!(@typed i64, $left, $right) };
    (@typed $ty:ty, $left:expr, $right:expr) => {
        mini_linq::passes::select! {
            arguments { positional(L, R) };
            pipeline { mini_linq::passes::print_pull };
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
#[test]
fn retained_masks_and_payload_builds_work_on_vec_slice_array_and_zone_hosts() {
    let left = vec![(1_i64, 10_i64), (1, 10), (1, 20), (2, 30), (3, 40), (5, 15)];
    let right = vec![(1_i64, 1_i64), (1, 1), (2, 0), (3, 1), (9, 1)];
    let expected = BTreeSet::from([(1, 10), (1, 20)]);
    assert_eq!(reduced!(left, right), expected);
    assert_eq!(reduced!(left.as_slice(), right.as_slice()), expected);
    assert_eq!(
        reduced!([(1_i64, 10_i64), (1, 20)], [(1_i64, 1_i64)]),
        expected
    );
    let left =
        ZoneRows::from_rows::<(MinMax<i64>, MinMax<i64>)>(left, NonZeroUsize::new(1).unwrap());
    let right =
        ZoneRows::from_rows::<(MinMax<i64>, MinMax<i64>)>(right, NonZeroUsize::new(1).unwrap());
    cq_storage::counters::reset();
    assert_eq!(reduced!(left, right), expected);
    let costs = cq_storage::counters::snapshot();
    assert!(costs.blocks_skipped > 0);
    assert!(
        costs.keyset_staging_bytes_peak > 0,
        "forwarded macro types must still select scalar keysets"
    );
    assert_eq!(
        costs.blocks_considered,
        costs.blocks_visited + costs.blocks_skipped
    );
}
#[test]
fn conflicting_points_keep_all_residuals_before_the_next_read() {
    let rows = [(1_i64, 10_i64), (2, 10), (3, 20)];
    let result: Vec<_> = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(R)); }
        struct Conflicting;
        relation R(a:i64,b:i64) = (rows);
        answer(a) :- R(a,b), if *a == 1 && *a == 2 && *b == 10;
    }
    .collect();
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
    let result: BTreeSet<_> = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(L,R)); }
        struct Filtered;
        relation L(a:i64) = (left);
        relation R(a:i64,label:String,value:i64) = (right);
        answer(a,label,value) :- L(a), R(a,label,value), if label == "yes";
    }
    .map(|(a, l, v)| (*a, l.clone(), *v))
    .collect();
    assert_eq!(
        result,
        BTreeSet::from([(1, "yes".to_owned(), 10), (3, "yes".to_owned(), 30)])
    );
}

#[test]
fn replayed_union_builds_own_each_positional_point_predicate() {
    let left = [(1_i32,), (2,), (3,)];
    let right = [(1_i32,), (2,), (3,)];
    let countries = [
        (1_i32, "FRANCE".to_owned()),
        (2, "GERMANY".to_owned()),
        (3, "OTHER".to_owned()),
    ];
    let result: BTreeSet<_> = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(L,R,N)); }
        struct ReplayedUnion;
        relation L(x:i32) = (left);
        relation R(y:i32) = (right);
        relation N(key:i32,name:String) = (countries);
        answer(x,y) :- L(x), R(y),
            { N(x,"FRANCE"), N(y,"GERMANY") }
            | { N(x,"GERMANY"), N(y,"FRANCE") };
    }
    .map(|(x, y)| (*x, *y))
    .collect();
    assert_eq!(result, BTreeSet::from([(1, 2), (2, 1)]));
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
    let result: Vec<_> = mini_linq::passes::select! {
        arguments { positional(L) };
        pipeline { mini_linq::passes::print_pull };
        physical {
            relation L(a:i64) = (left); relation R(b:i64) = (right);
            l = pipe rename L@0 {a -> x}; empty = pipe range select l if (x > 100);
            r = pipe rename R@1 {b -> x};
            s = pipe semi lazy [x] empty,r;
            out = pipe canon project s keep {x}; return out as answer(x);
        }
    }
    .collect();
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
    let actual: BTreeSet<_> = mini_linq::passes::print_pull! {
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
    }.map(|(x,y)| (*x,*y)).collect();
    assert_eq!(actual, BTreeSet::from([(1, 10), (1, 20)]));
}

#[test]
fn positional_bindings_do_not_shadow_captured_parameters() {
    let __cq_positional_r = 2_i64;
    let rows = [(1_i64,), (2,), (3,)];
    let values: Vec<_> = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(r)); }
        struct Hygiene;
        relation r(x:i64) = (rows);
        answer(x) :- r(x), if *x < __cq_positional_r;
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(values, vec![1]);
}

#[test]
fn positional_predicate_does_not_shadow_an_opaque_guard_capture() {
    fn keep(x: &i64, limit: i64) -> bool {
        *x < limit
    }
    let __cq_r1_predicate = 2_i64;
    let rows = [(1_i64,), (2,), (3,)];
    let values: Vec<_> = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(r)); }
        struct GuardHygiene;
        relation r(x:i64) = (rows);
        answer(x) :- r(x), if *x >= 1, if keep(x, __cq_r1_predicate);
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(values, vec![1]);
}

#[test]
fn a_computed_left_key_can_test_an_owned_set_without_becoming_a_selection() {
    let left = [(1_i64,), (7,)];
    let right = [(2_i64, 1_i64), (8, 0)];
    let values: Vec<_> = mini_linq::passes::select! {
        arguments { positional(L,R) };
        pipeline { mini_linq::passes::print_pull };
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
    .collect();
    assert_eq!(values, vec![2]);
}

#[test]
#[allow(non_snake_case)]
fn pipe_reads_remain_lazy_without_an_output_tuple_buffer() {
    fn partial(_: &i64) -> bool {
        panic!("unconsumed pipe reached its guard")
    }
    let R = [(1_i64,), (2,), (3,)];
    cq_storage::counters::reset();
    let rows = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(R)); }
        struct LazyRead;
        relation R(x:i64);
        answer(x) :- R(x), if *x >= 1, if partial(x);
    };
    assert_eq!(cq_storage::counters::snapshot().rows_tested, 0);
    assert_eq!(rows.take(0).count(), 0);
}

#[test]
fn raw_identifiers_survive_recorded_base_and_column_lineage() {
    let rows = [(1_i64,), (2,)];
    let values: Vec<_> = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(r#type)); }
        struct Raw;
        relation r#type(r#match:i64) = (rows);
        answer(r#loop) :- r#type(r#loop), if *r#loop > 1;
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(values, vec![2]);
}

#[test]
fn borrowed_scalar_parameters_match_the_recorded_column_key_type() {
    let rows = [(1_i64,), (2,), (3,)];
    let cutoff = &&2_i64;
    let point: Vec<_> = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(R)); }
        struct PointParameter;
        relation R(x:i64) = (rows);
        answer(x) :- R(x), if *x == cutoff;
    }
    .map(|(x,)| *x)
    .collect();
    let range: Vec<_> = mini_linq::compile! {
        pipeline { select = mini_linq::passes::select(positional(R)); }
        struct RangeParameter;
        relation R(x:i64) = (rows);
        answer(x) :- R(x), if *x >= cutoff;
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(point, [2]);
    assert_eq!(range, [2, 3]);
}

macro_rules! filtered_join {
    ($left:expr, $right:expr, {$($build:tt)*}) => {
        mini_linq::passes::select! {
            arguments { positional(L,R,H) };
            pipeline { mini_linq::passes::print_pull };
            physical {
                relation L(a:i64,b:i64) = ($left);
                relation R(c:i64,d:i64) = ($right);
                relation H(e:i64,f:i64) = ([(2_i64,7_i64),(2,8),(4,9)]);
                l = pipe rename L@0 {a -> x,b -> y};
                projected = pipe project l keep {x,y};
                window = pipe range select projected if (y >= 10);
                r = pipe rename R@1 {c -> x,d -> z};
                s = pipe range select r if (z > 1);
                $($build)*
                j = pipe probe first [x] window,b;
                out = pipe canon project j keep {x,y,z}; return out as answer(x,y,z);
            }
        }.map(|(x,y,z)| (*x,*y,*z)).collect::<BTreeSet<_>>()
    };
}

#[test]
fn covered_exact_and_bloom_scan_filters_preserve_answers_and_skip_blocks() {
    let left = vec![(i64::MIN,10_i64),(1,9),(1,20),(1,20),(2,30),(3,40),(i64::MAX,50)];
    let right = vec![(1_i64,2_i64),(1,4),(1,4),(2,0),(i64::MAX,4)];
    let expected = BTreeSet::from([(1,20,2),(1,20,4),(i64::MAX,50,4)]);
    assert_eq!(filtered_join!(left,right,{b = let project s keep {x,z};}), expected);
    let left = ZoneRows::from_rows::<(MinMax<i64>,MinMax<i64>)>(left, NonZeroUsize::new(1).unwrap());
    cq_storage::counters::reset();
    assert_eq!(filtered_join!(left,right,{b = let project s keep {x,z};}), expected);
    let costs = cq_storage::counters::snapshot();
    assert_eq!(costs.join_filter_passed, 3);
    assert!(costs.blocks_skipped_metadata > 0);
    cq_storage::counters::reset();
    assert_eq!(filtered_join!(left,right,{
        h = pipe rename H@2 {e -> z,f -> w};
        b = let probe first [z] s,h;
    }), expected);
    assert_eq!(cq_storage::counters::snapshot().join_filter_passed, 3);
    let empty: Vec<(i64,i64)> = Vec::new();
    assert!(filtered_join!(left,empty,{b = let project s keep {x,z};}).is_empty());
}
