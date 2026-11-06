use std::collections::{BTreeMap, BTreeSet};
use cq_lang_ra as ra;
use cq_lang_vo as vo;
use cq_relational_lowering::{compile, compile_with_origins, check_origins};
use quote::{ToTokens, quote};

fn planned(text: &str) -> vo::Program {
    let source = syn::parse_str(text).unwrap();
    let source = vo::cq::normal::normalize(&source).unwrap();
    cq_order::compile(&source, &Default::default()).unwrap()
}
fn restrictions(steps: &[vo::Step], prefix: &str, found: &mut BTreeMap<String, vo::syntax::Constraint>) {
    for (i, step) in steps.iter().enumerate() {
        let name = format!("{prefix}{i}");
        match step {
            vo::Step::Restrict(r, _) => { found.insert(name, r.predicate.clone()); }
            vo::Step::Block(block, _) => {
                for (branch, arm) in block.branches.iter().enumerate() {
                    restrictions(&arm.steps, &format!("{name}/{branch}/"), found);
                }
            }
            _ => (),
        }
    }
}
#[test]
fn deferred_restrictions_and_nested_branches_have_their_actual_step_paths() {
    for choice in ["|", "else"] {
        let source = planned(&format!(
            "struct Q; relation R(x:i32); relation S(x:i32); answer(x) :- \
             R(x), where x >= 1, \
             {{ S(x), where x <= 2, {{ R(x), where x != 0 }} | {{ S(x), where x >= 2 }} }} \
             {choice} {{ S(x), where x <= 3 }}, where x <= 4;"
        ));
        let (target, origins) = compile_with_origins(&source).unwrap();
        assert_eq!(target, compile(&source).unwrap(), "recording changed executable RA");
        check_origins(&source, &target, &origins).unwrap();
        let mut expected = BTreeMap::new();
        restrictions(&source.valuation.steps, "", &mut expected);
        assert_eq!(expected.len(), 6);
        assert!(expected.keys().any(|path| path.matches('/').count() == 4));
        for (path, predicate) in expected {
            let mut theta = ra::Theta::default();
            theta.push_constraint(predicate);
            let matches: Vec<_> = target.plan.definitions.iter().zip(&origins.definitions)
                .filter(|(_, (_, paths))| paths.contains(&path)).collect();
            assert!(!matches.is_empty(), "unrecorded deferred restriction {path}");
            for (definition, _) in matches {
                let ra::Operator::Select(select) = &definition.operator else {
                    panic!("restriction attributed to unrelated operator {}", definition.to_token_stream());
                };
                assert!(theta.equalities.iter().all(|e| select.condition.equalities.iter().any(|actual| actual == e)));
                assert!(theta.constraints.iter().all(|c| select.condition.constraints.iter().any(|actual| actual == c)));
            }
        }
    }
}
#[test]
fn reused_reduction_prefix_records_atom_even_without_a_new_definition() {
    let source: vo::cq::Program = syn::parse_quote! {
        struct Q; relation R(x:i32); relation S(x:i32);
        answer(x) :- R(x), S(x);
    };
    let ordering = &source.ordering;
    let source: vo::Program = syn::parse2(quote! {
        #source valuation #ordering {
            reduce R@0 with S@1(x); R@0(x); S@1(x);
        }
    }).unwrap();
    let (target, origins) = compile_with_origins(&source).unwrap();
    assert_eq!(target, compile(&source).unwrap());
    let first_reduction = target.plan.definitions.iter().position(|d|
        matches!(d.operator, ra::Operator::Semijoin(_))).unwrap();
    assert_eq!(origins.definitions[first_reduction].1, ["0", "1"]);
    check_origins(&source, &target, &origins).unwrap();
}
#[test]
fn unit_setup_and_return_keep_checked_origin_identities() {
    // CQ requires a relational occurrence. A negative-only query supplies a
    // real unit prefix, while the captured scalar keeps result arity positive.
    let source = planned("struct Q; relation R(x:i32); answer(k) :- let k:i32 = 1, !R(_);");
    assert_eq!(source.valuation.steps.len(), 1);
    let (target, origins) = compile_with_origins(&source).unwrap();
    let unit = target.plan.definitions.iter().position(|definition| matches!(definition.operator, ra::Operator::Unit(_))).unwrap();
    assert_eq!(origins.definitions[unit].1, ["0"]);
    assert!(origins.definitions.iter().any(|(_, paths)| paths.iter().any(|path| path == "$output")));
    assert!(origins.definitions.iter().flat_map(|(_, paths)| paths).all(|path| path == "0" || path == "$output"));
    assert_eq!(target, compile(&source).unwrap());
    check_origins(&source, &target, &origins).unwrap();
}
#[test]
fn forged_origin_and_forged_executable_lowering_are_rejected() {
    let source = planned("struct Q; relation R(x:i32); relation S(x:i32); answer(x) :- R(x), S(x);");
    let (target, mut origins) = compile_with_origins(&source).unwrap();
    let covered: BTreeSet<_> = origins.definitions.iter().flat_map(|(_, o)| o.iter().cloned()).collect();
    assert!(covered.contains("0") && covered.contains("1") && covered.contains("$output"));
    origins.definitions[0].1 = vec!["1".into()];
    assert!(check_origins(&source, &target, &origins).is_err());
    let (mut target, origins) = compile_with_origins(&source).unwrap();
    target.plan.definitions[0].result = syn::parse_quote!(tampered);
    assert!(check_origins(&source, &target, &origins).is_err());
}
