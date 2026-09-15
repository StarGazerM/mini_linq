//! Explicit edges admit their own domains; convenience routing does not constrain them.
use cq_ir::{cq, relational_plan as ra};
use cq_plan::JoinPolicy;
use cq_plan::PlanningOptions;
use cq_plan::Rule as JoinRule;

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
    let (graph, graph_proof) = cq_plan::graph::explain(&source, options).unwrap();
    let (prefix, prefix_proof) = cq_plan::prefix::explain(&source, options).unwrap();
    assert!(
        graph_proof
            .derivation
            .iter()
            .any(|r| matches!(r, JoinRule::Up { .. }))
    );
    assert_eq!(prefix_proof.derivation.last(), Some(&JoinRule::Finish));
    for (target, proof) in [(&graph, &graph_proof), (&prefix, &prefix_proof)] {
        cq_compose::check_cq_plan(&source, target, proof).unwrap();
        assert_eq!(
            &cq_compose::compile_cq_derivation(&source, proof).unwrap(),
            target
        );
        let mut incomplete = proof.clone();
        incomplete.derivation.pop();
        assert!(cq_compose::check_cq_plan(&source, target, &incomplete).is_err());
        assert!(cq_compose::compile_cq_derivation(&source, &incomplete).is_err());
    }
    assert!(cq_compose::check_cq_plan(&source, &graph, &prefix_proof).is_err());
    assert!(cq_compose::check_cq_plan(&source, &prefix, &graph_proof).is_err());
}

#[test]
fn explicit_edges_reject_unsupported_domains_instead_of_switching_implementation() {
    let expression: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32); answer(x) :- R(x), let y = *x + 1, if y > 0;
    };
    assert!(cq_plan::canonical::compile(&expression).is_err());
    assert!(cq_plan::graph::compile(&expression).is_err());
    cq_plan::prefix::compile(&expression).unwrap();

    let graph = cq_plan::graph::compile(&source()).unwrap();
    assert!(
        graph
            .plan
            .definitions
            .iter()
            .any(|d| matches!(d.operator, ra::Operator::Semijoin(_)))
    );
    assert!(cq_index::input::compile(&graph).is_err());
    let indexed = cq_index::result::compile(&graph).unwrap();
    assert!(
        cq_physical::input::compile_input_indexed(&indexed, &syn::parse_quote!(::mini_linq::pull))
            .is_err()
    );
    assert!(
        cq_physical::lazy::compile_lazy_indexed(&indexed, &syn::parse_quote!(::mini_linq::pull))
            .is_err()
    );
    cq_physical::materialized::compile_materialized(
        &indexed,
        &syn::parse_quote!(::mini_linq::collect_access),
    )
    .unwrap();
}

#[test]
fn every_explicit_entry_checks_source_well_formedness() {
    let invalid: cq::Module = syn::parse_quote! {
        struct P; relation R(a:i32); answer(x) :- Missing(x);
    };
    for lower in [
        cq_plan::canonical::compile,
        cq_plan::prefix::compile,
        cq_plan::graph::compile,
    ] {
        assert!(
            lower(&invalid)
                .unwrap_err()
                .to_string()
                .contains("not declared")
        );
    }
    let mut invalid = cq_plan::canonical::compile(&source()).unwrap();
    invalid.plan.output.input = syn::parse_quote!(missing);
    for lower in [cq_index::input::compile, cq_index::result::compile] {
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
        let (target, proof) = cq_plan::prefix::explain(
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
            assert!(cq_compose::check_cq_plan(&source, &target, &invalid).is_err());
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                cq_compose::compile_cq_derivation(&source, &invalid)
            }));
            assert!(
                result
                    .expect("malformed derivation must not panic")
                    .is_err()
            );
        }
    }
}

#[test]
fn default_index_choice_emits_only_its_bound_physical_continuation() {
    for (body, expected_path, result_keys) in [
        (
            quote::quote!(R(x), S(x)),
            syn::parse_quote!(renamed::input),
            false,
        ),
        (
            quote::quote!(R(x), S(x), let () = ()),
            syn::parse_quote!(renamed::lazy),
            true,
        ),
    ] {
        let cq: cq_ir::cq::Module = syn::parse2(quote::quote! {
            struct P; relation R(a:i32); relation S(a:i32);
            answer(x) :- #body;
        })
        .unwrap();
        let ra = cq_compose::compile_cq(&cq).unwrap();
        let tokens = cq_compose::expand_indexed(
            &ra,
            &syn::parse_quote!(renamed::input),
            &syn::parse_quote!(renamed::lazy),
        )
        .unwrap();
        let invocation: syn::ItemMacro = syn::parse2(tokens).unwrap();
        assert_eq!(invocation.mac.path, expected_path);
        let indexes: cq_ir::index_requirements::Module =
            syn::parse2(invocation.mac.tokens).unwrap();
        assert!(!indexes.indexes.is_empty());
        assert!(
            indexes
                .indexes
                .iter()
                .all(|key| key.result_token.is_some() == result_keys)
        );
        assert_eq!(indexes.relational_plan, ra);
    }
}
