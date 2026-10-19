use cq_syntax::algebra::{Bind, LiteralStep, disjoint_unions};
use quote::ToTokens;

#[test]
fn generator_and_bind_round_trip_preserve_kind() {
    let generator: cq_syntax::For = syn::parse_quote!(for (x, y) in pairs(seed));
    assert_eq!(
        syn::parse2::<cq_syntax::For>(generator.to_token_stream()).unwrap(),
        generator
    );
    assert!(generator.binding().annotation.is_none());
    for text in [
        "bind input with x = (value) scope [seed]",
        "bind input with for (x, y) = (pairs(seed)) scope [seed]",
    ] {
        let bind: Bind = syn::parse_str(text).unwrap();
        assert_eq!(bind.for_token.is_some(), text.contains("with for"));
        assert_eq!(syn::parse2::<Bind>(bind.to_token_stream()).unwrap(), bind);
    }
}

#[test]
fn generator_preserves_input_constants_without_scalar_literal_inference() {
    let root = syn::parse_quote!(root);
    let left = syn::parse_quote!(left);
    let right = syn::parse_quote!(right);
    let a: syn::Ident = syn::parse_quote!(a);
    let b: syn::Ident = syn::parse_quote!(b);
    let union = syn::parse_quote!(union);
    let operands = [a.clone(), b.clone()].into_iter().collect();
    let left_bind: Bind = syn::parse_quote!(bind root with tag = (0));
    let right_bind: Bind = syn::parse_quote!(bind root with tag = (1));
    let left_generator: Bind = syn::parse_quote!(bind left with for x = (items));
    let right_generator: Bind = syn::parse_quote!(bind right with for x = (items));
    let result = disjoint_unions([
        (&root, LiteralStep::Empty),
        (&left, LiteralStep::Bind(&left_bind)),
        (&right, LiteralStep::Bind(&right_bind)),
        (&a, LiteralStep::Bind(&left_generator)),
        (&b, LiteralStep::Bind(&right_generator)),
        (&union, LiteralStep::Union(&operands, None)),
    ]);
    assert!(result.contains("union"));

    // Even syntactically literal generator expressions must not be scalar facts;
    // their IntoIterator validity is Rust's responsibility, not this fold's.
    let left_generator: Bind = syn::parse_quote!(bind root with for x = (0));
    let right_generator: Bind = syn::parse_quote!(bind root with for x = (1));
    let result = disjoint_unions([
        (&root, LiteralStep::Empty),
        (&a, LiteralStep::Bind(&left_generator)),
        (&b, LiteralStep::Bind(&right_generator)),
        (&union, LiteralStep::Union(&operands, None)),
    ]);
    assert!(!result.contains("union"));
}
