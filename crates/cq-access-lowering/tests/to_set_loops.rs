use cq_ir::rust_access_plan;
use quote::ToTokens;

#[test]
fn push_preserves_control_scope_and_uses_one_exact_set() {
    let source: rust_access_plan::Plan = syn::parse_quote! {
        out(x, y) => ((x, y)) :-
        R(x) => bind (x,) <- (rows),
        Keep(x) => if (keep(&x)),
        S(x, y) => bind (y,) <- (lookup(&x));
    };
    let output = cq_access_lowering::to_set_loops::compile(&source).unwrap();
    let expected: syn::Expr = syn::parse_quote! {{
        let mut __cq_output = mini_linq::set_sink! { operator out; new };
        for (x,) in (rows) {
            if keep(&x) {
                for (y,) in (lookup(&x)) {
                    let x = ::core::clone::Clone::clone(&x);
                    mini_linq::set_sink! { operator out; insert(__cq_output, (x, y)) };
                }
            }
        }
        mini_linq::set_sink! { operator out; finish(__cq_output) }
    }};
    assert_eq!(output, expected);
    let text = output.to_token_stream().to_string();
    assert!(!text.contains("HashSet"));
    assert_eq!(text.matches("set_sink").count(), 3);
    assert!(!text.contains("Iterator"));
    assert!(!text.contains("Vec"));
}

#[test]
fn push_checks_access_before_emitting_rust() {
    let invalid: rust_access_plan::Plan = syn::parse_quote! {
        out(x) => (x) :- R(x) => if (true);
    };
    assert!(
        cq_access_lowering::to_set_loops::compile(&invalid)
            .unwrap_err()
            .to_string()
            .contains("fresh variables")
    );
}
