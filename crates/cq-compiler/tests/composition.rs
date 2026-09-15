//! Explicit edges admit their own domains; convenience routing does not constrain them.
use cq_compiler::{JoinPolicy, JoinRule, PlanningOptions};
use cq_ir::{cq, relational_plan as ra};

fn source() -> cq::Module {
    syn::parse_quote! {
        struct P;
        relation R(a:i32,b:i32); relation S(b:i32,c:i32);
        answer(a,c) :- R(a,b), S(b,c);
    }
}

#[test]
fn the_same_cq_can_select_either_derivation_grammar_and_replay_it() {
    let source = source();
    let options = PlanningOptions {
        strategy: JoinPolicy::Auto,
        max_arity: None,
    };
    let (graph, graph_proof) = cq_compiler::explain_cq_graph(&source, options).unwrap();
    let (prefix, prefix_proof) = cq_compiler::explain_cq_prefix(&source, options).unwrap();
    assert!(
        graph_proof
            .derivation
            .iter()
            .any(|r| matches!(r, JoinRule::Up { .. }))
    );
    assert_eq!(prefix_proof.derivation.last(), Some(&JoinRule::Finish));
    for (target, proof) in [(&graph, &graph_proof), (&prefix, &prefix_proof)] {
        cq_compiler::check_cq_plan(&source, target, proof).unwrap();
        assert_eq!(
            &cq_compiler::compile_cq_derivation(&source, proof).unwrap(),
            target
        );
        let mut incomplete = proof.clone();
        incomplete.derivation.pop();
        assert!(cq_compiler::check_cq_plan(&source, target, &incomplete).is_err());
        assert!(cq_compiler::compile_cq_derivation(&source, &incomplete).is_err());
    }
    assert!(cq_compiler::check_cq_plan(&source, &graph, &prefix_proof).is_err());
    assert!(cq_compiler::check_cq_plan(&source, &prefix, &graph_proof).is_err());
}

#[test]
fn explicit_edges_reject_unsupported_domains_instead_of_switching_implementation() {
    let expression: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32); answer(x) :- R(x), let y = *x + 1, if y > 0;
    };
    assert!(cq_compiler::compile_cq_canonical(&expression).is_err());
    assert!(cq_compiler::compile_cq_graph(&expression).is_err());
    cq_compiler::compile_cq_prefix(&expression).unwrap();

    let graph = cq_compiler::compile_cq_graph(&source()).unwrap();
    assert!(
        graph
            .plan
            .definitions
            .iter()
            .any(|d| matches!(d.operator, ra::Operator::Semijoin(_)))
    );
    assert!(cq_compiler::compile_input_indexes(&graph).is_err());
    let indexed = cq_compiler::compile_result_indexes(&graph).unwrap();
    assert!(cq_compiler::compile_input_indexed(&indexed).is_err());
    assert!(cq_compiler::compile_lazy_indexed(&indexed).is_err());
    cq_compiler::compile_materialized(&indexed).unwrap();
}

#[test]
fn every_explicit_entry_checks_source_well_formedness() {
    let invalid: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32); answer(x) :- Missing(x);
    };
    for lower in [
        cq_compiler::compile_cq_canonical,
        cq_compiler::compile_cq_prefix,
        cq_compiler::compile_cq_graph,
    ] {
        assert!(
            lower(&invalid)
                .unwrap_err()
                .to_string()
                .contains("not declared")
        );
    }
    let mut invalid = cq_compiler::compile_cq_canonical(&source()).unwrap();
    invalid.plan.output.input = syn::parse_quote!(missing);
    for lower in [
        cq_compiler::compile_input_indexes,
        cq_compiler::compile_result_indexes,
    ] {
        assert!(lower(&invalid).is_err());
    }
}

#[test]
fn incomplete_expression_proofs_return_diagnostics_before_graph_state_construction() {
    let sources: Vec<cq::Module> = vec![
        syn::parse_quote! {
            struct P; relation R(a:i32);
            answer(x) :- R(x), if x.is_positive();
        },
        syn::parse_quote! {
            struct P; relation R(a:i32);
            answer(y) :- R(x), let y:i32 = *x + 1, if keep(y);
        },
    ];
    for source in sources {
        let (target, proof) = cq_compiler::explain_cq_prefix(
            &source,
            PlanningOptions {
                strategy: JoinPolicy::Written,
                max_arity: None,
            },
        )
        .unwrap();
        for mutation in 0..3 {
            let mut invalid = proof.clone();
            match mutation {
                0 => {
                    invalid.derivation.pop();
                }
                1 => invalid.derivation.clear(),
                _ => {
                    invalid.derivation = vec![JoinRule::Introduce {
                        occurrence: usize::MAX,
                    }]
                }
            }
            assert!(cq_compiler::check_cq_plan(&source, &target, &invalid).is_err());
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                cq_compiler::compile_cq_derivation(&source, &invalid)
            }));
            assert!(
                result
                    .expect("malformed derivation must not panic")
                    .is_err()
            );
        }
    }
}
