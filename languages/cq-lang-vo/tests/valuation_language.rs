//! Complete valuation schedules over normal CQ programs.
use cq_lang_vo as valuation;
use quote::{ToTokens, quote};

fn plan(steps: proc_macro2::TokenStream) -> syn::Result<valuation::Program> {
    syn::parse2(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation T(a: i32, c: i32);
        answer(a, b, c) :- R(a, b), S(b, c), T(a, c), if { a > 0 }, let d: i32 = a + c;
        valuation { #steps }
    })
}

#[test]
fn complete_coarse_and_fine_plans_check_and_round_trip() {
    let coarse = plan(quote! {
        R@0(a, b); S@1(b, c); T@2(a, c); if { a > 0 }; let d: i32 = a + c;
    })
    .unwrap();
    let analysis = valuation::contract::check(&coarse).unwrap();
    assert_eq!(analysis.introduces[0], ["a", "b"]);
    assert_eq!(analysis.introduces[2], Vec::<String>::new());
    let fine = plan(quote! {
        R@0(_, b); R@0(a, b); S@1(b, c); T@2(a, c);
        if { a > 0 }; let d: i32 = a + c;
    })
    .unwrap();
    let analysis = valuation::contract::check(&fine).unwrap();
    assert_eq!(
        syn::parse2::<valuation::Program>(fine.to_token_stream()).unwrap(),
        fine
    );
    assert_eq!(analysis.bound_before[1], ["b"]);
    assert_eq!(analysis.introduces[1], ["a"]);
}

#[test]
fn contract_rejects_dropped_columns_missing_steps_and_early_rust() {
    for steps in [
        quote! { R@0(a,b); S@1(b,c); T@2(_,c); if { a > 0 }; let d:i32 = a+c; },
        quote! { R@0(a,_); S@1(_,c); T@2(a,c); if { a > 0 }; let d:i32 = a+c; },
        quote! { if { a > 0 }; R@0(a,b); S@1(b,c); T@2(a,c); let d:i32 = a+c; },
        quote! { R@0(a,b); S@1(b,c); T@2(a,c); if { a > 0 }; },
        quote! { R@0(a,b); S@1(b,c); S@2(a,c); if { a > 0 }; let d:i32 = a+c; },
    ] {
        assert!(valuation::contract::check(&plan(steps).unwrap()).is_err());
    }
}

#[test]
fn reductions_are_relational_and_never_consume_a_rust_guard() {
    let reduced = plan(quote! {
        reduce R@0 with T@2(a, _); reduce T@2 with R@0(a, _);
        R@0(a,b); S@1(b,c); T@2(a,c); if { a > 0 }; let d:i32 = a+c;
    })
    .unwrap();
    let analysis = valuation::contract::check(&reduced).unwrap();
    assert!(analysis.introduces[..2].iter().all(Vec::is_empty));
    assert!(plan(quote! { reduce R@0 if { a > 0 }; }).is_err());
}

#[test]
fn contract_rejects_late_misaligned_and_foreign_reductions() {
    let rejects = |steps, needle: &str| {
        let error = plan(steps)
            .and_then(|m| valuation::contract::check(&m))
            .expect_err("rejected")
            .to_string();
        assert!(error.contains(needle), "{error}");
    };
    rejects(
        quote! { R@0(a, b); reduce R@0 with T@2(a, _); if { a > 0 }; S@1(b, c); let d: i32 = a + c; T@2(a, c); },
        "R",
    );
    rejects(
        quote! { reduce R@0 with T@2(a, c); R@0(a, b); if { a > 0 }; S@1(b, c); let d: i32 = a + c; T@2(a, c); },
        "T",
    );
    rejects(
        quote! { reduce R@0 with T@2(_, _); R@0(a, b); if { a > 0 }; S@1(b, c); let d: i32 = a + c; T@2(a, c); },
        "T",
    );
    rejects(
        quote! { reduce R@0 with R@0(a, b); R@0(a, b); if { a > 0 }; S@1(b, c); let d: i32 = a + c; T@2(a, c); },
        "R",
    );
    rejects(
        quote! { reduce S@0 with T@2(a, _); R@0(a, b); if { a > 0 }; S@1(b, c); let d: i32 = a + c; T@2(a, c); },
        "R",
    );
}

#[test]
fn generators_round_trip_and_require_their_free_variables() {
    let source: valuation::Program = syn::parse2(quote! {
        struct Q; relation R(a: i32); relation answer(b: i32);
        answer(b) :- R(a), for b in 0..a, let c = b;
        valuation { R@0(a); for b in 0..a; let c = b; }
    })
    .unwrap();
    valuation::contract::check(&source).unwrap();
    assert_eq!(
        syn::parse2::<valuation::Program>(source.to_token_stream()).unwrap(),
        source
    );
    for index in [0, 2] {
        let mut changed = source.clone();
        changed.valuation.steps.swap(1, index);
        assert!(valuation::contract::check(&changed).is_err());
    }
    let mut changed = source.clone();
    changed.valuation.steps[1] = syn::parse2(quote!(let b = 0..a;)).unwrap();
    assert!(valuation::contract::check(&changed).is_err());
    let mut changed = source.clone();
    changed
        .valuation
        .steps
        .insert(0, syn::parse2(quote!(reduce R@0 with R@1(a);)).unwrap());
    assert!(valuation::contract::check(&changed).is_err());
}

#[test]
fn negations_round_trip_bind_nothing_and_commute_with_rust() {
    let source: valuation::Program = syn::parse_quote! {
        struct Negative; relation R(a:i32); relation K(a:i32,b:i32);
        answer(x) :- R(x), if opaque(x), !K(x,_), let y = x;
        valuation { R@0(x); !K(x,_); if opaque(x); let y = x; }
    };
    let analysis = valuation::contract::check(&source).unwrap();
    assert!(analysis.introduces[1].is_empty());
    assert_eq!(analysis.atoms.len(), 1);
    assert_eq!(
        syn::parse2::<valuation::Program>(source.to_token_stream()).unwrap(),
        source
    );
    let mut early = source.clone();
    early.valuation.steps.swap(0, 1);
    assert!(valuation::contract::check(&early).is_err());
    let mut late = source.clone();
    let negation = late.valuation.steps.remove(1);
    late.valuation.steps.push(negation);
    valuation::contract::check(&late).unwrap();
    let mut missing = source.clone();
    missing.valuation.steps.remove(1);
    assert!(valuation::contract::check(&missing).is_err());
    let mut duplicate = source.clone();
    duplicate
        .valuation
        .steps
        .insert(1, duplicate.valuation.steps[1].clone());
    assert!(valuation::contract::check(&duplicate).is_err());
    for reduction in [
        quote!(reduce R@0 with K@2(x,_);),
        quote!(reduce K@2 with R@0(x);),
    ] {
        let mut changed = source.clone();
        changed
            .valuation
            .steps
            .insert(0, syn::parse2(reduction).unwrap());
        assert!(valuation::contract::check(&changed).is_err());
    }
}
