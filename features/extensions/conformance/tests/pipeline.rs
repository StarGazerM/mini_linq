#![recursion_limit = "512"]
#![allow(non_snake_case)]
use cq_testkit::Rows;

#[test]
fn all_other_surfaces_forward_nested_core_blocks() {
    let A = [(1_i32,), (2,)];
    let B = [(2_i32,), (3,)];
    let C = [(4_i32,)];
    let rows: Rows = mini_linq::compile! {
        pipeline { surface = [cq_ext_aggregate::aggregate, cq_ext_recursion::recursion, cq_ext_negation::negation, cq_ext_correlated::correlated]; }
        struct Q; relation A(x:i32); relation B(x:i32); relation C(x:i32);
        answer(x) :- {{A(x)} | {B(x)}} else {C(x)};
    }.map(|(x,)|vec![*x]).collect();
    assert_eq!(rows, Rows::from([vec![1], vec![2], vec![3]]));
}

#[test]
fn every_surface_forwards_plain_cq_and_parameters() {
    let A = [(1,), (2,), (3,)];
    let limit = 1;
    let rows: Rows = mini_linq::compile! {
        pipeline { surface = [cq_ext_aggregate::aggregate, cq_ext_recursion::recursion, cq_ext_negation::negation, cq_ext_correlated::correlated]; }
        struct Plain; relation A(x:i32); answer(x) :- A(x), if *x > limit;
    }.map(|(x,)| vec![*x]).collect();
    assert_eq!(rows, [vec![2], vec![3]].into());
}

#[test]
fn direct_surface_packet_carries_arguments_through_stacked_union_negation() {
    let A = [(1,), (2,)];
    let B = [(2,), (3,)];
    let S = [(2,)];
    let rows: Vec<_> = cq_ext_negation::negation! {
        pipeline {
            mini_linq::passes::plan_written,
            mini_linq::passes::index_demands,
            mini_linq::passes::lower,
            mini_linq::passes::select,
            cq_ext_test_passes::repeated(twice)
        };
        struct Direct; relation A(x:i32); relation B(x:i32); relation S(x:i32);
            answer(x) :- { A(x) } | { B(x) }, neg!{S(x)};
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.into_iter().collect::<std::collections::BTreeSet<_>>(),
        [1, 3].into()
    );
}

#[test]
fn aggregate_nested_query_uses_the_callers_printer_and_deduplicates_its_witnesses() {
    let Sale = [(1, 1, 10), (1, 2, 10)];
    let groups = mini_linq::compile! {
        pipeline {
            surface = [cq_ext_negation::negation, cq_ext_aggregate::aggregate];
            print = cq_ext_test_passes::repeated(twice);
        }
        aggregate! { by(d); n=count(); s=sum(a);
            struct G; relation Sale(d:i32,id:i32,a:i32); rows(d,id,a) :- Sale(d,id,a);
        }
    };
    let rows: Rows = groups
        .iter()
        .flat_map(|((d,), values)| values.iter().map(move |&(n, s)| vec![*d, n as i32, s]))
        .collect();
    assert_eq!(rows, [vec![1, 2, 20]].into());
}

#[test]
fn recursive_seed_and_step_queries_use_the_callers_printer() {
    let Seed = [(1,)];
    let Edge = [(1, 2), (2, 3), (3, 1)];
    let rows: Vec<_> = mini_linq::compile! {
        pipeline {
            surface = [cq_ext_recursion::recursion];
            print = cq_ext_test_passes::repeated(twice);
        }
        recursion! { struct R; relation Seed(x:i32); relation Edge(x:i32,y:i32); relation reach(x:i32);
            reach(*x) :- Seed(x); reach(*y) :- reach(x), Edge(x,y);
        }
    }.collect();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.into_iter().map(|(x,)| vec![x]).collect::<Rows>(),
        [vec![1], vec![2], vec![3]].into()
    );
}

#[test]
fn correlated_inner_and_outer_queries_use_the_callers_printer() {
    let A = [(1,), (2,)];
    let S = [(1, 7), (1, 7)];
    let rows: Vec<_> = mini_linq::compile! {
        pipeline {
            surface = [cq_ext_correlated::correlated];
            print = cq_ext_test_passes::repeated(twice);
        }
        struct C; relation A(x:i32); relation S(x:i32,y:i32); relation answer(x:i32,v:i32);
        answer(*x,v) :- A(x), scalar!{v:i32=(-1); select(*y); S(x,y)};
    }
    .collect();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.into_iter().map(|(x, v)| vec![x, v]).collect::<Rows>(),
        [vec![1, 7], vec![2, -1]].into()
    );
}

#[test]
fn aggregate_keeps_owned_leaf_rows_alive_and_evaluates_them_once() {
    let calls = std::cell::Cell::new(0);
    let groups = mini_linq::compile! {
        pipeline { surface = [cq_ext_aggregate::aggregate]; }
        aggregate! {
            by(d); n=count(); s=sum(a); struct Owned;
            relation Sale(d:String,id:i32,a:i32) = ({
                calls.set(calls.get()+1);
                vec![(String::from("a"),1,10), (String::from("a"),2,-5)]
            });
            rows(d,id,a) :- Sale(d,id,a);
        }
    };
    assert_eq!(calls.get(), 1);
    let rows: Vec<_> = groups
        .iter()
        .flat_map(|((d,), values)| values.iter().map(move |&(n, s)| (d.as_str(), n, s)))
        .collect();
    assert_eq!(rows, [("a", 2, 5)]);
}
