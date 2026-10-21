//! Executable alternatives plus an independent typed naive reference.
//! These macros are dev-only: the ordinary compiler defaults remain untouched.
use cq_datalog_testkit::{Outcome, check_refinement};
use std::collections::HashSet;

macro_rules! reference {
    ($($source:tt)*) => { cq_datalog_test_passes::eval_source! { $($source)* } };
}
macro_rules! sequential {
    ($($source:tt)*) => { cq_datalog::datalog! { $($source)* }.collect::<HashSet<_>>() };
}
macro_rules! alternate {
    ($($source:tt)*) => {
        cq_datalog_test_passes::deduplicate_source! {
            pipeline {
                cq_datalog::passes::stratify,
                cq_datalog_test_passes::merge_strata,
                cq_datalog::passes::incremental,
                cq_datalog::passes::storage,
                cq_datalog_test_passes::alternate_storage,
                cq_datalog::passes::fork,
                cq_datalog::passes::print_rounds
            };
            $($source)*
        }.collect::<HashSet<_>>()
    };
}
macro_rules! closure {
    ($engine:ident, $edges:ident) => {
        $engine! {
            struct Closure;
            relation Edge(x:i32,y:i32) = (&$edges);
            relation Path(x:i32,y:i32);
            relation Mirror(x:i32,y:i32);
            output Path;
            Path(*x,*y) :- Edge(x,y);
            Path(*x,*y) :- Edge(x,y);
            Path(*x,*z) :- Path(x,y), Path(y,z);
            Mirror(*y,*x) :- Edge(x,y);
        }
    };
}

#[test]
fn nonlinear_duplicate_rules_independent_components_and_layouts() {
    ascent::ascent! {
        struct Oracle;
        relation edge(i32,i32);
        relation path(i32,i32);
        path(*x,*y) <-- edge(x,y);
        path(*x,*z) <-- path(x,y), path(y,z);
    }
    for edges in [
        vec![],
        vec![(0, 0)],
        vec![(0, 1), (1, 2), (0, 1)],
        vec![(0, 1), (1, 0), (3, 4)],
    ] {
        let mut oracle = Oracle {
            edge: edges.clone(),
            ..Default::default()
        };
        oracle.run();
        let expected: HashSet<_> = oracle.path.into_iter().collect();
        assert_eq!(closure!(reference, edges), expected);
        assert_eq!(closure!(sequential, edges), expected);
        check_refinement(|| closure!(reference, edges), || closure!(alternate, edges)).unwrap();
    }
}

macro_rules! negative {
    ($engine:ident, $nodes:ident, $edges:ident, $seeds:ident) => {
        $engine! {
            struct Negative;
            relation Node(x:i32) = (&$nodes);
            relation Edge(x:i32,y:i32) = (&$edges);
            relation Seed(x:i32) = (&$seeds);
            relation Reach(x:i32);
            relation Other(x:i32);
            relation Missing(x:i32);
            output Missing;
            Reach(*x) :- Seed(x);
            Reach(*y) :- Reach(x), Edge(x,y);
            Other(*x) :- Node(x);
            Missing(*x) :- Node(x), !Reach(x);
        }
    };
}

#[test]
fn completed_lower_strata_negation_agrees_with_ascent() {
    ascent::ascent! {
        struct Oracle;
        relation node(i32);
        relation edge(i32,i32);
        relation seed(i32);
        relation reach(i32);
        relation missing(i32);
        reach(*x) <-- seed(x);
        reach(*y) <-- reach(x), edge(x,y);
        missing(*x) <-- node(x), !reach(x);
    }
    let nodes = vec![(0,), (1,), (2,), (3,), (3,)];
    for edges in [vec![], vec![(0, 1), (1, 2)], vec![(0, 1), (1, 0)]] {
        let seeds = vec![(0,), (0,)];
        let mut oracle = Oracle {
            node: nodes.clone(),
            edge: edges.clone(),
            seed: seeds.clone(),
            ..Default::default()
        };
        oracle.run();
        let expected: HashSet<_> = oracle.missing.into_iter().collect();
        assert_eq!(negative!(reference, nodes, edges, seeds), expected);
        assert_eq!(negative!(sequential, nodes, edges, seeds), expected);
        check_refinement(
            || negative!(reference, nodes, edges, seeds),
            || negative!(alternate, nodes, edges, seeds),
        )
        .unwrap();
    }
}

macro_rules! computed {
    ($engine:ident, $rows:ident) => {
        $engine! {
            struct Computed;
            relation Input(label:String,n:i32) = (&$rows);
            relation Result(label:String,n:i32);
            relation Independent(n:i32);
            output Result;
            Result(label.clone(), next) :- Input(label,n), if *n < 3,
                let prefix = label.to_uppercase(), for extra in 0..2,
                let next = *n + extra, if !prefix.is_empty();
            Result(label.clone(), *n + 1) :- Result(label,n), if *n < 4;
            Independent(*n) :- Input(_,n);
        }
    };
}

#[test]
fn opaque_rust_strings_generators_and_computed_recursive_heads() {
    ascent::ascent! {
        struct Oracle;
        relation input(String,i32);
        relation result(String,i32);
        result(label.clone(), next) <-- input(label,n), if *n < 3,
            let prefix = label.to_uppercase(), for extra in 0..2,
            let next = *n + extra, if !prefix.is_empty();
        result(label.clone(), *n + 1) <-- result(label,n), if *n < 4;
    }
    let rows = vec![
        ("alpha".to_owned(), 0),
        ("alpha".to_owned(), 0),
        ("".to_owned(), 1),
        ("beta".to_owned(), 2),
    ];
    let mut oracle = Oracle {
        input: rows.clone(),
        ..Default::default()
    };
    oracle.run();
    let expected: HashSet<_> = oracle.result.into_iter().collect();
    assert_eq!(computed!(reference, rows), expected);
    assert_eq!(computed!(sequential, rows), expected);
    check_refinement(|| computed!(reference, rows), || computed!(alternate, rows)).unwrap();
}

#[test]
fn panic_is_bottom_not_an_empty_answer() {
    fn partial(n: &i32) -> bool {
        assert_ne!(*n, 0, "partial guard");
        true
    }
    let rows = [(0_i32,)];
    let result = check_refinement(
        || {
            reference! {
                struct Q; relation E(x:i32) = (&rows); relation R(x:i32); output R;
                R(x) :- E(x), if partial(x);
            }
        },
        HashSet::<(i32,)>::new,
    )
    .unwrap();
    assert!(matches!(result.reference, Outcome::Panicked(_)));
    assert!(
        check_refinement(
            || reference! {
                struct Q; relation E(x:i32) = (&rows); relation R(x:i32); output R;
                R(x) :- E(x), if *x != 0, if partial(x);
            },
            || panic!("new target panic"),
        )
        .is_err()
    );
}

#[test]
fn typed_multiple_outputs_and_edb_canonicalization() {
    let rows = [(1_i32, 1_i32), (1, 1), (1, 2), (2, 2)];
    let (diagonal, input) = reference! {
        struct Q; relation E(x:i32,y:i32) = (&rows); relation R(x:i32); output R,E;
        R(x) :- E(x,x);
    };
    assert_eq!(diagonal, HashSet::from([(1,), (2,)]));
    assert_eq!(input, HashSet::from([(1, 1), (1, 2), (2, 2)]));
}

#[test]
fn normalized_guards_compare_borrowed_columns_and_owned_bindings() {
    let rows = [(1_i32, 1_i32), (2, 2), (3, 3), (3, 4)];
    let low = 1;
    let high = 3;
    macro_rules! guarded {
        ($engine:ident) => {
            $engine! {
                struct Guarded;
                relation E(x:i32,y:i32) = (&rows);
                relation R(x:i32);
                output R;
                R(*x) :- E(x,y), if *x == *y,
                    if *x >= low && *x <= high,
                    if *x == 1 || *x == 3,
                    let next = *x + 1, if next > 1;
            }
        };
    }
    let expected = HashSet::from([(1,), (3,)]);
    assert_eq!(guarded!(reference), expected);
    assert_eq!(guarded!(sequential), expected);
    check_refinement(|| guarded!(reference), || guarded!(alternate)).unwrap();
}

macro_rules! recorded_choices {
    ($($source:tt)*) => {
        cq_datalog::datalog! {
            pipeline {
                storage_rewrites = [cq_datalog_test_passes::ordered_storage,
                    cq_datalog_test_passes::direct_later_delta];
                round = [cq_datalog_test_passes::reverse_publication];
            };
            $($source)*
        }.collect::<HashSet<_>>()
    };
}

#[test]
fn ordered_providers_reversed_publication_and_later_direct_delta_match_ascent() {
    ascent::ascent! {
        struct Oracle;
        relation edge(i32, i32);
        relation path(i32, i32);
        relation mirror(i32, i32);
        path(*x, *y) <-- edge(x, y);
        path(*x, *z) <-- edge(x, y), mirror(y, z);
        mirror(*x, *y) <-- path(x, y);
    }
    for edges in [
        vec![],
        vec![(0, 1)],
        vec![(0, 1), (1, 2), (2, 3), (0, 1)],
        vec![(0, 1), (1, 0), (2, 3)],
    ] {
        let mut oracle = Oracle {
            edge: edges.clone(),
            ..Default::default()
        };
        oracle.run();
        let actual = recorded_choices! {
            struct Choices;
            relation Edge(x:i32,y:i32) = (&edges);
            relation Path(x:i32,y:i32);
            relation Mirror(x:i32,y:i32);
            output Path;
            Path(*x,*y) :- Edge(x,y);
            Path(*x,*z) :- Edge(x,y), Mirror(y,z);
            Mirror(*x,*y) :- Path(x,y);
        };
        assert_eq!(actual, oracle.path.into_iter().collect());
    }
}
