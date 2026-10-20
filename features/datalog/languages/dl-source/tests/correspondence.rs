use cq_lang_datalog_source::{Program, check_rewrite, contract};
use quote::quote;
type Mutation = Box<dyn Fn(&mut Program)>;
const SOURCE: &str = include_str!("fixtures/source.datalog");
#[test]
fn unsafe_unstratified_modes_and_optional_syntax_are_rejected() {
    for source in [
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- E(x), !A(x);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); relation B(x:i32); output A; A(*x) :- E(x), !B(x); B(*x) :- A(x);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*y) :- E(x);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- E(x), !E(y);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- !E(x), E(x);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- E(x), !E(*x+1);",
        "struct P; relation E(x:i32) = (e); relation A(x:i32); output A; A(*x) :- E(x), neg!{E(x)};",
        "struct P; relation E(x:i32) = (e); output E; E(*x) :- E(x);",
    ] {
        let program: Program = syn::parse_str(source).unwrap();
        assert!(contract::check(&program).is_err(), "{source}");
    }
    // Frozen core has no relation modes; these fail during parsing, not checking.
    assert!(
        syn::parse_str::<Program>(
            "struct P; relation E(x:i32) parameter; relation A(x:i32); output A; A(*x) :- E(x);"
        )
        .is_err()
    );
}

#[test]
fn negation_rejects_keys_bound_only_by_let() {
    for binding in [quote!(let y = *x), quote!(let y:i32 = *x)] {
        let program: Program = syn::parse2(quote! {
            struct OwnedKey;
            relation D(x:i32) = (domain);
            relation R(x:i32) = (blocked);
            relation Out(x:i32);
            output Out;
            Out(*x) :- D(x), #binding, !R(y);
        })
        .unwrap();
        let error = contract::check(&program).unwrap_err().to_string();
        assert!(error.contains("bound by a positive atom"), "{error}");
    }
}

#[test]
fn rewrites_preserve_source_name_schemas_rules_output_and_observer() {
    let original: Program = syn::parse_str(SOURCE).unwrap();
    let mutations: Vec<Mutation> = vec![
        Box::new(|p| p.name = syn::parse_quote!(Renamed)),
        Box::new(|p| p.outputs = vec![syn::parse_quote!(Reach)]),
        Box::new(|p| p.observer = Some(syn::parse_quote!(observe_round))),
        Box::new(|p| p.declarations[0].columns[0].ty = syn::parse_quote!(i64)),
        Box::new(|p| p.rules.swap(0, 1)),
        Box::new(|p| p.rules[0].head.args[0] = syn::parse_quote!(*y)),
    ];
    for (index, mutate) in mutations.into_iter().enumerate() {
        let mut changed = original.clone();
        mutate(&mut changed);
        contract::check(&changed).unwrap();
        assert!(
            check_rewrite(&original, &changed).is_err(),
            "mutation {index}"
        );
    }
}

#[test]
fn generated_bindings_follow_let_negation_scope_restrictions() {
    let program: Program = syn::parse_quote! {
        struct GeneratedKey;
        relation D(x:i32) = (domain);
        relation R(x:i32) = (blocked);
        relation Out(x:i32);
        output Out;
        Out(y) :- D(x), for y in [*x], !R(y);
    };
    let error = contract::check(&program).unwrap_err().to_string();
    assert!(error.contains("bound by a positive atom"), "{error}");
}
