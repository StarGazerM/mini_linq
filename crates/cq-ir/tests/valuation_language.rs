//! Normal form and valuation plans: normalization moves constants, repeats,
//! wildcards and expression arguments out of atoms; a plan schedules the
//! normal query and is checked against it; kernels compare up to renaming.
use cq_ir::cq::{self, normal::Kernel};
use cq_ir::valuation;
use quote::{ToTokens, quote};

fn cq(tokens: proc_macro2::TokenStream) -> cq::Module {
    syn::parse2(tokens).unwrap()
}

#[test]
fn normalization_moves_every_non_variable_argument_out_of_atoms() {
    let source = cq(quote! {
        struct Q;
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(x, z) :- R(x, 5), S(_, z), R(x, x), if z > 1;
    });
    let normal = cq::normal::normalize(&source).unwrap();
    let text = normal.program.query.to_token_stream().to_string();
    // The constant becomes a join key bound before every atom.
    assert!(
        text.starts_with("answer (x , z) : - let __cq_0_1 : i32 = 5 , R (x , __cq_0_1)"),
        "{text}"
    );
    // The wildcard is a distinct witness; the repeat gets an equality guard.
    assert!(
        text.contains("S (__cq_1_0 , z) , R (x , __cq_2_1) , if __cq_2_1 == x , if z > 1"),
        "{text}"
    );
    cq::normal::check(&normal).unwrap();
    assert!(cq::normal::check(&source).is_err());
    // Normalizing again is the identity.
    assert_eq!(cq::normal::normalize(&normal).unwrap(), normal);
}

#[test]
fn kernels_compare_up_to_renaming_of_variables() {
    let one = cq(quote! {
        struct Q; relation R(a: i32, b: i32);
        answer(x) :- R(x, y), if y > 1;
    });
    let two = cq(quote! {
        struct Q; relation R(a: i32, b: i32);
        answer(p) :- R(p, q), if q > 1;
    });
    let three = cq(quote! {
        struct Q; relation R(a: i32, b: i32);
        answer(p) :- R(q, p), if q > 1;
    });
    assert_eq!(Kernel::of(&one), Kernel::of(&two));
    assert_ne!(Kernel::of(&one), Kernel::of(&three));
}

fn plan(steps: proc_macro2::TokenStream) -> syn::Result<valuation::Module> {
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
fn coarse_and_fine_plans_of_the_triangle_check_and_erase_to_the_query() {
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
    assert_eq!(analysis.bound_before[3], ["a"]);
    assert_eq!(analysis.introduces[3], ["b"]);
    assert_eq!(valuation::kernel(&coarse), valuation::kernel(&fine));
    assert_eq!(
        valuation::kernel(&fine),
        Kernel::of(&valuation::erase(&fine))
    );
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
        "`a` is bound; a subatom may drop only unbound columns",
    );
    rejects(
        quote! { R@0(a, _); if a > 0; S@1(_, c); T@2(a, c); let d: i32 = a + c; },
        "never binds `b`",
    );
    rejects(
        quote! { if a > 0; R@0(a, b); S@1(b, c); let d: i32 = a + c; T@2(a, c); },
        "before it is bound",
    );
    rejects(
        quote! { R@0(a, b); if a > 0; S@1(b, c); T@2(a, c); },
        "every binding must be placed",
    );
    rejects(
        quote! { R@0(a, b); if a > 0; S@1(b, c); let d: i32 = a + c; S@2(a, c); },
        "clause #2 is `T`",
    );
}
