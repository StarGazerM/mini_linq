use cq_ir::physical_context::{self as physical, Context};
use quote::{ToTokens, quote};

#[test]
fn existing_ra_and_only_free_bindings_round_trip() {
    for source in [
        quote! { q = project r keep {k}; {r(x,k) => rows {right};} },
        quote! { t = semijoin l with q; {l(x,k) => rows {left}; q(k) {keys};} },
        quote! { t = natural_join l with r; {l(x,k) => rows {left}; r(k,y) => rows {right};} },
    ] {
        let context: Context = syn::parse2(source.clone()).unwrap();
        assert_eq!(context.to_token_stream().to_string(), source.to_string());
        assert_eq!(context, syn::parse2(context.to_token_stream()).unwrap());
    }
}
#[test]
fn projection_derives_order_and_positions_without_a_left_operand() {
    let context: Context = syn::parse_quote! {
        q = project r keep {b,a}; {r(z,b,a) => rows {right};}
    };
    physical::contract::check_project(&context).unwrap();
    let facts = physical::analyze_project(&context).unwrap();
    assert_eq!(facts.columns, [2, 1]);
    assert_eq!(facts.output_layout, ["a", "b"]);
    for source in [
        quote! { q = project r keep {a,a}; {r(a) => p {r};} },
        quote! { q = project r keep {b}; {r(a) => p {r};} },
        quote! { q = project r keep {a}; {r(a) {r};} },
        quote! { q = project r keep {a}; {r(a) => p {r}; l(a) => p {l};} },
    ] {
        assert!(physical::contract::check_project(&syn::parse2(source).unwrap()).is_err());
    }
}
#[test]
fn binary_layouts_derive_keys_and_outputs_independent_of_binding_order() {
    let mut context: Context = syn::parse_quote! {
        t = natural_join l with r; {
            l(z,b,a) => rows {left}; r(b,y,a) => rows {right};
        }
    };
    for _ in 0..2 {
        let facts = physical::analyze(&context).unwrap();
        assert_eq!(facts.separator, ["a", "b"]);
        assert_eq!(facts.left_columns, [2, 1]);
        assert_eq!(facts.right_columns, [2, 0]);
        assert_eq!(facts.output_layout, ["a", "b", "y", "z"]);
        context.bindings.reverse();
    }
}
#[test]
fn reduction_requires_the_projected_resource_not_the_original_rhs() {
    for binding in [
        quote! {q(a,b) {keys};},
        quote! {q(b,a) {keys};},
        quote! {q(a,b,y) {keys};},
        quote! {q(a,b) => original_rows {right};},
        quote! {old_q(a,b) {keys};},
    ] {
        let valid = binding.to_string() == quote!(q(a,b) {keys};).to_string();
        let context: Context = syn::parse2(quote! {
            t = semijoin l with q; {l(z,b,a) => rows {left}; #binding}
        })
        .unwrap();
        assert_eq!(physical::contract::check_semijoin(&context).is_ok(), valid);
    }
}
#[test]
fn invalid_binding_domains_layouts_and_capture_are_rejected() {
    for bindings in [
        quote! {l(x) => p {l};},
        quote! {l(x) => p {l}; r(x) => p {r}; extra(x) => p {extra};},
        quote! {l(x) => p {l}; r(x) => p {r}; r#l(x) => p {other};},
        quote! {l(x,r#x) => p {l}; r(x) => p {r};},
        quote! {l(0) => p {l}; r(x) => p {r};},
        quote! {l(x+1) => p {l}; r(x) => p {r};},
        quote! {l(_) => p {l}; r(x) => p {r};},
        quote! {l((x)) => p {l}; r(x) => p {r};},
        quote! {l(crate::x) => p {l}; r(x) => p {r};},
    ] {
        let context: Context = syn::parse2(quote!(t = natural_join l with r; {#bindings})).unwrap();
        assert!(physical::contract::check_join(&context).is_err());
    }
    let context: Context =
        syn::parse_quote! {r#l = natural_join l with r; {l(x)=>p{l}; r(x)=>p{r};}};
    assert!(physical::analyze(&context).is_err());
}
#[test]
fn self_join_empty_projection_and_empty_resource_are_supported() {
    let context: Context = syn::parse_quote! {t = natural_join r with r; {r(z,x)=>p{value};}};
    physical::contract::check_join(&context).unwrap();
    let facts = physical::analyze(&context).unwrap();
    assert!(std::ptr::eq(facts.left, facts.right));
    assert_eq!(facts.left_columns, [1, 0]);
    let project: Context = syn::parse_quote! {q = project r keep {}; {r(x)=>p{value};}};
    assert!(
        physical::analyze_project(&project)
            .unwrap()
            .columns
            .is_empty()
    );
    let reduce: Context = syn::parse_quote! {t = semijoin l with q; {l(x)=>p{left}; q(){keys};}};
    physical::contract::check_semijoin(&reduce).unwrap();
    assert!(physical::analyze(&reduce).unwrap().separator.is_empty());
}
#[test]
fn obsolete_administrative_and_phase_syntax_is_not_semantic_ir() {
    for input in [
        quote! {storage cq_storage; access collect; work(&work); bindings {} t=semijoin l with r;},
        quote! {q=project r keep {k}; {r(k)=>p{r};} build keys HashSet;},
        quote! {t=semijoin l with q; {l(k)=>p{l}; q(k){keys};} reduce resource r(k)(keys);},
    ] {
        assert!(syn::parse2::<Context>(input).is_err());
    }
}
