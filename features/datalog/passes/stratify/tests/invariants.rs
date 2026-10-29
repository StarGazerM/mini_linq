use cq_datalog_incremental as seminaive;
use cq_datalog_stratify::{self as stratify, invariants};
use cq_lang_datalog_source::{BodyItem, Program};
use cq_lang_datalog_strata::contract;
use quote::ToTokens;

fn source() -> Program {
    syn::parse_quote! {
        struct Filtered;
        relation Seed(x:i32,y:i32) = (seed);
        relation Edge(x:i32,y:i32) = (edges);
        relation Block(x:i32) = (blocked);
        relation Reach(x:i32,y:i32);
        output Reach;
        Reach(*x,*y) :- Seed(x,y);
        Reach(*x,*z) :- Reach(x,y), Edge(y,z), !Block(z);
    }
}

#[test]
fn frozen_filter_roundtrips_and_erases_independently() {
    let before = stratify::compile(&source()).unwrap();
    let after = invariants::compile(&before).unwrap();
    assert_eq!(after.source.invariants.len(), 1);
    assert_eq!(after.source.invariants[0].rule, 1);
    let parsed: cq_lang_datalog_strata::Program = syn::parse2(after.to_token_stream()).unwrap();
    assert_eq!(parsed, after);
    invariants::check(&before, &parsed).unwrap();
    let scheduled = seminaive::compile(&parsed).unwrap();
    cq_lang_datalog_incremental::contract::check(
        &syn::parse2(scheduled.to_token_stream()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        cq_lang_datalog_strata::invariants::erase(&after).unwrap(),
        before
    );
}

#[test]
fn helper_ownership_follows_declared_column_types() {
    for (ty, expected) in [
        ("i64", "*x"),
        ("::core::primitive::u32", "*x"),
        ("bool", "*x"),
        ("char", "*x"),
        ("&'static str", "*x"),
        ("String", "(*x).clone()"),
        ("UserType", "(*x).clone()"),
    ] {
        let expression = cq_lang_datalog_source::invariants::owned_column(
            &syn::parse_quote!(x),
            &syn::parse_str(ty).unwrap(),
        );
        assert_eq!(expression, syn::parse_str::<syn::Expr>(expected).unwrap());
    }
    let after = invariants::compile(&stratify::compile(&source()).unwrap()).unwrap();
    assert_eq!(
        after.source.rules.last().unwrap().head.args[0],
        syn::parse_quote!(*y)
    );
    let mut forged = after;
    forged.source.rules.last_mut().unwrap().head.args[0] = syn::parse_quote!((*y).clone());
    assert!(contract::check(&forged).is_err());
}

#[test]
fn forged_filter_binding_schema_and_scc_are_rejected() {
    let before = stratify::compile(&source()).unwrap();
    let good = invariants::compile(&before).unwrap();
    let mut bad = good.clone();
    bad.source.rules.last_mut().unwrap().body.pop();
    assert!(contract::check(&bad).is_err());
    let mut bad = good.clone();
    bad.source.rules.last_mut().unwrap().head.args[0] = syn::parse_quote!((*z).clone());
    assert!(contract::check(&bad).is_err());
    let mut bad = good.clone();
    bad.source.declarations.last_mut().unwrap().columns[0].ty = syn::parse_quote!(String);
    assert!(contract::check(&bad).is_err());
    let mut bad = good.clone();
    bad.components.reverse();
    assert!(contract::check(&bad).is_err());
    let mut bad = good.clone();
    if let BodyItem::Positive { atom } = &mut bad.source.rules[1].body[1] {
        let first = atom.args[0].clone();
        atom.args[0] = atom.args[1].clone();
        atom.args[1] = first;
    }
    assert!(contract::check(&bad).is_err());
}

#[test]
fn leaves_drivers_unfiltered_atoms_opaque_clauses_and_expanding_joins_alone() {
    for body in [
        "Reach(*x,*z) :- Edge(y,z), !Block(z), Reach(x,y);",
        "Reach(*x,*z) :- Reach(x,y), Edge(y,z);",
        "Reach(*x,*z) :- Reach(x,y), Edge(y,z), Edge(z,w), !Block(w);",
        "Reach(*x,*z) :- Reach(x,y), Edge(y,z), where z > 0, !Block(z);",
        "Reach(*x,*z) :- Reach(x,y), Edge(y,z), !Block(x);",
    ] {
        let mut p = source();
        p.rules[1] = syn::parse_str(body).unwrap();
        let p = stratify::compile(&p).unwrap();
        assert!(
            invariants::compile(&p)
                .unwrap()
                .source
                .invariants
                .is_empty(),
            "{body}"
        );
    }
}

#[test]
fn frozen_lower_strata_are_materialized_before_the_consumer() {
    let p: Program = syn::parse_quote! {
        struct Lower;
        relation Edge(x:i32,y:i32) = (edges);
        relation Block(x:i32) = (blocked);
        relation Frozen(x:i32,y:i32);
        relation Reach(x:i32,y:i32);
        output Reach;
        Frozen(*x,*y) :- Edge(x,y);
        Reach(*x,*y) :- Edge(x,y);
        Reach(*x,*z) :- Reach(x,y), Frozen(y,z), !Block(z);
    };
    let after = invariants::compile(&stratify::compile(&p).unwrap()).unwrap();
    assert_eq!(after.source.invariants.len(), 1);
    contract::check(&after).unwrap();
}

#[test]
fn maximal_filter_includes_bound_negations_before_the_frozen_atom() {
    let mut p = source();
    p.rules[1] = syn::parse_quote! {
        Reach(*x,*z) :- Reach(x,y), !Block(y), Edge(y,z), !Block(z);
    };
    let before = stratify::compile(&p).unwrap();
    let after = invariants::compile(&before).unwrap();
    assert_eq!(after.source.invariants.len(), 1);
    assert_eq!(after.source.rules.last().unwrap().body.len(), 3);
    invariants::check(&before, &after).unwrap();
    let mut omitted = after.clone();
    omitted.source.rules.last_mut().unwrap().body.pop();
    assert!(contract::check(&omitted).is_err());
}

fn benchmark_program(text: &str) -> Program {
    use proc_macro2::{TokenStream, TokenTree};
    fn find(tokens: TokenStream) -> Option<Program> {
        let tokens: Vec<_> = tokens.into_iter().collect();
        for window in tokens.windows(3) {
            if let [
                TokenTree::Ident(name),
                TokenTree::Punct(bang),
                TokenTree::Group(body),
            ] = window
                && name == "datalog"
                && bang.as_char() == '!'
            {
                let mut body = body.stream().into_iter().peekable();
                if matches!(body.peek(), Some(TokenTree::Ident(name)) if name == "pipeline") {
                    body.next();
                    assert!(matches!(body.next(), Some(TokenTree::Group(_))));
                }
                return Some(syn::parse2(body.collect()).unwrap());
            }
        }
        tokens.into_iter().find_map(|token| match token {
            TokenTree::Group(group) => find(group.stream()),
            _ => None,
        })
    }
    find(text.parse().unwrap()).expect("benchmark must contain a Datalog invocation")
}

#[test]
fn ddisasm_selects_only_the_recursive_register_filter() {
    let source = benchmark_program(include_str!("../../../ddisasm/src/mini_engine.rs"));
    let before = stratify::compile(&source).unwrap();
    let after = invariants::compile(&before).unwrap();
    assert_eq!(
        after
            .source
            .invariants
            .iter()
            .map(|h| h.rule)
            .collect::<Vec<_>>(),
        [8]
    );
    invariants::check(&before, &after).unwrap();
    seminaive::compile(&after).unwrap();
}

#[test]
fn doop_invariant_selection_is_checked_without_blanket_join_materialization() {
    let source = benchmark_program(include_str!("../../../doop/shared/src/mini_engine.rs"));
    let before = stratify::compile(&source).unwrap();
    let after = invariants::compile(&before).unwrap();
    eprintln!(
        "DOOP invariant rules: {:?}",
        after
            .source
            .invariants
            .iter()
            .map(|h| h.rule)
            .collect::<Vec<_>>()
    );
    invariants::check(&before, &after).unwrap();
    seminaive::compile(&after).unwrap();
}
