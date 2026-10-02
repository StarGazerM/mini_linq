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
