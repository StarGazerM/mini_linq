//! Complete valuation schedules over normal CQ programs.
use cq_lang_vo as valuation;
use quote::{ToTokens, quote};

fn plan(steps: proc_macro2::TokenStream) -> syn::Result<valuation::Program> {
    syn::parse2(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        relation T(a: i32, c: i32);
        answer(a, b, c) :- R(a, b), S(b, c), T(a, c), if a > 0, let d: i32 = a + c;
        valuation { #steps }
    })
}

#[test]
fn complete_coarse_and_fine_plans_check_and_round_trip() {
    let coarse = plan(quote! {
        R@0(a, b); if a > 0; S@1(b, c); let d: i32 = a + c; T@2(a, c);
    })
    .unwrap();
    let analysis = valuation::contract::check(&coarse).unwrap();
    assert_eq!(analysis.introduces[0], ["a", "b"]);
    assert_eq!(analysis.introduces[4], Vec::<String>::new());
    let fine = plan(quote! {
        R@0(a, _); T@2(a, _); if a > 0;
        R@0(a, b); S@1(b, _);
        S@1(b, c); T@2(a, c); let d: i32 = a + c;
    })
    .unwrap();
    let analysis = valuation::contract::check(&fine).unwrap();
    assert_eq!(
        syn::parse2::<valuation::Program>(fine.to_token_stream()).unwrap(),
        fine
    );
    assert_eq!(analysis.bound_before[3], ["a"]);
    assert_eq!(analysis.introduces[3], ["b"]);
}

#[test]
fn contract_rejects_dropped_bound_columns_uncovered_clauses_and_early_guards() {
    let rejects = |steps, needle: &str| {
        let error = plan(steps)
            .and_then(|m| valuation::contract::check(&m))
            .expect_err("rejected")
            .to_string();
        assert!(error.contains(needle), "{error}");
    };
    rejects(
        quote! { R@0(a, b); if a > 0; S@1(b, c); let d: i32 = a + c; T@2(_, c); },
        "T",
    );
    rejects(
        quote! { R@0(a, _); if a > 0; S@1(_, c); T@2(a, c); let d: i32 = a + c; },
        "b",
    );
    rejects(
        quote! { if a > 0; R@0(a, b); S@1(b, c); let d: i32 = a + c; T@2(a, c); },
        "a",
    );
    rejects(quote! { R@0(a, b); if a > 0; S@1(b, c); T@2(a, c); }, "d");
    rejects(
        quote! { R@0(a, b); if a > 0; S@1(b, c); let d: i32 = a + c; S@2(a, c); },
        "T",
    );
}

#[test]
fn reductions_precede_the_reduced_clause_and_place_a_guard_once() {
    let reduced = plan(quote! {
        reduce R@0 with T@2(a, _); reduce R@0 if a > 0; reduce T@2 with R@0(a, _);
        R@0(a, b); S@1(b, c); let d: i32 = a + c; T@2(a, c);
    })
    .unwrap();
    let analysis = valuation::contract::check(&reduced).unwrap();
    // Reductions bind nothing; the guard they place is no step.
    assert_eq!(analysis.bound_before[3], Vec::<String>::new());
    assert!(analysis.introduces[..3].iter().all(Vec::is_empty));
    assert_eq!(analysis.introduces[3], ["a", "b"]);
    // A later guard step is the next guard the reductions leave.
    let twice = plan(quote! {
        R@0(a, b); if a > 0; S@1(b, c); let d: i32 = a + c; if a > 0; T@2(a, c);
    });
    assert!(twice.is_err() || valuation::contract::check(&twice.unwrap()).is_err());
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
        quote! { R@0(a, b); reduce R@0 with T@2(a, _); if a > 0; S@1(b, c); let d: i32 = a + c; T@2(a, c); },
        "R",
    );
    rejects(
        quote! { reduce R@0 with T@2(a, c); R@0(a, b); if a > 0; S@1(b, c); let d: i32 = a + c; T@2(a, c); },
        "T",
    );
    rejects(
        quote! { reduce R@0 with T@2(_, _); R@0(a, b); if a > 0; S@1(b, c); let d: i32 = a + c; T@2(a, c); },
        "T",
    );
    rejects(
        quote! { reduce R@0 with R@0(a, b); R@0(a, b); if a > 0; S@1(b, c); let d: i32 = a + c; T@2(a, c); },
        "R",
    );
    rejects(
        quote! { reduce S@1 if a > 0; R@0(a, b); S@1(b, c); let d: i32 = a + c; T@2(a, c); },
        "a",
    );
    rejects(
        quote! { reduce R@0 if a > 1; R@0(a, b); if a > 0; S@1(b, c); let d: i32 = a + c; T@2(a, c); },
        "a",
    );
    rejects(
        quote! { reduce R@0 if a > 0; R@0(a, b); if a > 0; S@1(b, c); let d: i32 = a + c; T@2(a, c); },
        "a",
    );
    rejects(
        quote! { reduce S@0 with T@2(a, _); R@0(a, b); if a > 0; S@1(b, c); let d: i32 = a + c; T@2(a, c); },
        "R",
    );
}
