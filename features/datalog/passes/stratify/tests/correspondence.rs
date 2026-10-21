use cq_datalog_stratify::{check, compile};
use cq_lang_datalog_source as source;
use cq_lang_datalog_strata as strata;
use quote::{ToTokens, quote};

#[test]
fn exact_sccs_dependency_order_and_source_preservation() {
    let source: source::Program = syn::parse_quote! {
        struct P;
        relation E(x:i32) = (e);
        relation A(x:i32); relation B(x:i32); relation C(x:i32);
        output C;
        A(*x) :- E(x); A(*x) :- B(x); B(*x) :- A(x);
        C(*x) :- E(x), !A(x);
    };
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    assert_eq!(
        target,
        syn::parse2::<strata::Program>(target.to_token_stream()).unwrap()
    );
    let expected: strata::Program =
        syn::parse2(quote!(#source strata { component [A,B]; component [C]; })).unwrap();
    assert_eq!(target, expected);
    for components in [
        vec![
            vec![syn::parse_quote!(A)],
            vec![syn::parse_quote!(B)],
            vec![syn::parse_quote!(C)],
        ],
        vec![vec![
            syn::parse_quote!(A),
            syn::parse_quote!(B),
            syn::parse_quote!(C),
        ]],
        vec![
            vec![syn::parse_quote!(C)],
            vec![syn::parse_quote!(A), syn::parse_quote!(B)],
        ],
        vec![],
    ] {
        let mut changed = target.clone();
        changed.components = components;
        assert!(check(&source, &changed).is_err());
    }
    let mut changed = target;
    changed.source.observer = Some(syn::parse_quote!(observer));
    assert!(check(&source, &changed).is_err());
}

#[test]
fn independent_authored_component_order_does_not_replay_selection() {
    let source: source::Program = syn::parse_quote! {
        struct P; relation E(x:i32) = (e); relation A(x:i32); relation B(x:i32);
        output A; A(*x) :- E(x); B(*x) :- E(x);
    };
    let target: strata::Program =
        syn::parse2(quote!(#source strata { component [B]; component [A]; })).unwrap();
    check(&source, &target).unwrap();
    assert_ne!(compile(&source).unwrap(), target);
}

#[test]
fn authored_coarser_partition_is_admitted_without_changing_default_selection() {
    let source: source::Program = syn::parse_quote! {
        struct Coarser;
        relation E(x:i32) = (e);
        relation A(x:i32); relation B(x:i32);
        output B;
        A(*x) :- E(x);
        B(*x) :- A(x);
    };
    let target: strata::Program = syn::parse2(quote!(#source strata { component [B,A]; })).unwrap();
    check(&source, &target).unwrap();
    let default: strata::Program =
        syn::parse2(quote!(#source strata { component [A]; component [B]; })).unwrap();
    assert_eq!(compile(&source).unwrap(), default);
}
