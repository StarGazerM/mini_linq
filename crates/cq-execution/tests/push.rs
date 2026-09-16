use cq_ir::rust_access_plan;
use quote::ToTokens;

#[test]
fn push_preserves_control_scope_and_uses_one_exact_set() {
    let source: rust_access_plan::Plan = syn::parse_quote! {
        out(x, y) => ((x, y)) :-
        R(x) => for (x,) in (rows),
        Keep(x) => if (keep(&x)),
        S(x, y) => for (y,) in (lookup(&x));
    };
    let output = cq_execution::push::compile(&source).unwrap();
    let expected: syn::Expr = syn::parse_quote! {{
        let mut __cq_output = ::std::collections::HashSet::new();
        for (x,) in (rows) {
            if keep(&x) {
                for (y,) in (lookup(&x)) {
                    let x = ::core::clone::Clone::clone(&x);
                    __cq_output.insert((x, y));
                }
            }
        }
        __cq_output
    }};
    assert_eq!(output, expected);
    let text = output.to_token_stream().to_string();
    assert_eq!(text.matches("HashSet").count(), 1);
    assert!(!text.contains("Iterator"));
    assert!(!text.contains("Vec"));
}

#[test]
fn push_checks_access_before_emitting_rust() {
    let invalid: rust_access_plan::Plan = syn::parse_quote! {
        out(x) => (x) :- R(x) => if (true);
    };
    assert!(
        cq_execution::push::compile(&invalid)
            .unwrap_err()
            .to_string()
            .contains("fresh variables")
    );
}
