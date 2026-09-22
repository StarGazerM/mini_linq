use cq_compose::check_cq_plan;
use cq_compose::compile_cq_derivation;
use cq_compose::explain_cq_plan;
use cq_ir::{cq, relational_plan as ra};
use cq_query_lowering::JoinPolicy;
use cq_query_lowering::PlanningOptions;

fn source() -> cq::Module {
    syn::parse_quote! {
        struct P;
        relation R(a:i32,b:i32);
        relation S(a:i32,b:i32);
        relation answer(value:String);
        answer(format!("{x}:{label}")) :- R(x,y), S(f(x),z),
            let label = format!("{y}:{z}"), if accept(x), if accept(y);
    }
}

#[test]
fn every_policy_respects_rust_dependencies_and_derivation_reentry() {
    let source = source();
    let mut previous = None;
    for strategy in [
        JoinPolicy::Written,
        JoinPolicy::Connected,
        JoinPolicy::GreedyWidth,
        JoinPolicy::Auto,
    ] {
        let (target, report) = explain_cq_plan(
            &source,
            PlanningOptions {
                strategy,
                max_arity: None,
            },
        )
        .unwrap();
        assert!(report.analysis.is_empty() && report.join_tree.is_none());
        assert_eq!(compile_cq_derivation(&source, &report).unwrap(), target);
        if let Some(previous) = previous {
            assert_eq!(previous, target);
        }
        previous = Some(target);
    }
}

#[test]
fn locally_valid_expression_mutations_fail_the_source_target_contract() {
    let source = source();
    let (target, report) = explain_cq_plan(
        &source,
        PlanningOptions {
            strategy: JoinPolicy::Auto,
            max_arity: None,
        },
    )
    .unwrap();
    for i in 0..target.plan.definitions.len() {
        let mut changed = target.clone();
        match &mut changed.plan.definitions[i].operator {
            ra::Operator::Select(s) => s.condition = syn::parse_quote!(true),
            ra::Operator::Extend(e) => e.value = syn::parse_quote!(unrelated_rust()),
            _ => continue,
        }
        ra::contract::check(&changed).unwrap();
        assert!(check_cq_plan(&source, &changed, &report).is_err());
    }
    for i in 0..report.derivation.len() {
        let mut changed = report.clone();
        changed.derivation.remove(i);
        assert!(check_cq_plan(&source, &target, &changed).is_err());
        let mut changed = report.clone();
        changed.derivation.insert(i, report.derivation[i].clone());
        assert!(check_cq_plan(&source, &target, &changed).is_err());
    }
}

#[test]
fn missing_match_predicate_and_opaque_macro_liveness_are_detected() {
    let source: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32,b:i32); relation answer(value:String);
        answer(format!("{x}")) :- R(x, f(x));
    };
    let (target, report) = explain_cq_plan(
        &source,
        PlanningOptions {
            strategy: JoinPolicy::Auto,
            max_arity: None,
        },
    )
    .unwrap();
    let mut changed = target.clone();
    let ra::Operator::Project(p) = &mut changed.plan.definitions.last_mut().unwrap().operator
    else {
        panic!()
    };
    p.attributes = syn::parse_quote!(x);
    ra::contract::check(&changed).unwrap();
    assert!(check_cq_plan(&source, &changed, &report).is_err());

    let mut changed = target.clone();
    let removed = changed.plan.definitions.remove(1);
    let ra::Operator::Select(s) = removed.operator else {
        panic!()
    };
    let ra::Operator::Project(p) = &mut changed.plan.definitions[1].operator else {
        panic!()
    };
    p.input = s.input;
    ra::contract::check(&changed).unwrap();
    assert!(check_cq_plan(&source, &changed, &report).is_err());
}

#[test]
fn adjacent_guards_cannot_be_swapped_even_when_both_are_available() {
    let source = source();
    let (mut target, report) = explain_cq_plan(
        &source,
        PlanningOptions {
            strategy: JoinPolicy::Auto,
            max_arity: None,
        },
    )
    .unwrap();
    let guards: Vec<_> = target
        .plan
        .definitions
        .iter()
        .enumerate()
        .filter_map(|(i, d)| matches!(d.operator, ra::Operator::Select(_)).then_some(i))
        .collect();
    let [a, b] = guards.as_slice() else { panic!() };
    let ra::Operator::Select(left) = &target.plan.definitions[*a].operator else {
        panic!()
    };
    let expr = left.condition.clone();
    let ra::Operator::Select(right) = &mut target.plan.definitions[*b].operator else {
        panic!()
    };
    let other = std::mem::replace(&mut right.condition, expr);
    let ra::Operator::Select(left) = &mut target.plan.definitions[*a].operator else {
        panic!()
    };
    left.condition = other;
    ra::contract::check(&target).unwrap();
    assert!(check_cq_plan(&source, &target, &report).is_err());
}

#[test]
fn a_legal_alternative_join_schedule_can_reenter_the_same_rules() {
    use cq_query_lowering::Rule as JoinRule;
    let source: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32,b:i32); relation S(a:i32,b:i32); relation T(a:i32);
        relation answer(value:i32);
        answer(f(x)) :- R(7,x), T(y), S(x,y);
    };
    let (planned, mut proof) = explain_cq_plan(
        &source,
        PlanningOptions {
            strategy: JoinPolicy::Connected,
            max_arity: None,
        },
    )
    .unwrap();
    assert_eq!(
        proof.derivation,
        vec![
            JoinRule::Constant {
                clause: 0,
                argument: 0
            },
            JoinRule::Match { clause: 0 },
            JoinRule::Match { clause: 2 },
            JoinRule::Match { clause: 1 },
            JoinRule::Finish
        ]
    );
    proof.derivation.swap(2, 3);
    let written = compile_cq_derivation(&source, &proof).unwrap();
    assert_ne!(written, planned);
    check_cq_plan(&source, &written, &proof).unwrap();
    proof.derivation.remove(0);
    assert!(compile_cq_derivation(&source, &proof).is_err());
}

#[test]
fn expression_boundaries_and_unavailable_guard_inputs_reject_reentry() {
    for guard in ["opaque(x,y)", "x > 0"] {
        let source:cq::Module=syn::parse_str(&format!("struct P; relation R(a:i32); relation T(a:i32); relation S(a:i32,b:i32); relation answer(value:i32); answer(f(x)) :- R(x), T(y), if {guard}, S(x,y);")).unwrap();
        let (_, mut proof) = explain_cq_plan(
            &source,
            PlanningOptions {
                strategy: JoinPolicy::Written,
                max_arity: None,
            },
        )
        .unwrap();
        if guard.starts_with("opaque") {
            proof.derivation.swap(2, 3);
        } else {
            proof.derivation.swap(0, 2);
        }
        assert!(compile_cq_derivation(&source, &proof).is_err(), "{guard}");
    }
}
