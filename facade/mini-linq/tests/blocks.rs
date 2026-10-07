#![allow(non_snake_case)]

#[test]
fn multi_rule_opaque_macros_retain_the_source_binder_hygiene() {
    macro_rules! accept {
        ($x:expr) => {
            *$x > 0
        };
    }
    macro_rules! query {
        ($r:expr, $s:expr) => {
            mini_linq::mini_linq! {
                struct Q; relation R(x:i32)=($r); relation S(x:i32)=($s);
                answer(x) :- R(x), if accept!(x);
                answer(x) :- S(x), if accept!(x);
            }
        };
    }
    let R = [(-1_i32,), (1,)];
    let S = [(2_i32,)];
    let answers: std::collections::BTreeSet<_> = query!(R, S).map(|(x,)| *x).collect();
    assert_eq!(answers, std::collections::BTreeSet::from([1, 2]));
}

#[test]
fn fallback_can_return_owned_non_copy_interface_values() {
    let R = [(1_i32,), (2,)];
    let rows: std::collections::BTreeSet<_> = mini_linq::mini_linq! {
        struct Q; relation R(x:i32);
        answer(y) :- R(x),
            {if std::convert::identity(*x)==1, let y:String=String::from("first")}
            else {let y:String=String::from("second")};
    }
    .map(|(y,)| y)
    .collect();
    assert_eq!(
        rows,
        std::collections::BTreeSet::from(["first".to_owned(), "second".to_owned()])
    );
}
use std::collections::BTreeSet;

#[test]
fn binding_union_deduplicates_overlap_and_drops_local_witnesses() {
    let R = [(1, 4), (1, 5), (2, 6)];
    let S = [(2, 9), (3, 8)];
    let result: BTreeSet<_> = mini_linq::compile! {
        pipeline {}
        struct Q; relation R(x:i32,y:i32); relation S(x:i32,y:i32);
        answer(x) :- { R(x,a) } | { S(x,b) };
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(result, [1, 2, 3].into());
}

#[test]
fn fallback_is_per_free_variable_and_preserves_the_suffix() {
    let P = [(1,), (2,), (3,)];
    let R = [(1, 10), (1, 11)];
    let S = [(1, 99), (2, 20)];
    let result: BTreeSet<_> = mini_linq::compile! {
        pipeline {}
        struct Q; relation P(x:i32); relation R(x:i32,y:i32); relation S(x:i32,y:i32);
        answer(x,y) :- P(x), { R(x,y) } else { S(x,y) }, if *y < 90;
    }
    .map(|(x, y)| (*x, *y))
    .collect();
    assert_eq!(result, [(1, 10), (1, 11), (2, 20)].into());
}

#[test]
fn rust_fallback_never_evaluates_a_later_nonempty_rows_branch() {
    let P = [(1,), (2,)];
    let R = [(1,)];
    let second = |x: &i32| {
        assert_eq!(*x, 2);
        true
    };
    let suffix = |x: &i32| *x > 0;
    let result: BTreeSet<_> = mini_linq::compile! {
        pipeline {}
        struct Q; relation P(x:i32); relation R(x:i32);
        answer(x) :- P(x), { R(x) } else { if second(x) }, if suffix(x);
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(result, [1, 2].into());
}

#[test]
fn test_union_can_remove_a_second_branch_panic() {
    let P = [(1,)];
    let first = |_: &i32| true;
    let second = |_: &i32| -> bool { panic!("second arm should not be needed") };
    let result: Vec<_> = mini_linq::compile! {
        pipeline {}
        struct Q; relation P(x:i32);
        answer(x) :- P(x), { if first(x) } | { if second(x) };
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(result, vec![1]);
}

#[test]
fn nested_fallback_and_union_with_rust_bindings() {
    let P = [(1,), (2,)];
    let yes = |x: &i32| *x == 1;
    let result: BTreeSet<_> = mini_linq::compile! {
        pipeline {}
        struct Q; relation P(x:i32); relation answer(x:i32,y:i32);
        answer(x,y) :- P(x),
            { if yes(x), { let y:i32 = *x + 10 } | { let y:i32 = *x + 20 } }
            else { let y:i32 = *x + 30 };
    }
    .map(|(x, y)| (*x, y))
    .collect();
    assert_eq!(result, [(1, 11), (1, 21), (2, 32)].into());
}

#[test]
fn constraint_only_pure_arms_use_the_prefix_domain() {
    let P = [(1,), (2,), (3,)];
    let result: BTreeSet<_> = mini_linq::compile! {
        pipeline {}
        struct Q; relation P(x:i32);
        answer(x) :- P(x), { if *x == 1 } | { if *x == 2 };
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(result, [1, 2].into());
}

#[test]
fn multi_rule_sugar_has_a_head_only_interface_and_borrowed_answers() {
    let R = [(1, 4), (2, 5)];
    let S = [(2, 6), (3, 7)];
    let result: BTreeSet<_> = mini_linq::compile! {
        pipeline {}
        struct Q; relation R(x:i32,y:i32); relation S(x:i32,y:i32);
        answer(x) :- R(x,witness); answer(y) :- S(y,witness);
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(result, [1, 2, 3].into());
}

#[test]
fn computed_multi_rule_heads_keep_the_declared_output_types() {
    let R = [(1,), (2,), (2,)];
    let result: BTreeSet<_> = mini_linq::compile! {
        pipeline {}
        struct Q; relation R(x:i32); relation answer(x:i32);
        answer(*x) :- R(x); answer(*x+1) :- R(x);
    }
    .collect();
    assert_eq!(result, [(1,), (2,), (3,)].into());
}

#[test]
fn opaque_branch_captures_survive_elimination_of_later_binders() {
    let R = [(1, 2)];
    let Q = [(1, 3)];
    let S = [(1, 2, 4)];
    let z = &99_i32;
    macro_rules! capture {
        ($value:ident) => {
            *$value == 99
        };
    }
    let result: Vec<_> = mini_linq::compile! {
        pipeline { plan = core_pass_fixtures::plan_elimination_fine; }
        struct C; relation R(x:i32,a:i32); relation Q(x:i32,b:i32); relation S(x:i32,a:i32,z:i32);
        answer(x) :- R(x,a), Q(x,b), { if capture!(z) } | { if false }, S(x,a,z);
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(result, vec![1]);
}

#[test]
fn one_arms_local_name_does_not_hide_another_arms_host_capture() {
    let R = [(1,), (2,)];
    let local = |_: &i32| false;
    let result: BTreeSet<_> = mini_linq::compile! {
        pipeline {}
        struct C; relation R(x:i32); relation answer(x:i32);
        answer(x) :- { R(x), let local:i32 = *x + 10 } | { R(x), if local(x) };
    }
    .map(|(x,)| *x)
    .collect();
    assert_eq!(result, [1, 2].into());
}

#[test]
fn union_is_lazy_across_disjuncts_and_retains_borrowed_non_copy_columns() {
    let A = [(String::from("a"),)];
    let B = [(String::from("b"),)];
    let first = std::cell::Cell::new(0);
    let second = std::cell::Cell::new(0);
    let visit_first = |_: &String| {
        first.set(first.get() + 1);
        true
    };
    let visit_second = |_: &String| {
        second.set(second.get() + 1);
        true
    };
    let mut result = mini_linq::compile! { pipeline {}
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
    let calls = std::cell::Cell::new(0);
    let result: BTreeSet<_> = mini_linq::compile! { pipeline {}
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
fn union_matches_authored_sets_with_duplicates_and_empty_disjuncts() {
    for (A, B, expected) in [
        (
            vec![(1,), (1,), (2,)],
            vec![(2,), (3,)],
            vec![vec![1], vec![2], vec![3]],
        ),
        (vec![], vec![(4,), (4,)], vec![vec![4]]),
        (vec![], vec![], vec![]),
    ] {
        let expected: cq_testkit::Rows = expected.into_iter().collect();
        let result = cq_testkit::harness::check_refinement(
            || {
                A.iter()
                    .chain(&B)
                    .map(|&(x,)| vec![x])
                    .collect::<cq_testkit::Rows>()
            },
            || {
                mini_linq::compile! {
                    pipeline {}
                    struct U; relation A(x:i32); relation B(x:i32);
                        answer(x) :- A(x); answer(x) :- B(x);
                }
                .map(|(x,)| vec![*x])
                .collect::<cq_testkit::Rows>()
            },
        )
        .unwrap();
        assert_eq!(
            result.target,
            cq_testkit::harness::Outcome::Returned(expected)
        );
    }
}
