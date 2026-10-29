//! Native Round-reference cases: rewritten bindings, negation keys, and bottom.
use std::collections::HashSet;

#[test]
fn nonlinear_rounds_follow_permuted_layouts_and_later_direct_deltas() {
    let edges = [(1_i32, 2_i32), (2, 3), (3, 4), (1, 2)];
    let result = cq_datalog::datalog! {
        pipeline {
            storage_rewrites = [
                cq_datalog_test_passes::alternate_storage,
                cq_datalog_test_passes::direct_later_delta
            ];
            round = [cq_datalog_test_passes::reverse_publication];
            print = cq_datalog_test_passes::eval_round;
        }
        struct Closure;
        relation Edge(x:i32,y:i32) = (&edges);
        relation Reach(x:i32,y:i32);
        output Reach, Edge;
        Reach(*x,*y) :- Edge(x,y);
        Reach(*x,*z) :- Reach(x,y), Reach(y,z);
    };
    assert_eq!(
        result.0,
        HashSet::from([(1, 2), (1, 3), (1, 4), (2, 3), (2, 4), (3, 4)])
    );
    assert_eq!(result.1, HashSet::from(edges));
}

#[test]
fn borrowed_filtered_keys_preserve_native_owned_rust_and_push_interfaces() {
    let names = [
        ("alice".to_owned(),),
        ("bob".to_owned(),),
        ("alice".to_owned(),),
    ];
    let blocked = [
        ("blocked".to_owned(), "bob".to_owned()),
        ("other".to_owned(), "alice".to_owned()),
    ];
    let result = cq_datalog::datalog! {
        pipeline {
            cq = pipeline { print = ::cq_datalog::passes::print_push; print_input = structures; };
            print = cq_datalog_test_passes::eval_round;
        }
        struct Names;
        relation Name(name:String) = (&names);
        relation Blocked(kind:String,name:String) = (&blocked);
        relation Allowed(name:String,n:i32);
        output Allowed;
        Allowed(owned.clone(), n) :- Name(name), !Blocked("blocked",name),
            let owned = name.to_uppercase(), for n in 0_i32..2;
        Allowed(name.clone(), *n + 1) :- Allowed(name,n), where n < 3;
    };
    assert_eq!(
        result,
        HashSet::from([
            ("ALICE".to_owned(), 0),
            ("ALICE".to_owned(), 1),
            ("ALICE".to_owned(), 2),
            ("ALICE".to_owned(), 3),
        ])
    );
}

#[test]
fn component_publication_retains_mutual_deltas_and_empty_negation_keys() {
    for blocked in [&[][..], &[(99_i32,)][..]] {
        let result = cq_datalog::datalog! {
            pipeline {
                round = [cq_datalog_test_passes::reverse_publication];
                print = cq_datalog_test_passes::eval_round;
            }
            struct Mutual;
            relation Seed(n:i32) = ([(0_i32,)]);
            relation Blocked(n:i32) = (blocked);
            relation A(n:i32);
            relation B(n:i32);
            relation Allowed(n:i32);
            output A, B, Allowed;
            A(*n) :- Seed(n);
            B(*n + 1) :- A(n), where n < 3;
            A(*n) :- B(n);
            Allowed(*n) :- A(n), !Blocked(_);
        };
        assert_eq!(result.0, HashSet::from([(0,), (1,), (2,), (3,)]));
        assert_eq!(result.1, HashSet::from([(1,), (2,), (3,)]));
        assert_eq!(
            result.2,
            if blocked.is_empty() {
                result.0
            } else {
                HashSet::new()
            }
        );
    }
}

#[test]
fn native_partial_computations_propagate_bottom_only_when_reached() {
    fn partial(n: &i32) -> i32 {
        assert_ne!(*n, 1, "native Round bottom");
        *n
    }
    for input in [&[][..], &[(1_i32,)][..]] {
        let outcome = std::panic::catch_unwind(|| {
            cq_datalog::datalog! {
                pipeline { print = cq_datalog_test_passes::eval_round; }
                struct Partial;
                relation Input(n:i32) = (input);
                relation Out(n:i32);
                output Out;
                Out(value) :- Input(n), let value = partial(n);
            }
        });
        if input.is_empty() {
            assert_eq!(outcome.unwrap(), HashSet::new());
        } else {
            assert!(outcome.is_err());
        }
    }
}

#[test]
fn surface_if_let_reaches_recorded_native_generators() {
    let values = [(Some(2_i32),), (None,), (Some(5),)];
    let result = cq_datalog::datalog! {
        pipeline { print = cq_datalog_test_passes::eval_round; }
        struct Options;
        relation Input(value:Option<i32>) = (&values);
        relation Out(value:i32);
        output Out;
        Out(n) :- Input(value), if let Some(n) = *value, where n > 2;
    };
    assert_eq!(result, HashSet::from([(5,)]));
}
