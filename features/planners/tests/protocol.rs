use cq_planners::Planner;
use cq_testkit::Rows;
use quote::{ToTokens, quote};

#[test]
fn every_planner_runs_through_the_named_facade() {
    let r = [(1, 2), (1, 2), (2, 3), (7, 8)];
    let s = [(2, 4), (3, 5), (3, 5)];
    let expected = Rows::from([vec![1, 4], vec![2, 5]]);
    macro_rules! verify {
        ($($arguments:tt)*) => {{
            let answer: Rows = mini_linq::compile! {
                pipeline { plan = cq_planners::order($($arguments)*); }
                struct Q;
                relation r(x:i32,y:i32); relation s(y:i32,z:i32);
                answer(x,z) :- r(x,y),s(y,z);
            }.map(|(x,z)| vec![*x,*z]).collect();
            assert_eq!(answer, expected);
        }};
    }
    verify!(written, coarse);
    verify!(written, fine);
    verify!(hypergraph);
    verify!(prefix connected, coarse);
    verify!(prefix connected, fine);
    verify!(prefix greedy, coarse);
    verify!(prefix greedy, fine);
    verify!(elimination, coarse);
    verify!(elimination, fine);
    verify!();
}

#[test]
fn explicit_queue_and_borrowed_leaf_with_partial_binding() {
    let rows = [(0,), (1,), (2,), (2,)];
    let input = || rows.as_slice();
    let expected = Rows::from([vec![10], vec![5]]);
    let answer: Rows = cq_planners::order! {
        arguments { prefix greedy, fine };
        pipeline {
            mini_linq::passes::index_demands,
            mini_linq::passes::lower,
            mini_linq::passes::select,
            mini_linq::passes::print_pull
        };
        struct Q;
        relation R(x:i32) = (input());
        relation answer(y:i32);
        answer(y) :- R(x), if { *x != 0 }, let y:i32 = 10 / *x;
    }
    .map(|(y,)| vec![y])
    .collect();
    assert_eq!(answer, expected);
}

#[test]
fn checked_stage_emits_only_recorded_ra_and_forwards_the_remaining_queue() {
    let source = quote! { struct Q; relation R(x:i32); answer(x) :- R(x); };
    for planner in Planner::ALL {
        let arguments = planner.to_token_stream();
        let emitted = cq_stage::pass::<cq_planners::cq::Program, cq_planners::ra::Program>(
            quote! { arguments { #arguments }; pipeline { capture::ra(frozen), terminal::printer }; #source },
            |source, arguments| cq_planners::compile(source, syn::parse2(arguments)?),
        );
        let parsed_source = syn::parse2(source.clone()).unwrap();
        let target = cq_planners::compile(&parsed_source, planner).unwrap();
        let expected = quote! {
            capture::ra! { arguments { frozen }; pipeline { terminal::printer }; #target }
        };
        assert_eq!(emitted.to_string(), expected.to_string());
    }
    let ended = cq_stage::pass::<cq_planners::cq::Program, cq_planners::ra::Program>(
        quote! { pipeline {}; #source },
        |source, arguments| cq_planners::compile(source, syn::parse2(arguments)?),
    );
    assert!(
        ended
            .to_string()
            .contains("pipeline ended before its Rust printer")
    );
}
