//! Both emitters execute identical checked decisions; runtime counters expose duplicate construction.
#![recursion_limit = "512"]
use cq_load::{MinMax, ZoneRows};
use std::collections::BTreeSet;
use std::num::NonZeroUsize;

macro_rules! compare_reduced {
    ($left:expr, $right:expr) => {{
        cq_storage::counters::reset();
        let pull = cq_testkit::positional_reduced!([mini_linq::passes::sequence, mini_linq::passes::sequential] mini_linq::passes::translate_pull, $left, $right);
        let pull_cost = cq_storage::counters::snapshot();
        cq_storage::counters::reset();
        let push = cq_testkit::positional_reduced!(own_push::push, $left, $right);
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
fn positional_predicate_does_not_shadow_an_opaque_guard_capture() {
    fn keep(x: &i64, limit: i64) -> bool {
        *x < limit
    }
    let __cq_r1_predicate = 2_i64;
    let __push_position_r1 = 2_i64;
    let rows = [(1_i64,), (2,), (3,)];
    macro_rules! run {
        ($endpoint:ident, $($printer:ident)::+) => {
            mini_linq::compile! {
                pipeline { print_input = $endpoint; print = $($printer)::+; select = mini_linq::passes::select(positional(r)); }
                struct GuardHygiene;
                relation r(x:i64) = (rows);
                answer(x) :- r(x), where x >= 1, if keep(x, __cq_r1_predicate), if keep(x, __push_position_r1);
            }
            .map(|(x,)| *x)
            .collect()
        };
    }
    let values: Vec<_> = run!(sequence, mini_linq::passes::translate_pull);
    let pushed: Vec<_> = run!(structures, own_push::push);
    assert_eq!(values, pushed);
    assert_eq!(values, vec![1]);
}

#[test]
fn borrowed_keysets_preserve_positive_and_negative_membership() {
    macro_rules! run {
        ($([$($passes:path),*])? $($printer:ident)::+, $operator:ident, $budget:literal, $left:expr, $right:expr) => {
            $($printer)::+! {
                pipeline { $($($passes),*)? };
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
                    out canon = sort;
                }
            }.map(|(x,)| *x).collect::<BTreeSet<_>>()
        };
    }
    macro_rules! compare {
        ($operator:ident, $budget:literal, $left:expr, $right:expr, $expected:expr) => {{
            let pull = run!(
                [mini_linq::passes::sequential, mini_linq::passes::translate_pull] mini_linq::passes::sequence,
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
        ($endpoint:ident, $printer:path, $comparison:tt) => {
            mini_linq::compile! {
                pipeline {
                    select = mini_linq::passes::select(positional(R));
                    print_input = $endpoint;
                    print = $printer;
                }
                struct BorrowedParameter;
                relation R(x:i64) = (rows);
                answer(x) :- R(x), where x $comparison cutoff;
            }
            .map(|(x,)| *x)
            .collect::<Vec<_>>()
        };
    }
    assert_eq!(run!(sequence, mini_linq::passes::translate_pull, ==), [2]);
    assert_eq!(run!(structures, own_push::push, ==), [2]);
    assert_eq!(
        run!(sequence, mini_linq::passes::translate_pull, >=),
        [2, 3]
    );
    assert_eq!(run!(structures, own_push::push, >=), [2, 3]);
}

#[test]
fn pipe_read_spines_allocate_only_the_retained_snapshot() {
    macro_rules! run {
        ([$($pass:path),*] $($printer:ident)::+, $source:expr, $mode:ident, {$($extra:tt)*}) => {{
            cq_storage::counters::reset();
            let mut occurrences = 0;
            let rows: BTreeSet<_> = $($printer)::+! {
                context { consumer = canonical };
                pipeline { $($pass),* };
                physical {
                    relation R(a:i64,b:i64) = ($source);
                    consumer canonical;
                    r = pipe rename R@0 {a -> x,b -> y};
                    first = $mode range select r if (x >= 1);
                    projected = pipe project first keep {x,y};
                    second = pipe range select projected if (y < 20);
                    last = let range select second if (x < 4);
                    return last as answer(x,y);
                }
                structures {
                    r host scan = ::core::iter::Map<_,_> [];
                    first read = positional base R lineage{x -> a,y -> b} column a marker ::cq_load::ColumnAt<0> predicate physical;
                    second read = positional base R lineage{x -> a,y -> b} column b marker ::cq_load::ColumnAt<1> predicate physical;
                    last read = positional base R lineage{x -> a,y -> b} column a marker ::cq_load::ColumnAt<0> predicate physical;
                    $($extra)*
                    last selection = ::cq_storage::Selection<_> base R lineage{x -> a,y -> b} summary per_block;
                }
            }.inspect(|_| occurrences += 1).map(|(x,y)| (*x,*y)).collect();
            (rows, cq_storage::counters::snapshot(), occurrences)
        }};
    }
    macro_rules! compare {
        ($source:expr, $len:expr, $blocks:expr, $expected:expr) => {{
            let piped = run!(
                [mini_linq::passes::sequential, mini_linq::passes::translate_pull]
                mini_linq::passes::sequence, $source, pipe, {}
            );
            let pushed = run!([] own_push::push, $source, pipe, {});
            assert_eq!(piped, pushed);
            assert_eq!(piped.0, $expected);
            let mask_bytes = 8 * ($len.div_ceil(64) + $blocks.div_ceil(64));
            assert_eq!(piped.1.bytes_retained_peak, mask_bytes as u64);
            assert_eq!(piped.1.rows_copied, 0);
            assert_eq!(piped.2, if $len == 0 { 0 } else { 3 });

            let retained = run!(
                [mini_linq::passes::sequential, mini_linq::passes::translate_pull]
                mini_linq::passes::sequence, $source, let, {
                    first selection = ::cq_storage::Selection<_> base R lineage{x -> a,y -> b} summary per_block;
                }
            );
            let retained_push = run!([] own_push::push, $source, let, {
                first selection = ::cq_storage::Selection<_> base R lineage{x -> a,y -> b} summary per_block;
            });
            assert_eq!(retained, retained_push);
            assert_eq!(retained.0, piped.0);
            assert_eq!(retained.2, piped.2);
            assert_eq!(retained.1.bytes_retained_peak, 2 * mask_bytes as u64);
            assert_eq!(retained.1.rows_copied, 0);
        }};
    }
    let mut data = vec![(1_i64, 10_i64), (1, 10), (2, 30), (3, 15), (4, 10)];
    data.extend(std::iter::repeat_n((0, 0), 252));
    let expected = BTreeSet::from([(1, 10), (3, 15)]);
    compare!(data.as_slice(), data.len(), 1usize, expected);
    for block_size in [1, 3, 63, 64, 65, 130] {
        let host = ZoneRows::from_rows::<(MinMax<i64>, MinMax<i64>)>(
            data.clone(),
            NonZeroUsize::new(block_size).unwrap(),
        );
        compare!(host, data.len(), data.len().div_ceil(block_size), expected);
    }
    let empty: [(i64, i64); 0] = [];
    compare!(empty, 0usize, 0usize, BTreeSet::<(i64, i64)>::new());
}

cq_testkit::positional_cases!(structures; own_push::push);
