use cq_lang_physical as ir;
use cq_rewrite_passes::host_sets;
use quote::{ToTokens, quote};

#[test]
fn host_set_text_round_trips_and_removes_only_implied_canon() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation E(a:i64,b:i64);
        e = pipe rename E@0 {a -> x,b -> y};
        p = pipe canon project e keep {x,y};
        return p as answer(x,y);
    }};
    let target = host_sets(&source, quote!(E)).unwrap();
    let expected: ir::Program = syn::parse_quote! { physical {
        relation E(a:i64,b:i64);
        host_set E;
        e = pipe rename E@0 {a -> x,b -> y};
        p = pipe project e keep {x,y};
        return p as answer(x,y);
    }};
    assert_eq!(target, expected);
    assert_eq!(
        syn::parse2::<ir::Program>(target.to_token_stream()).unwrap(),
        target
    );
    assert_eq!(host_sets(&target, quote!(E)).unwrap(), target);
    let selected = cq_structure_selection::compile(&target).unwrap();
    let facts = cq_testkit::structures::contract::check(&selected).unwrap();
    assert_eq!(facts.unique["p"], ["x", "y"]);
    assert!(facts.physical.canon.is_empty());
    assert!(host_sets(&source, quote!(missing)).is_err());
    assert!(host_sets(&source, quote!(e)).is_err());
    assert!(host_sets(&source, quote!()).is_err());
}

#[test]
fn dropping_full_heading_key_keeps_canon() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation E(a:i64,b:i64);
        e = pipe rename E@0 {a -> x,b -> y};
        p = pipe canon project e keep {x};
        return p as answer(x);
    }};
    let target = host_sets(&source, quote!(E)).unwrap();
    assert!(target.program.definitions[1].canon.is_some());
    cq_structure_selection::compile(&target).unwrap();
}

#[test]
fn lowered_triangle_accepts_certified_host_keys_without_canon() {
    let scenario = cq_testkit::SCENARIOS
        .iter()
        .find(|s| s.name == "triangle")
        .unwrap();
    let source = scenario.parse_ix().unwrap();
    let lowered = cq_physical_lowering::compile(&source).unwrap();
    let relations: Vec<_> = lowered
        .program
        .definitions
        .iter()
        .filter_map(|d| match &d.operator {
            ir::Operator::Rename(r) => Some(r.relation.clone()),
            _ => None,
        })
        .collect();
    let target = host_sets(&lowered, quote!(#(#relations),*)).unwrap();
    assert!(target.program.definitions.iter().all(|d| d.canon.is_none()));
    assert!(target.program.return_canon.is_none());
    cq_physical_lowering::check(&source, &target).unwrap();
    cq_structure_selection::compile(&target).unwrap();
    let mut unchecked = target;
    unchecked.program.host_sets.clear();
    assert!(cq_physical_lowering::check(&source, &unchecked).is_err());
    assert!(cq_structure_selection::compile(&unchecked).is_err());
}

#[test]
fn fine_triangle_key_losing_prefix_still_needs_canon() {
    let scenario = cq_testkit::SCENARIOS
        .iter()
        .find(|s| s.name == "fine_triangle")
        .unwrap();
    let source = scenario.parse_ix().unwrap();
    let lowered = cq_physical_lowering::compile(&source).unwrap();
    let target = host_sets(&lowered, quote!(R, S, T)).unwrap();
    // Its initial project keeps only a from R(a,b), then re-expands R.
    // Certifying R's full heading does not make that prefix unique.
    assert!(target.program.definitions.iter().any(|d| d.canon.is_some()));
    cq_physical_lowering::check(&source, &target).unwrap();
    cq_structure_selection::compile(&target).unwrap();
}

#[test]
fn duplicate_and_unknown_fact_text_is_rejected() {
    for facts in [quote!(host_set E; host_set E;), quote!(host_set Missing;)] {
        let program: ir::Program = syn::parse2(quote! { physical {
            relation E(a:i64);
            #facts
            e = pipe rename E@0 {a -> x};
            return canon e as answer(x);
        }})
        .unwrap();
        assert!(ir::contract::check(&program).is_err());
    }
}
