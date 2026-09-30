#![recursion_limit = "512"]
#![allow(non_snake_case)]
use std::{cell::Cell, collections::BTreeSet};

macro_rules! policy { ($($ix:tt)*) => { ::mini_linq::lower_physical! { $($ix)* policy {} } }; }
pub(crate) use policy as physical_policy;
#[allow(unused_imports)]
mod mini_linq {
    pub(crate) use super::physical_policy;
    pub(crate) use ::cq_state_providers::default_indexes as index_provider;
    pub(crate) use ::cq_state_providers::hash_distinct as distinct_state;
    pub(crate) use ::mini_linq::mini_linq as union_hole;
    pub(crate) use ::mini_linq::mini_linq as negation_hole;
    pub(crate) use ::mini_linq::mini_linq as aggregate_hole;
    pub(crate) use ::mini_linq::mini_linq as recursion_hole;
    pub(crate) use ::mini_linq::mini_linq as correlated_hole;
    pub(crate) use ::mini_linq::{
        cq_to_ra, index_demands as choose_index_policy, select_structures, structures_to_rust,
    };
}

#[test]
fn union_is_lazy_across_disjuncts_and_retains_borrowed_non_copy_columns() {
    let A = [(String::from("a"),)];
    let B = [(String::from("b"),)];
    let first = Cell::new(0);
    let second = Cell::new(0);
    let visit_first = |_: &String| {
        first.set(first.get() + 1);
        true
    };
    let visit_second = |_: &String| {
        second.set(second.get() + 1);
        true
    };
    let mut result = cq_ext_union::union! {
        struct Lazy; relation A(x:String); relation B(x:String);
        answer(x) :- A(x), if visit_first(x); answer(x) :- B(x), if visit_second(x);
    };
    assert_eq!((first.get(), second.get()), (0, 0));
    let (value,) = result.next().unwrap();
    assert!(std::ptr::eq(value, &A[0].0));
    assert_eq!((first.get(), second.get()), (1, 0));
    assert_eq!(result.count(), 1);
    assert_eq!(second.get(), 1);
}

#[test]
fn union_leaf_expressions_run_once_for_all_disjuncts() {
    let calls = Cell::new(0);
    let result: BTreeSet<_> = cq_ext_union::union! {
        struct Leaves;
        relation A(x:i32) = ({calls.set(calls.get()+1); [(1,),(2,),(2,)]});
        relation answer(x:i32);
        answer(*x) :- A(x); answer(*x) :- A(x), if *x > 1;
    }
    .collect();
    assert_eq!(result, [(1,), (2,)].into());
    assert_eq!(calls.get(), 1);
}

#[test]
fn negative_projection_handles_global_absence_repeated_keys_and_empty_inner() {
    let A = [(1,), (2,)];
    let S = [(1, 1), (2, 3)];
    let absent: Vec<_> = cq_ext_negation::negation! {
        struct Repeated; relation A(x:i32); relation S(x:i32,y:i32); relation answer(x:i32);
        answer(*x) :- A(x), neg!{S(x,x)};
    }
    .collect();
    assert_eq!(absent, [(2,)]);
    let result: Vec<_> = cq_ext_negation::negation! {
        struct Global; relation A(x:i32); relation S(x:i32,y:i32); relation answer(x:i32);
        answer(*x) :- A(x), neg!{S(_,_)};
    }
    .collect();
    assert!(result.is_empty());
    let S: [(i32, i32); 0] = [];
    let result: BTreeSet<_> = cq_ext_negation::negation! {
        struct GlobalEmpty; relation A(x:i32); relation S(x:i32,y:i32); relation answer(x:i32);
        answer(*x) :- A(x), neg!{S(_,_)};
    }
    .collect();
    assert_eq!(result, [(1,), (2,)].into());
}

#[test]
fn negative_and_correlated_computed_heads_retain_set_semantics() {
    let A = [(1, 10), (1, 11), (2, 20)];
    let S = [(2, 99)];
    let negative: Vec<_> = cq_ext_negation::negation! {
        struct Projection; relation A(x:i32,payload:i32); relation S(x:i32,y:i32); relation answer(x:i32);
        answer(*x) :- A(x,payload), neg!{S(x,_)};
    }.collect();
    assert_eq!(negative, [(1,)]);
    let correlated: Vec<_> = cq_ext_correlated::correlated! {
        struct Projection; relation A(x:i32,payload:i32); relation answer(x:i32);
        answer(*x) :- A(x,payload), if *x < 2;
    }
    .collect();
    assert_eq!(correlated, [(1,)]);
}

#[test]
fn negation_shares_once_evaluated_leaves_with_its_inner_hole() {
    let calls = Cell::new(0);
    let result: Vec<_> = cq_ext_negation::negation! {
        struct Leaves;
        relation A(x:i32) = ({calls.set(calls.get()+1);[(1,),(2,)]});
        relation S(x:i32,y:i32) = ({calls.set(calls.get()+1);[(1,10)]});
        relation answer(x:i32); answer(*x) :- A(x), neg!{S(x,_)};
    }
    .collect();
    assert_eq!(result, [(2,)]);
    assert_eq!(calls.get(), 2);
}

#[test]
fn aggregate_grouped_output_is_a_native_leading_key_host() {
    let Sale = [(1, 1, 10), (1, 2, -5), (2, 3, 7), (1, 1, 10)];
    let Keys = [(1,)];
    let Totals = cq_ext_aggregate::aggregate! {
        by(d); n=count(); s=sum(a); struct Stats; relation Sale(d:i32,id:i32,a:i32);
        rows(d,id,a) :- Sale(d,id,a);
    };
    let result: Vec<_> = ::mini_linq::mini_linq! {
        struct Reuse; relation Keys(d:i32); relation Totals(d:i32,n:usize,s:i32); relation answer(d:i32,n:usize,s:i32);
        answer(*d,*n,*s) :- Keys(d), Totals(d,n,s);
    }.collect();
    assert_eq!(result, [(1, 2, 5)]);
}

#[test]
fn aggregate_keys_can_own_non_copy_values() {
    let Sale = [
        (String::from("a"), 1, -5),
        (String::from("a"), 2, 10),
        (String::from("b"), 3, 7),
    ];
    let result = cq_ext_aggregate::aggregate! {
        by(d); lo=min(a); hi=max(a); struct Stats; relation Sale(d:String,id:i32,a:i32);
        rows(d,id,a) :- Sale(d,id,a);
    };
    let groups: BTreeSet<_> = result
        .iter()
        .flat_map(|((d,), payload)| payload.iter().map(move |&(lo, hi)| (d.as_str(), lo, hi)))
        .collect();
    assert_eq!(groups, [("a", -5, 10), ("b", 7, 7)].into());
}

#[test]
fn generated_fold_locals_do_not_capture_row_columns() {
    let Sale = [(1, 3), (1, 7)];
    let result = cq_ext_aggregate::aggregate! {
        by(d); total=sum(entry); struct Names; relation Sale(d:i32,entry:i32);
        rows(d,entry) :- Sale(d,entry);
    };
    let actual: Vec<_> = result
        .iter()
        .flat_map(|((d,), payload)| payload.iter().map(move |&(sum,)| (*d, sum)))
        .collect();
    assert_eq!(actual, [(1, 10)]);
}

#[test]
fn raw_identifiers_preserve_logical_symbols_across_extension_boundaries() {
    let A = [(1,), (2,)];
    let S = [(1, 1)];
    let correlated: Vec<_> = cq_ext_correlated::correlated! {
        struct Raw; relation r#A(x:i32); relation r#S(a:i32,b:i32); relation r#answer(x:i32);
        answer(*x) :- A(x), exists!{S(r#x,x)};
    }
    .collect();
    assert_eq!(correlated, [(1,)]);
    let negative: Vec<_> = cq_ext_negation::negation! {
        struct Raw; relation r#A(x:i32); relation r#S(a:i32,b:i32); relation r#answer(x:i32);
        answer(*x) :- A(x), neg!{S(r#x,x)};
    }
    .collect();
    assert_eq!(negative, [(2,)]);
    let Sale = [(1, 3), (1, 7), (2, 9)];
    let grouped = cq_ext_aggregate::aggregate! {
        by(r#d); total=sum(r#a); struct Raw; relation Sale(d:i32,a:i32); rows(d,a) :- Sale(d,a);
    };
    let actual: BTreeSet<_> = grouped
        .iter()
        .flat_map(|((d,), payload)| payload.iter().map(move |&(sum,)| (*d, sum)))
        .collect();
    assert_eq!(actual, [(1, 10), (2, 9)].into());
    let Seed = [(0,)];
    let Edge = [(0, 1), (1, 2)];
    let recursive: BTreeSet<_> = cq_ext_recursion::recursion! {
        struct Raw; relation Seed(x:i32); relation Edge(x:i32,y:i32); relation r#reach(x:i32);
        reach(*x) :- Seed(x); r#reach(*y) :- r#reach(x), Edge(x,y);
    }
    .collect();
    assert_eq!(recursive, [(0,), (1,), (2,)].into());
}

#[test]
fn recursion_unions_seeds_and_multiple_linear_steps_and_stops_on_cycles() {
    let Seed = [(0,)];
    let Extra = [(2,)];
    let Edge = [(0, 1), (1, 2), (2, 1), (2, 3)];
    let rounds = Cell::new(0);
    let deltas = Cell::new(0);
    let result: BTreeSet<_> = cq_ext_recursion::recursion! {
        observe=(|_:usize,n:usize|{rounds.set(rounds.get()+1);deltas.set(deltas.get()+n);});
        struct Multi; relation Seed(x:i32); relation Extra(x:i32); relation Edge(x:i32,y:i32); relation reach(x:i32);
        reach(*x) :- Seed(x); reach(*x) :- Extra(x);
        reach(*y) :- reach(x), Edge(x,y); reach(*y) :- reach(x), Edge(y,x);
    }.collect();
    assert_eq!(result, [(0,), (1,), (2,), (3,)].into());
    assert_eq!(rounds.get(), 2);
    assert_eq!(deltas.get(), 4);
}

#[test]
fn recursion_owns_non_copy_delta_rows_and_hoists_leaves_once() {
    let calls = Cell::new(0);
    let result: BTreeSet<_> = cq_ext_recursion::recursion! {
        struct Owned;
        relation Seed(x:String) = ({calls.set(calls.get()+1);[(String::from("a"),)]});
        relation Edge(x:String,y:String) = ({calls.set(calls.get()+1);[(String::from("a"),String::from("b")),(String::from("b"),String::from("a"))]});
        relation reach(x:String);
        reach((*x).clone()) :- Seed(x); reach((*y).clone()) :- reach(x), Edge(x,y);
    }.collect();
    assert_eq!(result, [(String::from("a"),), (String::from("b"),)].into());
    assert_eq!(calls.get(), 2);
}

#[test]
fn correlated_exists_evaluates_with_each_outer_value_and_borrowed_parameters() {
    let A = [(1,), (2,), (3,)];
    let S = [(1, 5), (2, 1), (9, 9)];
    let calls = Cell::new(0);
    let parameter = 2;
    let observe = |_: &i32| {
        calls.set(calls.get() + 1);
        true
    };
    let result: Vec<_> = cq_ext_correlated::correlated! {
        struct PerRow; relation A(x:i32); relation S(x:i32,y:i32); relation answer(x:i32);
        answer(*x) :- A(x), exists!{S(k,y), if observe(x), if k == x, if *y > parameter};
    }
    .collect();
    assert_eq!(result, [(1,)]);
    assert!(calls.get() >= 3);
}

#[test]
fn correlated_environment_keeps_borrowed_non_copy_hosts_and_parameters() {
    let A = [(String::from("apple"),), (String::from("pear"),)];
    let S = [(String::from("apple"),)];
    let prefix = String::from("app");
    let prefix = &prefix;
    let result: Vec<_> = cq_ext_correlated::correlated! {
        struct Borrowed; relation A(x:String); relation S(y:String);
        answer(x) :- A(x), exists!{S(y), if y.starts_with(prefix.as_str()), if x.as_str() == y.as_str()};
    }.collect();
    assert_eq!(result.len(), 1);
    assert!(std::ptr::eq(result[0].0, &A[0].0));
}

#[test]
fn scalar_default_duplicate_rows_and_cardinality_error() {
    let A = [(1,), (2,)];
    let S = [(1, 7), (1, 7)];
    let result: BTreeSet<_> = cq_ext_correlated::correlated! {
        struct Scalar; relation A(x:i32); relation S(x:i32,y:i32); relation answer(x:i32,v:i32);
        answer(*x,v) :- A(x), scalar!{v:i32=(-1); select(*y); S(x,y)};
    }
    .collect();
    assert_eq!(result, [(1, 7), (2, -1)].into());
    let S = [(1, 7), (1, 8)];
    let error = std::panic::catch_unwind(|| {
        cq_ext_correlated::correlated! {
        struct ScalarError; relation A(x:i32); relation S(x:i32,y:i32); relation answer(x:i32,v:i32);
        answer(*x,v) :- A(x), scalar!{v:i32=(0); select(*y); S(x,y)};
    }.collect::<Vec<_>>()
    });
    assert!(error.is_err());
}

#[test]
fn scalar_cardinality_counts_distinct_selected_values() {
    let A = [(1,)];
    let S = [(1, 7, 10), (1, 7, 11)];
    let result: Vec<_> = cq_ext_correlated::correlated! {
        struct Projected; relation A(x:i32); relation S(x:i32,y:i32,id:i32); relation answer(x:i32,v:i32);
        answer(*x,v) :- A(x), scalar!{v:i32=(0); select(*y); S(x,y,id)};
    }.collect();
    assert_eq!(result, [(1, 7)]);
}

#[test]
fn correlated_leaf_inputs_are_evaluated_once_before_outer_iteration() {
    let calls = Cell::new(0);
    let result: BTreeSet<_> = cq_ext_correlated::correlated! {
        struct Leaves;
        relation A(x:i32) = ({calls.set(calls.get()+1);[(1,),(2,)]});
        relation S(x:i32,y:i32) = ({calls.set(calls.get()+1);[(1,7),(2,8)]});
        relation answer(x:i32); answer(*x) :- A(x), exists!{S(x,y)};
    }
    .collect();
    assert_eq!(result, [(1,), (2,)].into());
    assert_eq!(calls.get(), 2);
}
