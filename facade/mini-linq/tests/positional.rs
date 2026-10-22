//! Generated execution checks, separate from runtime laws and IR contracts.
#![recursion_limit = "512"]
use cq_load::{MinMax, ZoneRows};
use std::collections::BTreeSet;
use std::num::NonZeroUsize;

#[test]
fn retained_masks_and_payload_builds_work_on_vec_slice_array_and_zone_hosts() {
    let left = vec![(1_i64, 10_i64), (1, 10), (1, 20), (2, 30), (3, 40), (5, 15)];
    let right = vec![(1_i64, 1_i64), (1, 1), (2, 0), (3, 1), (9, 1)];
    let expected = BTreeSet::from([(1, 10), (1, 20)]);
    assert_eq!(
        cq_testkit::positional_reduced!(
            [mini_linq::passes::sequence, mini_linq::passes::sequential]
            mini_linq::passes::translate_pull, left, right
        ),
        expected
    );
    assert_eq!(
        cq_testkit::positional_reduced!(
            [mini_linq::passes::sequence, mini_linq::passes::sequential]
            mini_linq::passes::translate_pull,
            left.as_slice(),
            right.as_slice()
        ),
        expected
    );
    assert_eq!(
        cq_testkit::positional_reduced!(
            [mini_linq::passes::sequence, mini_linq::passes::sequential]
            mini_linq::passes::translate_pull,
            [(1_i64, 10_i64), (1, 20)],
            [(1_i64, 1_i64)]
        ),
        expected
    );
    let left =
        ZoneRows::from_rows::<(MinMax<i64>, MinMax<i64>)>(left, NonZeroUsize::new(1).unwrap());
    let right =
        ZoneRows::from_rows::<(MinMax<i64>, MinMax<i64>)>(right, NonZeroUsize::new(1).unwrap());
    cq_storage::counters::reset();
    assert_eq!(
        cq_testkit::positional_reduced!(
            [mini_linq::passes::sequence, mini_linq::passes::sequential]
            mini_linq::passes::translate_pull, left, right
        ),
        expected
    );
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
            pipeline { mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull };
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
fn covered_exact_scan_filters_preserve_answers_and_skip_blocks() {
    let left = vec![
        (i64::MIN, 10_i64),
        (1, 9),
        (1, 20),
        (1, 20),
        (2, 30),
        (3, 40),
        (i64::MAX, 50),
    ];
    // The build keeps 2 distinct keys of 8 source rows: selective enough to
    // construct its filter (at most 35% kept).
    let right = vec![
        (1_i64, 2_i64),
        (1, 4),
        (1, 4),
        (2, 0),
        (i64::MAX, 4),
        (5, 0),
        (6, 0),
        (7, 0),
    ];
    let expected = BTreeSet::from([(1, 20, 2), (1, 20, 4), (i64::MAX, 50, 4)]);
    assert_eq!(
        filtered_join!(left,right,{b = let project s keep {x,z};}),
        expected
    );
    let left =
        ZoneRows::from_rows::<(MinMax<i64>, MinMax<i64>)>(left, NonZeroUsize::new(1).unwrap());
    cq_storage::counters::reset();
    assert_eq!(
        filtered_join!(left,right,{b = let project s keep {x,z};}),
        expected
    );
    let costs = cq_storage::counters::snapshot();
    assert_eq!(costs.join_filter_passed, 3);
    assert!(costs.blocks_skipped_metadata > 0);
    cq_storage::counters::reset();
    assert_eq!(
        filtered_join!(left,right,{
            h = pipe rename H@2 {e -> z,f -> w};
            b = let probe first [z] s,h;
        }),
        expected
    );
    assert_eq!(cq_storage::counters::snapshot().join_filter_passed, 3);
    let empty: Vec<(i64, i64)> = Vec::new();
    assert!(filtered_join!(left,empty,{b = let project s keep {x,z};}).is_empty());
    // A build keeping 2 distinct keys of 4 source rows (50%) is abandoned:
    // no filter is constructed or tested, and the exact probe alone decides.
    cq_storage::counters::reset();
    let unselective = vec![(1_i64, 2_i64), (1, 4), (2, 0), (i64::MAX, 4)];
    assert_eq!(
        filtered_join!(left,unselective,{b = let project s keep {x,z};}),
        expected
    );
    assert_eq!(cq_storage::counters::snapshot().join_filter_tested, 0);
}

cq_testkit::positional_cases!(
    [mini_linq::passes::sequence, mini_linq::passes::sequential]
    mini_linq::passes::translate_pull
);
