//! Executable checks of contextual assumptions and operand evaluation.
use std::cell::Cell;
use std::collections::{BTreeSet, HashSet};
use std::num::NonZeroUsize;

use cq_storage::{BlockStore, Counters, RangeSummary, ZoneSummary};

fn observe<'a, T>(count: &Cell<usize>, value: &'a T) -> &'a T {
    count.set(count.get() + 1);
    value
}

#[test]
fn key_projections_evaluate_their_only_operand_and_work_once() {
    let right = HashSet::from([[2], [7]]);
    let right_calls = Cell::new(0);
    let work_calls = Cell::new(0);
    let work = Counters::default();
    let observed_work = &work;
    macro_rules! rows {
        ($($operation:tt)*) => {
            ::mini_linq::hash_rows! { storage ::cq_storage; work (observed_work); $($operation)* }
        };
    }
    macro_rules! build {
        ($recipe:ident) => {
            ::mini_linq::$recipe! {
                storage ::cq_storage;
                access ::mini_linq::collect_access;
                keys ::std::collections::HashSet;
                work (observe(&work_calls, &work));
                q = project r keep {b}; {
                    r(b) => rows { observe(&right_calls, &right) };
                }
            }
        };
    }

    let exact = build!(exact_semijoin);
    assert_eq!(exact.0, right);
    assert_eq!((right_calls.get(), work_calls.get()), (1, 1));
    assert_eq!(work.take().index_rows, 2);

    right_calls.set(0);
    work_calls.set(0);
    let zone = build!(zone_semijoin);
    assert_eq!(zone.0, right);
    assert!(RangeSummary::may_match(&zone.1, &[(0, 0), (2, 2)], &[1]));
    assert!(!RangeSummary::may_match(&zone.1, &[(0, 0), (8, 8)], &[1]));
    assert_eq!((right_calls.get(), work_calls.get()), (1, 1));
    assert_eq!(work.take().index_rows, 2);
}

#[test]
fn exact_reduction_uses_projected_resource_and_preserves_the_left_layout() {
    let left = HashSet::from([[9, 2, 1], [8, 3, 1], [7, 2, 4]]);
    let resource = (HashSet::from([[1, 2]]),);
    let left_calls = Cell::new(0);
    let resource_calls = Cell::new(0);
    let work_calls = Cell::new(0);
    let work = Counters::default();
    let observed_work = &work;
    macro_rules! rows {
        ($($operation:tt)*) => {
            ::mini_linq::hash_rows! { storage ::cq_storage; work (observed_work); $($operation)* }
        };
    }
    let result = mini_linq::exact_semijoin! {
        storage ::cq_storage;
        access ::mini_linq::collect_access;
        work (observe(&work_calls, &work));
        reduced = semijoin l with q; {
            l(z,b,a) => rows { observe(&left_calls, &left) };
            q(a,b) { observe(&resource_calls, &resource) };
        }
    };
    assert_eq!(result, HashSet::from([[9, 2, 1]]));
    assert_eq!(
        (left_calls.get(), resource_calls.get(), work_calls.get()),
        (1, 1, 1)
    );
    let observed = work.take();
    assert_eq!(observed.rows_visited, 3);
    assert_eq!(observed.exact_probes, 3);
}

#[test]
fn zone_reduction_uses_projected_resource_and_prunes_before_exact_probing() {
    let store = BlockStore::from_rows(
        [[9, 2, 1], [8, 3, 1], [7, 2, 4]],
        NonZeroUsize::new(1).unwrap(),
    );
    let left = mini_linq::block_rows! { storage ::cq_storage; input (&store) };
    let mut summary = ZoneSummary::<2>::default();
    RangeSummary::observe(&mut summary, &[1, 2]);
    let resource = (HashSet::from([[1, 2]]), summary);
    let left_calls = Cell::new(0);
    let resource_calls = Cell::new(0);
    let work_calls = Cell::new(0);
    let work = Counters::default();
    let observed_work = &work;
    macro_rules! rows {
        ($($operation:tt)*) => {
            ::mini_linq::block_rows! { storage ::cq_storage; work (observed_work); $($operation)* }
        };
    }
    let result = mini_linq::zone_semijoin! {
        storage ::cq_storage;
        access ::mini_linq::collect_access;
        work (observe(&work_calls, &work));
        reduced = semijoin l with q; {
            l(z,b,a) => rows { observe(&left_calls, &left) };
            q(a,b) { observe(&resource_calls, &resource) };
        }
    };
    assert_eq!(
        (left_calls.get(), resource_calls.get(), work_calls.get()),
        (1, 1, 1)
    );
    let observed = work.take();
    assert_eq!(observed.blocks_considered, 3);
    assert_eq!(observed.blocks_skipped, 2);
    assert_eq!(observed.blocks_opened, 1);
    assert_eq!(observed.rows_visited, 1);
    assert_eq!(observed.exact_probes, 1);
    let rows: HashSet<_> = mini_linq::block_rows! { storage ::cq_storage; scan (&result) }
        .copied()
        .collect();
    assert_eq!(rows, HashSet::from([[9, 2, 1]]));
}

#[test]
fn both_join_recipes_derive_raw_named_keys_from_noncanonical_layouts() {
    let left = HashSet::from([[9, 2, 1], [8, 3, 1]]);
    let right = BTreeSet::from([[1, 5, 2], [1, 6, 3], [4, 7, 2]]);
    let left_calls = Cell::new(0);
    let right_calls = Cell::new(0);
    let work_calls = Cell::new(0);
    let work = Counters::default();
    let observed_work = &work;
    macro_rules! left_rows {
        ($($operation:tt)*) => {
            ::mini_linq::hash_rows! { storage ::cq_storage; work (observed_work); $($operation)* }
        };
    }
    macro_rules! right_rows {
        ($($operation:tt)*) => {
            ::mini_linq::sorted_rows! { storage ::cq_storage; work (observed_work); $($operation)* }
        };
    }
    macro_rules! join {
        ($recipe:ident) => {
            ::mini_linq::$recipe! {
                storage ::cq_storage;
                access ::mini_linq::collect_access;
                work (observe(&work_calls, &work));
                joined = natural_join l with r#r; {
                    r#l(z,r#type,a) => left_rows { observe(&left_calls, &left) };
                    r(a,y,r#type) => right_rows { observe(&right_calls, &right) };
                }
            }
        };
    }
    let expected = HashSet::from([[1, 2, 5, 9], [1, 3, 6, 8]]);
    let hash = join!(hash_join);
    assert_eq!(hash, expected);
    assert_eq!(
        (left_calls.get(), right_calls.get(), work_calls.get()),
        (1, 1, 1)
    );
    left_calls.set(0);
    right_calls.set(0);
    work_calls.set(0);
    let nested = join!(nested_loop_join);
    assert_eq!(nested, expected);
    assert_eq!(
        (left_calls.get(), right_calls.get(), work_calls.get()),
        (1, 1, 1)
    );
}

#[test]
fn a_self_join_evaluates_the_single_binding_once_per_operand_use() {
    let rows = HashSet::from([[3, 2], [8, 1]]);
    let operand_calls = Cell::new(0);
    let work_calls = Cell::new(0);
    let work = Counters::default();
    let observed_work = &work;
    macro_rules! rows {
        ($($operation:tt)*) => {
            ::mini_linq::hash_rows! { storage ::cq_storage; work (observed_work); $($operation)* }
        };
    }
    macro_rules! join {
        ($recipe:ident) => {
            ::mini_linq::$recipe! {
                storage ::cq_storage;
                access ::mini_linq::collect_access;
                work (observe(&work_calls, &work));
                joined = natural_join r with r; {
                    r(z,x) => rows { observe(&operand_calls, &rows) };
                }
            }
        };
    }
    let expected = HashSet::from([[2, 3], [1, 8]]);
    assert_eq!(join!(hash_join), expected);
    assert_eq!((operand_calls.get(), work_calls.get()), (2, 1));
    operand_calls.set(0);
    work_calls.set(0);
    assert_eq!(join!(nested_loop_join), expected);
    assert_eq!((operand_calls.get(), work_calls.get()), (2, 1));
}

#[test]
fn configured_recipes_and_providers_run_without_instrumentation() {
    macro_rules! rows {
        ($($operation:tt)*) => {
            ::mini_linq::hash_rows! { storage ::cq_storage; $($operation)* }
        };
    }
    macro_rules! exact {
        ($($source:tt)*) => {
            ::mini_linq::exact_semijoin! {
                storage ::cq_storage;
                access ::mini_linq::collect_access;
                keys ::std::collections::HashSet;
                $($source)*
            }
        };
    }
    let left = HashSet::from([[9, 2], [8, 3]]);
    let right = HashSet::from([[2, 7], [2, 8]]);
    let resource = exact! {
        q = project r keep {b}; { r(b,y) => rows { right }; }
    };
    assert_eq!(resource.0, HashSet::from([[2]]));
    let result = exact! {
        reduced = semijoin l with q; {
            l(z,b) => rows { left };
            q(b) { resource };
        }
    };
    assert_eq!(result, HashSet::from([[9, 2]]));
    let joined = mini_linq::hash_join! {
        storage ::cq_storage;
        access ::mini_linq::collect_access;
        joined = natural_join l with r; {
            l(z,b) => rows { left };
            r(b,y) => rows { right };
        }
    };
    assert_eq!(joined, HashSet::from([[2, 7, 9], [2, 8, 9]]));
}

// The facade intercepts the real configured IX continuation. Its only terminal
// case accepts the explicit empty index overlay supplied by CP-Defer.
macro_rules! observe_composed {
    (@prefix [$($prefix:tt)*] indexes {}) => {
        pub fn observed_boundary() -> &'static str { "configured empty IndexRequirements" }
        ::mini_linq::composed! { $($prefix)* indexes {} }
    };
    (@prefix [$($prefix:tt)*] $next:tt $($remaining:tt)*) => {
        $crate::observe_composed! { @prefix [$($prefix)* $next] $($remaining)* }
    };
    (@prefix [$($prefix:tt)*]) => {
        compile_error!("physical_plan did not dispatch configured empty IndexRequirements");
    };
    ($($input:tt)*) => {
        $crate::observe_composed! { @prefix [] $($input)* }
    };
}
pub(crate) use observe_composed;

mod facade {
    pub(crate) use crate::observe_composed as composed;
    pub use ::mini_linq::*;
}

mod staged {
    use super::facade as mini_linq;

    ::mini_linq::physical_plan! {
        storage ::cq_storage;
        access ::mini_linq::collect_access;
        materialize ::mini_linq::hash_rows;
        inputs { R => ::mini_linq::hash_rows; }
        semijoin ::mini_linq::exact_semijoin;
        keys ::std::collections::HashSet;
        join ::mini_linq::hash_join;
        pub struct Dispatched;
        relation R(x:i64,y:i64);
        answer(x,y) :- R(x,y);
        relational {
            r = rename R {x -> x,y -> y};
            joined = natural_join r with r;
            output joined as answer(x,y);
        }
    }
}

#[test]
fn the_caller_alias_receives_configured_index_requirements_before_realization() {
    assert_eq!(
        staged::observed_boundary(),
        "configured empty IndexRequirements"
    );
    let input = HashSet::from([[2, 7], [1, 5]]);
    assert_eq!(
        staged::Dispatched::run(&input, NonZeroUsize::new(2).unwrap()),
        vec![(1, 5), (2, 7)]
    );
}

// Macro names inhabit textual scope even when generated identifiers use mixed
// site spans. User choices must remain callable when they resemble generated
// names, including choices whose names belong to another wrapper role.
mod same_role_aliases {
    macro_rules! __ml_provider0 {
        ($($source:tt)*) => { ::mini_linq::hash_rows! { $($source)* } };
    }
    macro_rules! __ml_recipe0 {
        ($($source:tt)*) => { ::mini_linq::exact_semijoin! { $($source)* } };
    }
    ::mini_linq::physical_plan! {
        storage ::cq_storage;
        access ::mini_linq::collect_access;
        materialize r#__ml_provider0;
        inputs { R => r#__ml_provider0; S => r#__ml_provider0; }
        semijoin r#__ml_recipe0;
        keys ::std::collections::HashSet;
        join ::mini_linq::hash_join;
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
    macro_rules! __ml_recipe0 {
        ($($source:tt)*) => { ::mini_linq::hash_rows! { $($source)* } };
    }
    macro_rules! __ml_provider0 {
        ($($source:tt)*) => { ::mini_linq::exact_semijoin! { $($source)* } };
    }
    macro_rules! __ml_provider1 {
        ($($source:tt)*) => { ::mini_linq::collect_access! { $($source)* } };
    }
    ::mini_linq::physical_plan! {
        storage ::cq_storage;
        access __ml_provider1;
        materialize __ml_recipe0;
        inputs { R => __ml_recipe0; S => __ml_recipe0; }
        semijoin __ml_provider0;
        keys ::std::collections::HashSet;
        join ::mini_linq::hash_join;
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
