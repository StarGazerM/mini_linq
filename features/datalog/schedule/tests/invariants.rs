use cq_datalog_schedule::{invariants, seminaive, stratify};
use cq_lang_datalog::{BodyItem, Program, contract};
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
    assert_eq!(after.invariants.len(), 1);
    assert_eq!(after.invariants[0].rule, 1);
    let parsed: Program = syn::parse2(after.to_token_stream()).unwrap();
    assert_eq!(parsed, after);
    invariants::check(&before, &parsed).unwrap();
    let scheduled = seminaive::compile(&parsed).unwrap();
    contract::check(&syn::parse2(scheduled.to_token_stream()).unwrap()).unwrap();
    assert_eq!(cq_lang_datalog::invariants::erase(&after).unwrap(), before);
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
        let expression = cq_lang_datalog::invariants::owned_column(
            &syn::parse_quote!(x),
            &syn::parse_str(ty).unwrap(),
        );
        assert_eq!(expression, syn::parse_str::<syn::Expr>(expected).unwrap());
    }
    let after = invariants::compile(&stratify::compile(&source()).unwrap()).unwrap();
    assert_eq!(
        after.rules.last().unwrap().head.args[0],
        syn::parse_quote!(*y)
    );
    let mut forged = after;
    forged.rules.last_mut().unwrap().head.args[0] = syn::parse_quote!((*y).clone());
    assert!(contract::check(&forged).is_err());
}

#[test]
fn forged_filter_binding_schema_and_scc_are_rejected() {
    let before = stratify::compile(&source()).unwrap();
    let good = invariants::compile(&before).unwrap();
    let mut bad = good.clone();
    bad.rules.last_mut().unwrap().body.pop();
    assert!(contract::check(&bad).is_err());
    let mut bad = good.clone();
    bad.rules.last_mut().unwrap().head.args[0] = syn::parse_quote!((*z).clone());
    assert!(contract::check(&bad).is_err());
    let mut bad = good.clone();
    bad.declarations.last_mut().unwrap().columns[0].ty = syn::parse_quote!(String);
    assert!(contract::check(&bad).is_err());
    let mut bad = good.clone();
    bad.strata.as_mut().unwrap().reverse();
    assert!(contract::check(&bad).is_err());
    let mut bad = good.clone();
    if let BodyItem::Positive { atom } = &mut bad.rules[1].body[1] {
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
        "Reach(*x,*z) :- Reach(x,y), Edge(y,z), if *z > 0, !Block(z);",
        "Reach(*x,*z) :- Reach(x,y), Edge(y,z), !Block(x);",
    ] {
        let mut p = source();
        p.rules[1] = syn::parse_str(body).unwrap();
        let p = stratify::compile(&p).unwrap();
        assert!(
            invariants::compile(&p).unwrap().invariants.is_empty(),
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
    assert_eq!(after.invariants.len(), 1);
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
    assert_eq!(after.invariants.len(), 1);
    assert_eq!(after.rules.last().unwrap().body.len(), 3);
    invariants::check(&before, &after).unwrap();
    let mut omitted = after.clone();
    omitted.rules.last_mut().unwrap().body.pop();
    assert!(contract::check(&omitted).is_err());
}

fn benchmark_program(text: &str) -> Program {
    struct Visitor(Option<Program>);
    impl<'a> syn::visit::Visit<'a> for Visitor {
        fn visit_macro(&mut self, node: &'a syn::Macro) {
            if node.path.segments.last().unwrap().ident == "datalog" {
                self.0 = Some(syn::parse2(node.tokens.clone()).unwrap());
            }
        }
    }
    let mut visitor = Visitor(None);
    syn::visit::Visit::visit_file(&mut visitor, &syn::parse_file(text).unwrap());
    visitor.0.unwrap()
}

#[test]
fn ddisasm_selects_only_the_recursive_register_filter() {
    let source = benchmark_program(include_str!("../../ddisasm/src/mini_engine.rs"));
    let before = stratify::compile(&source).unwrap();
    let after = invariants::compile(&before).unwrap();
    assert_eq!(
        after.invariants.iter().map(|h| h.rule).collect::<Vec<_>>(),
        [8]
    );
    invariants::check(&before, &after).unwrap();
    seminaive::compile(&after).unwrap();
}

#[test]
fn doop_invariant_selection_is_checked_without_blanket_join_materialization() {
    let source = benchmark_program(include_str!("../../doop/src/mini_engine.rs"));
    let before = stratify::compile(&source).unwrap();
    let after = invariants::compile(&before).unwrap();
    eprintln!(
        "DOOP invariant rules: {:?}",
        after.invariants.iter().map(|h| h.rule).collect::<Vec<_>>()
    );
    invariants::check(&before, &after).unwrap();
    seminaive::compile(&after).unwrap();
}
