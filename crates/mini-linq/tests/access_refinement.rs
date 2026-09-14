//! Native Access leaves compose independently of RA and retain exact residuals.
use cq_storage::{BlockStore, Counters, RangeSummary};
use std::cell::Cell;
use std::collections::HashSet;
use std::num::NonZeroUsize;

fn observe<'a, T>(calls: &Cell<usize>, value: &'a T) -> &'a T {
    calls.set(calls.get() + 1);
    value
}

#[test]
fn key_builders_evaluate_access_sources_and_optional_work_once() {
    let right = HashSet::from([[2_i64], [7]]);
    let source_calls = Cell::new(0);
    let source = &right;
    let source_count = &source_calls;
    let work_calls = Cell::new(0);
    let work = Counters::default();
    macro_rules! build {
        ($implementation:ident) => {
            ::mini_linq::$implementation! {
                storage ::cq_storage;
                iterator ::mini_linq::stream_access;
                keys ::std::collections::HashSet;
                work (observe(&work_calls, &work));
                build {
                    keys(k) => ([k]) :- R(k) => for (k,) in (
                        observe(source_count, source).iter().map(|r| (r[0],))
                    );
                }
            }
        };
    }
    let exact = build!(exact_keys);
    assert_eq!(exact, right);
    assert_eq!((source_calls.get(), work_calls.get()), (1, 1));
    assert_eq!(work.take().index_rows, right.len());
    source_calls.set(0);
    work_calls.set(0);
    let zone = build!(zone_keys);
    assert_eq!(zone.0, right);
    assert!(RangeSummary::may_match(&zone.1, &[(2, 2)], &[0]));
    assert!(!RangeSummary::may_match(&zone.1, &[(8, 8)], &[0]));
    assert_eq!((source_calls.get(), work_calls.get()), (1, 1));
    assert_eq!(work.take().index_rows, right.len());
}

#[test]
fn ordinary_scan_does_not_evaluate_or_expand_a_metadata_predicate() {
    let left = HashSet::from([[9_i64, 2], [8, 3]]);
    let source_calls = Cell::new(0);
    macro_rules! rows {
        ($($operation:tt)*) => { ::mini_linq::hash_rows! { storage ::cq_storage; $($operation)* } };
    }
    let candidates = mini_linq::scan_rows! {
        storage ::cq_storage;
        iterator ::mini_linq::stream_access;
        candidates rows { observe(&source_calls, &left) }
            (compile_error!("native scan must not require block metadata"))
    };
    assert_eq!(candidates.copied().collect::<HashSet<_>>(), left);
    assert_eq!(source_calls.get(), 1);
}

#[test]
fn pruned_access_checks_metadata_before_open_and_keeps_the_exact_residual() {
    let store = BlockStore::from_rows(
        [[9_i64, 1], [8, 2], [7, 3], [6, 90]],
        NonZeroUsize::new(1).unwrap(),
    );
    let view = mini_linq::block_rows! { storage ::cq_storage; input (&store) };
    let keys = mini_linq::zone_keys! {
        storage ::cq_storage; iterator ::mini_linq::stream_access; keys ::std::collections::HashSet;
        build { keys(k) => ([k]) :- K(k) => for (k,) in ([(1_i64,), (3,)]); }
    };
    let keys_ref = &keys;
    let work = Counters::default();
    let observed_work = &work;
    let source_calls = Cell::new(0);
    macro_rules! rows {
        ($($operation:tt)*) => {
            ::mini_linq::block_rows! { storage ::cq_storage; work (observed_work); $($operation)* }
        };
    }
    let candidates = mini_linq::prune_rows! {
        storage ::cq_storage; iterator ::mini_linq::stream_access; work (observed_work);
        candidates rows { observe(&source_calls, &view) }
            (|bounds: &[(i64, i64)]| mini_linq::zone_keys! {
                storage ::cq_storage; iterator ::mini_linq::stream_access;
                may_match (keys_ref) (bounds) (&[1])
            })
    };
    let selected: HashSet<_> = mini_linq::collect_access! {
        out(h) => (*rows! { row (h) }) :-
        Candidate(h) => for (h,) in (candidates.map(|h| (h,))),
        if mini_linq::zone_keys! {
            storage ::cq_storage; iterator ::mini_linq::stream_access;
            contains (keys_ref) ([rows! { row (h) }[1]])
        } => if (mini_linq::zone_keys! {
            storage ::cq_storage; iterator ::mini_linq::stream_access; work (observed_work);
            contains (keys_ref) ([rows! { row (h) }[1]])
        });
    };
    assert_eq!(selected, HashSet::from([[9, 1], [7, 3]]));
    assert_eq!(source_calls.get(), 1);
    let observed = work.take();
    assert_eq!(observed.blocks_considered, 4);
    assert_eq!(observed.blocks_skipped, 1);
    assert_eq!(observed.blocks_opened, 3);
    assert_eq!(observed.exact_probes, 3); // The envelope includes key 2.
}

#[test]
fn witness_implementations_preserve_pairs_and_nested_build_stays_lazy() {
    let right = [[1_i64, 10], [1, 11], [2, 20]];
    let right = &right;
    let calls = Cell::new(0);
    let calls = &calls;
    macro_rules! build {
        ($implementation:ident) => {
            ::mini_linq::$implementation! {
                storage ::cq_storage; iterator ::mini_linq::stream_access;
                build {
                    witnesses(k, row) => (([k], row)) :-
                    R(k, row) => for (k, row) in ({
                        calls.set(calls.get() + 1);
                        right.iter().map(|row| (row[0], row))
                    });
                }
            }
        };
    }
    let hash = build!(hash_witnesses);
    assert_eq!(calls.get(), 1);
    calls.set(0);
    let nested = build!(nested_witnesses);
    assert_eq!(calls.get(), 0);
    let expected = vec![[1, 10], [1, 11]];
    let hash_rows: Vec<_> = mini_linq::hash_witnesses! {
        storage ::cq_storage; iterator ::mini_linq::stream_access; probe (&hash) ([1_i64])
    }
    .copied()
    .collect();
    let nested_rows: Vec<_> = mini_linq::nested_witnesses! {
        storage ::cq_storage; iterator ::mini_linq::stream_access; probe (&nested) ([1_i64])
    }
    .copied()
    .collect();
    assert_eq!(hash_rows, expected);
    assert_eq!(nested_rows, expected);
    assert_eq!(calls.get(), 1);
}

// Macro names inhabit textual scope even when generated identifiers use mixed
// site spans. User choices must remain callable when they resemble generated
// names, including choices whose names belong to another wrapper role.
mod same_role_aliases {
    macro_rules! __ml_provider0 {
        ($($source:tt)*) => { ::mini_linq::hash_rows! { $($source)* } };
    }
    macro_rules! __ml_access0 {
        ($($source:tt)*) => { ::mini_linq::exact_keys! { $($source)* } };
    }
    ::mini_linq::physical_plan! {
        storage ::cq_storage;
        access ::mini_linq::collect_access;
        iterator ::mini_linq::stream_access;
        materialize r#__ml_provider0;
        inputs { R => r#__ml_provider0; S => r#__ml_provider0; }
        scan ::mini_linq::scan_rows;
        key_provider r#__ml_access0;
        keys ::std::collections::HashSet;
        join ::mini_linq::hash_witnesses;
        pub struct ReservedNames;
        relation R(k:i64); relation S(k:i64);
        answer(k) :- R(k), S(k);
        relational {
            r = rename R {k -> k};
            s = rename S {k -> k};
            reduced = semijoin r with s;
            output reduced as answer(k);
        }
    }
}

mod cross_role_aliases {
    macro_rules! __ml_access0 {
        ($($source:tt)*) => { ::mini_linq::hash_rows! { $($source)* } };
    }
    macro_rules! __ml_provider0 {
        ($($source:tt)*) => { ::mini_linq::exact_keys! { $($source)* } };
    }
    macro_rules! __ml_provider1 {
        ($($source:tt)*) => { ::mini_linq::collect_access! { $($source)* } };
    }
    ::mini_linq::physical_plan! {
        storage ::cq_storage;
        access __ml_provider1;
        iterator ::mini_linq::stream_access;
        materialize __ml_access0;
        inputs { R => __ml_access0; S => __ml_access0; }
        scan ::mini_linq::scan_rows;
        key_provider __ml_provider0;
        keys ::std::collections::HashSet;
        join ::mini_linq::hash_witnesses;
        pub struct CrossRoleNames;
        relation R(k:i64,x:i64); relation S(k:i64,y:i64);
        answer(k,x,y) :- R(k,x), S(k,y);
        relational {
            r = rename R {k -> k,x -> x};
            s = rename S {k -> k,y -> y};
            reduced = semijoin r with s;
            joined = natural_join reduced with s;
            output joined as answer(k,x,y);
        }
    }
}

#[test]
fn configured_aliases_do_not_shadow_selected_macro_names() {
    let size = NonZeroUsize::new(2).unwrap();
    let left = HashSet::from([[1], [2], [3]]);
    let right = HashSet::from([[1], [3]]);
    let expected = vec![(1,), (3,)];
    assert_eq!(
        same_role_aliases::ReservedNames::run(&left, &right, size),
        expected
    );
    let (observed, stats) = same_role_aliases::ReservedNames::run_with_stats(&left, &right, size);
    assert_eq!(observed, expected);
    assert_eq!(stats.steps.last().unwrap().2.exact_probes, left.len());

    let left = HashSet::from([[1, 10], [2, 20]]);
    let right = HashSet::from([[1, 7], [1, 8], [9, 90]]);
    let expected = vec![(1, 10, 7), (1, 10, 8)];
    assert_eq!(
        cross_role_aliases::CrossRoleNames::run(&left, &right, size),
        expected
    );
    assert_eq!(
        cross_role_aliases::CrossRoleNames::run_with_stats(&left, &right, size).0,
        expected
    );
}

#[test]
fn streaming_access_preserves_support_without_allocating_a_distinct_set() {
    let stream: Vec<_> = mini_linq::stream_access! {
        out(k) => ([k]) :- R(k,_v) => for (k,_v) in ([(1_i64,10), (1,11), (2,20)]);
    }
    .collect();
    let support: HashSet<_> = mini_linq::collect_access! {
        out(k) => ([k]) :- R(k,_v) => for (k,_v) in ([(1_i64,10), (1,11), (2,20)]);
    };
    assert_eq!(stream, vec![[1], [1], [2]]);
    assert_eq!(stream.into_iter().collect::<HashSet<_>>(), support);
}
