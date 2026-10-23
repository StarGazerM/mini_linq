#![recursion_limit = "512"]
#![allow(non_snake_case)]
use std::collections::BTreeSet;

use cq_testkit::{Rows, harness::check_refinement};

#[test]
fn anti_membership_matches_an_independent_projection_oracle() {
    let A = [(1,), (2,), (2,), (3,)];
    let S = [(2, 20), (2, 21), (9, 90)];
    let expected: Rows = [vec![1], vec![3]].into();
    let result = check_refinement(
        || {
            A.iter()
                .filter(|(x,)| !S.iter().any(|(key, _)| key == x))
                .map(|&(x,)| vec![x])
                .collect::<Rows>()
        },
        || {
            mini_linq::compile! {
                pipeline {}
                struct N; relation A(x:i32); relation S(x:i32,y:i32);
                answer(x) :- A(x), !S(x,_);
            }
            .map(|(x,)| vec![*x])
            .collect::<Rows>()
        },
    )
    .unwrap();
    assert_eq!(
        result.target,
        cq_testkit::harness::Outcome::Returned(expected)
    );
}

fn aggregate_reference(sales: &[(i32, i32, i32)]) -> Rows {
    let rows: BTreeSet<_> = sales.iter().copied().collect();
    let keys: BTreeSet<_> = rows.iter().map(|&(d, _, _)| d).collect();
    keys.into_iter()
        .map(|key| {
            let amounts: Vec<_> = rows
                .iter()
                .filter(|(d, _, _)| *d == key)
                .map(|&(_, _, a)| a)
                .collect();
            vec![
                key,
                amounts.len() as i32,
                amounts.iter().sum(),
                *amounts.iter().min().unwrap(),
                *amounts.iter().max().unwrap(),
            ]
        })
        .collect()
}

#[test]
fn aggregate_matches_authored_set_folds_and_empty_input() {
    for Sale in [vec![(1, 1, 10), (1, 2, -5), (2, 3, 7), (1, 1, 10)], vec![]] {
        check_refinement(
            || aggregate_reference(&Sale),
            || {
                let groups = mini_linq::compile! {
                    pipeline { surface = [cq_ext_aggregate::aggregate]; }
                    aggregate! {
                        by(d); n=count(); s=sum(a); lo=min(a); hi=max(a);
                        struct G; relation Sale(d:i32,id:i32,a:i32); rows(d,id,a) :- Sale(d,id,a);
                    }
                };
                groups
                    .iter()
                    .flat_map(|((d,), measures)| {
                        measures
                            .iter()
                            .map(move |&(n, s, lo, hi)| vec![*d, n as i32, s, lo, hi])
                    })
                    .collect::<Rows>()
            },
        )
        .unwrap();
    }
    assert_eq!(
        aggregate_reference(&[(1, 1, 10), (1, 2, -5), (2, 3, 7), (1, 1, 10)]),
        [vec![1, 2, 5, -5, 10], vec![2, 1, 7, 7, 7]].into()
    );
}

fn paths(edges: &[(i32, i32)]) -> Rows {
    let starts: BTreeSet<_> = edges.iter().map(|&(x, _)| x).collect();
    let mut answers = Rows::new();
    for start in starts {
        let mut pending: Vec<_> = edges
            .iter()
            .filter(|(x, _)| *x == start)
            .map(|&(_, y)| y)
            .collect();
        let mut visited = BTreeSet::new();
        while let Some(node) = pending.pop() {
            if visited.insert(node) {
                answers.insert(vec![start, node]);
                pending.extend(edges.iter().filter(|(x, _)| *x == node).map(|&(_, y)| y));
            }
        }
    }
    answers
}

#[test]
fn linear_recursion_matches_graph_search_with_cycles_and_empty_seeds() {
    for Edge in [vec![(1, 2), (2, 3), (3, 2), (1, 2)], vec![]] {
        check_refinement(
            || paths(&Edge),
            || {
                mini_linq::compile! {
                    pipeline { surface = [cq_ext_recursion::recursion]; }
                    recursion! {
                        struct R; relation Edge(x:i32,y:i32); relation reach(x:i32,y:i32);
                        reach(*x,*y) :- Edge(x,y); reach(*x,*z) :- reach(x,y), Edge(y,z);
                    }
                }
                .map(|(x, y)| vec![x, y])
                .collect::<Rows>()
            },
        )
        .unwrap();
    }
    assert_eq!(
        paths(&[(1, 2), (2, 3), (3, 2)]),
        [
            vec![1, 2],
            vec![1, 3],
            vec![2, 2],
            vec![2, 3],
            vec![3, 2],
            vec![3, 3]
        ]
        .into()
    );
}

#[test]
fn correlated_exists_and_parameters_match_nested_loop_answers() {
    let A = [(1,), (2,), (3,)];
    let S = [(1, 5), (1, 5), (2, 1), (9, 9)];
    let limit = 2;
    let result = check_refinement(
        || {
            A.iter()
                .filter(|(x,)| S.iter().any(|(key, y)| key == x && *y > limit))
                .map(|&(x,)| vec![x])
                .collect::<Rows>()
        },
        || {
            mini_linq::compile! {
                pipeline { surface = [cq_ext_correlated::correlated]; }
                struct C; relation A(x:i32); relation S(x:i32,y:i32);
                answer(x) :- A(x), exists!{S(x,y), if *y > limit};
            }
            .map(|(x,)| vec![*x])
            .collect::<Rows>()
        },
    )
    .unwrap();
    assert_eq!(
        result.target,
        cq_testkit::harness::Outcome::Returned([vec![1]].into())
    );
}

fn scalar_reference(outer: &[(i32,)], inner: &[(i32, i32)]) -> Rows {
    outer
        .iter()
        .map(|&(x,)| {
            let values: BTreeSet<_> = inner
                .iter()
                .filter(|(key, _)| *key == x)
                .map(|&(_, y)| y)
                .collect();
            assert!(values.len() <= 1);
            vec![x, values.into_iter().next().unwrap_or(-1)]
        })
        .collect()
}

#[test]
fn scalar_refines_the_written_cardinality_check_and_default() {
    let A = [(1,), (2,)];
    for S in [vec![(1, 7), (1, 7)], vec![], vec![(1, 7), (1, 8)]] {
        let result = check_refinement(
            || scalar_reference(&A, &S),
            || {
                mini_linq::compile! {
                pipeline { surface = [cq_ext_correlated::correlated]; }
                struct C; relation A(x:i32); relation S(x:i32,y:i32); relation answer(x:i32,v:i32);
                answer(*x,v) :- A(x), scalar!{v:i32=(-1); select(*y); S(x,y)};
            }.map(|(x,y)| vec![x,y]).collect::<Rows>()
            },
        )
        .unwrap();
        if S.len() == 2 && S[0] != S[1] {
            assert!(matches!(
                result.target,
                cq_testkit::harness::Outcome::Panicked(_)
            ));
        }
    }
    assert_eq!(
        scalar_reference(&A, &[(1, 7)]),
        [vec![1, 7], vec![2, -1]].into()
    );
}

#[test]
fn stacked_surfaces_forward_core_union_and_negation_matching_authored_answers() {
    let A = [(1,), (2,), (2,)];
    let B = [(2,), (3,)];
    let S = [(2,)];
    let result = check_refinement(
        || A.iter().chain(&B).filter(|(x,)| !S.contains(&(*x,))).map(|&(x,)| vec![x]).collect::<Rows>(),
        || mini_linq::compile! {
            pipeline { surface = [cq_ext_aggregate::aggregate, cq_ext_recursion::recursion, cq_ext_correlated::correlated]; }
            struct Stack; relation A(x:i32); relation B(x:i32); relation S(x:i32);
                answer(x) :- { A(x) } | { B(x) }, !S(x);
        }.map(|(x,)| vec![*x]).collect::<Rows>(),
    ).unwrap();
    assert_eq!(
        result.target,
        cq_testkit::harness::Outcome::Returned([vec![1], vec![3]].into())
    );
}

#[test]
fn partial_rust_preserves_written_order_refinement_in_surface_queries() {
    let A = [(0,), (1,), (2,)];
    let positive = |x: &i32| 1 / *x > 0;
    check_refinement(
        || {
            A.iter()
                .filter(|(x,)| *x > 0)
                .filter(|(x,)| positive(x))
                .map(|&(x,)| vec![x])
                .collect::<Rows>()
        },
        || {
            mini_linq::compile! {
                pipeline {}
                struct Partial; relation A(x:i32);
                    answer(x) :- A(x), if *x > 0, if positive(x);
            }
            .map(|(x,)| vec![*x])
            .collect::<Rows>()
        },
    )
    .unwrap();
}
