#[macro_use]
mod generated_support;

use cq_lang_physical::{self as physical, Choice, Operator};
use cq_testkit::{Rows, harness, structures};
use generated_support::{Inputs, Run, binary, fan, generic, oracle};

fn captured(rewritten: bool) -> structures::Program {
    let text = if rewritten {
        triangle! {
            r,s,t;
            plan = cq_planners::order(written, fine);
            physical = [cq_rewrites::canon_prefixes, cq_rewrites::smallest_intersections];
            print_input = structures;
            print = cq_rewrite_test_driver::capture_structures;
        }
    } else {
        triangle! {
            r,s,t;
            plan = cq_planners::order(written, fine);
            print_input = structures;
            print = cq_rewrite_test_driver::capture_structures;
        }
    };
    syn::parse_str(text).unwrap()
}

#[test]
fn generated_fine_triangle_and_fan_have_active_smallest_intersections() {
    let source = captured(false);
    let target = captured(true);
    physical::contract::check(&source.physical).unwrap();
    physical::contract::check(&target.physical).unwrap();
    physical::check_rewrite(&source.physical, &target.physical).unwrap();
    structures::contract::check(&target).unwrap();
    let smallest: Vec<_> = target
        .physical
        .program
        .definitions
        .windows(2)
        .filter(|pair| {
            matches!(&pair[0].operator, Operator::Probe(_, key)
                if matches!(key.choice, Some(Choice::Smallest(_))))
        })
        .collect();
    assert_eq!(
        smallest.len(),
        1,
        "generated fine plan needs an active choice"
    );
    let facts = physical::contract::check(&target.physical).unwrap();
    for pair in smallest {
        assert!(physical::binary_intersection(&pair[0], &pair[1], &facts).is_some());
    }
    // The fan is data for the same generated triangle query. Verify it uses
    // this active plan rather than substituting an authored adjacent fixture.
    let inputs = fan(6);
    assert_eq!(generic(&inputs[0], &inputs[1], &inputs[2]), oracle(&inputs));
}

#[test]
fn generated_triangle_and_fan_answers_match_cq_oracle_with_duplicates_and_empty_inputs() {
    let triangle: Inputs = [
        vec![(1, 10), (1, 10), (1, 11), (2, 20), (3, 30)],
        vec![(10, 100), (10, 100), (11, 101), (20, 200), (30, 300)],
        vec![(1, 100), (1, 100), (2, 200), (9, 900)],
    ];
    let mut duplicated_fan = fan(7);
    for rows in &mut duplicated_fan {
        rows.extend(rows.clone());
        rows.reverse();
    }
    for inputs in [triangle, fan(6), duplicated_fan] {
        let expected = oracle(&inputs);
        assert!(!expected.is_empty());
        for run in [binary as Run, generic] {
            assert_eq!(run(&inputs[0], &inputs[1], &inputs[2]), expected);
            for empty in 0..3 {
                let mut inputs = inputs.clone();
                inputs[empty].clear();
                let expected: Rows = oracle(&inputs);
                assert!(expected.is_empty());
                assert_eq!(run(&inputs[0], &inputs[1], &inputs[2]), expected);
            }
        }
    }
}

macro_rules! three_way_query {
    ($seed:ident, $r:ident, $s:ident, $t:ident; $($options:tt)*) => {
        mini_linq::compile! {
            pipeline {
                plan = mini_linq::passes::plan(no_reduce, order(
                    Seed, R@1(a,x), S@2(a,x), T@3(a,x), R@1, S@2, T@3
                ));
                $($options)*
            }
            struct ThreeWayAdvice;
            relation Seed(a:i32) = ($seed);
            relation R(a:i32,x:i32,y:i32) = ($r);
            relation S(a:i32,x:i32,z:i32) = ($s);
            relation T(a:i32,x:i32,w:i32) = ($t);
            answer(a,x,y,z,w) :- Seed(a), R(a,x,y), S(a,x,z), T(a,x,w);
        }
    };
}

#[test]
fn step_order_and_occurrence_smallest_advice_reach_three_provider_execution() {
    let source: structures::Program = syn::parse_str(three_way_query! {
        seed,r,s,t;
        print_input = structures;
        print = cq_rewrite_test_driver::capture_structures;
    }).unwrap();
    let target: structures::Program = syn::parse_str(three_way_query! {
        seed,r,s,t;
        physical = [cq_rewrites::smallest_intersections(smallest(R@1,S@2,T@3))];
        print_input = structures;
        print = cq_rewrite_test_driver::capture_structures;
    }).unwrap();
    physical::check_rewrite(&source.physical, &target.physical).unwrap();
    let facts = structures::contract::check(&target).unwrap();
    assert!(physical::candidate_prefixes(&source.physical, &facts.physical).is_empty());
    let sites = physical::candidate_prefixes(&target.physical, &facts.physical);
    assert_eq!(sites.len(), 1);
    assert_eq!(sites[0].sides.len(), 3);
    let seed = [(1,)];
    let r = [(1, 7, 10), (1, 7, 11), (1, 8, 12), (1, 7, 10)];
    let s = [(1, 7, 20), (1, 9, 21)];
    let t = [(1, 7, 30), (1, 7, 31), (1, 10, 32)];
    let expected: Rows = [vec![1,7,10,20,30], vec![1,7,10,20,31],
        vec![1,7,11,20,30], vec![1,7,11,20,31]].into_iter().collect();
    let baseline: Rows = three_way_query! { seed,r,s,t; }
        .map(|(a,x,y,z,w)| vec![*a,*x,*y,*z,*w]).collect();
    let advised: Rows = three_way_query! {
        seed,r,s,t;
        physical = [cq_rewrites::smallest_intersections(smallest(R@1,S@2,T@3))];
    }.map(|(a,x,y,z,w)| vec![*a,*x,*y,*z,*w]).collect();
    assert_eq!(baseline, expected);
    assert_eq!(advised, expected);
}

fn nonzero(value: &i32) -> bool {
    *value != 0
}

fn positive(value: &i32) -> bool {
    *value > 0
}

macro_rules! partial_triangle {
    ($inputs:ident; $($pipeline:tt)*) => {
        mini_linq::compile! {
            pipeline { $($pipeline)* }
            struct PartialTriangle;
            relation r(a:i32,b:i32) = (&$inputs[0]);
            relation s(b:i32,c:i32) = (&$inputs[1]);
            relation t(a:i32,c:i32) = (&$inputs[2]);
            answer(a,b,c,quotient) :- r(a,b), if (nonzero(a)),
                let quotient:i32 = (12 / *a), if (positive(a)), s(b,c), t(a,c);
        }
    };
}

fn partial_reference(inputs: &Inputs) -> Rows {
    partial_triangle! {
        inputs;
        plan = cq_planners::order(written, coarse);
    }
    .map(|(a, b, c, quotient)| vec![*a, *b, *c, quotient])
    .collect()
}

fn partial_target(inputs: &Inputs) -> Rows {
    partial_triangle! {
        inputs;
        plan = cq_planners::order(written, fine);
        physical = [cq_rewrites::canon_prefixes, cq_rewrites::smallest_intersections];
    }
    .map(|(a, b, c, quotient)| vec![*a, *b, *c, quotient])
    .collect()
}

fn captured_partial(rewritten: bool) -> structures::Program {
    let text = if rewritten {
        partial_triangle! {
            inputs;
            plan = cq_planners::order(written, fine);
            physical = [cq_rewrites::canon_prefixes, cq_rewrites::smallest_intersections];
            print_input = structures;
            print = cq_rewrite_test_driver::capture_structures;
        }
    } else {
        partial_triangle! {
            inputs;
            plan = cq_planners::order(written, fine);
            print_input = structures;
            print = cq_rewrite_test_driver::capture_structures;
        }
    };
    syn::parse_str(text).unwrap()
}

#[test]
fn generated_hoist_pins_opaque_rust_and_refines_written_execution() {
    let source = captured_partial(false);
    let target = captured_partial(true);
    let rust = |program: &physical::Program| {
        program
            .program
            .definitions
            .iter()
            .enumerate()
            .filter(|(_, d)| matches!(d.operator, Operator::Bind(_) | Operator::Guard(_)))
            .map(|(position, d)| (position, d.clone()))
            .collect::<Vec<_>>()
    };
    let source_rust = rust(&source.physical);
    assert_eq!(source_rust.len(), 3);
    assert_eq!(source_rust, rust(&target.physical));
    physical::check_rewrite(&source.physical, &target.physical).unwrap();
    structures::contract::check(&target).unwrap();
    assert_eq!(target.physical.program.definitions.iter().filter(|d| {
        matches!(&d.operator, Operator::Probe(_, k) if matches!(k.choice, Some(Choice::Smallest(_))))
    }).count(), 1);

    let triangle: Inputs = [
        vec![(0, 10), (-1, 10), (1, 10), (1, 10), (2, 20)],
        vec![(10, 100), (20, 200)],
        vec![(0, 100), (-1, 100), (1, 100), (2, 200)],
    ];
    let mut empty = fan(6);
    empty[2].clear();
    for inputs in [triangle, fan(6), empty] {
        let expected: Rows = oracle(&inputs)
            .into_iter()
            .filter(|row| row[0] > 0)
            .map(|mut row| {
                row.push(12 / row[0]);
                row
            })
            .collect();
        let result =
            harness::check_refinement_on(&inputs, partial_reference, partial_target).unwrap();
        assert!(
            matches!(result.reference, harness::Outcome::Returned(ref rows) if *rows == expected)
        );
        assert!(matches!(result.target, harness::Outcome::Returned(ref rows) if *rows == expected));
    }
}
