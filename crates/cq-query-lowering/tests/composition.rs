//! Explicit edges admit their own domains; convenience routing does not constrain them.
use cq_ir::cq;
use cq_query_lowering::JoinPolicy;
use cq_query_lowering::PlanningOptions;
use cq_query_lowering::Rule as JoinRule;

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
    let (graph, graph_proof) = cq_query_lowering::hypergraph::explain(&source, options).unwrap();
    let (prefix, prefix_proof) =
        cq_query_lowering::clause_prefix::explain(&source, options).unwrap();
    assert!(
        graph_proof
            .derivation
            .iter()
            .any(|r| matches!(r, JoinRule::Up { .. }))
    );
    assert_eq!(prefix_proof.derivation.last(), Some(&JoinRule::Finish));
    for (target, proof) in [(&graph, &graph_proof), (&prefix, &prefix_proof)] {
        cq_query_lowering::composition::check_cq_plan(&source, target, proof).unwrap();
        assert_eq!(
            &cq_query_lowering::composition::compile_cq_derivation(&source, proof).unwrap(),
            target
        );
        let mut incomplete = proof.clone();
        incomplete.derivation.pop();
        assert!(
            cq_query_lowering::composition::check_cq_plan(&source, target, &incomplete).is_err()
        );
        assert!(
            cq_query_lowering::composition::compile_cq_derivation(&source, &incomplete).is_err()
        );
    }
    assert!(cq_query_lowering::composition::check_cq_plan(&source, &graph, &prefix_proof).is_err());
    assert!(cq_query_lowering::composition::check_cq_plan(&source, &prefix, &graph_proof).is_err());
}

#[test]
fn explicit_edges_reject_unsupported_domains_instead_of_switching_implementation() {
    let expression: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32); answer(x) :- R(x), let y = *x + 1, if y > 0;
    };
    assert!(cq_query_lowering::written_order::compile(&expression).is_err());
    assert!(cq_query_lowering::hypergraph::compile(&expression).is_err());
    cq_query_lowering::clause_prefix::compile(&expression).unwrap();
}

#[test]
fn every_explicit_entry_checks_source_well_formedness() {
    let invalid: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32); answer(x) :- Missing(x);
    };
    for lower in [
        cq_query_lowering::written_order::compile,
        cq_query_lowering::clause_prefix::compile,
        cq_query_lowering::hypergraph::compile,
    ] {
        assert!(
            lower(&invalid)
                .unwrap_err()
                .to_string()
                .contains("not declared")
        );
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
        let (target, proof) = cq_query_lowering::clause_prefix::explain(
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
            assert!(
                cq_query_lowering::composition::check_cq_plan(&source, &target, &invalid).is_err()
            );
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                cq_query_lowering::composition::compile_cq_derivation(&source, &invalid)
            }));
            assert!(
                result
                    .expect("malformed derivation must not panic")
                    .is_err()
            );
        }
    }
}
