use cq_syntax::algebra::{Bind, BlockOrigin, disjoint_unions};
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
fn only_fallback_construction_proves_union_disjointness() {
    let union: syn::Ident = syn::parse_quote!(union);
    // Literal values are never decoded: an origin-free union or a union block
    // is canonicalized whatever literals its arms select.
    assert!(disjoint_unions([(&union, None)]).is_empty());
    let block: BlockOrigin = syn::parse_quote!(block @0 free [] interface [tag]);
    assert!(disjoint_unions([(&union, Some(&block))]).is_empty());
    let fallback: BlockOrigin = syn::parse_quote!(block @0 free [] interface [tag] fallback);
    assert!(disjoint_unions([(&union, Some(&fallback))]).contains("union"));
}

#[test]
fn occurrence_constants_round_trip_and_require_literal_values() {
    use cq_syntax::algebra::Rename;
    for text in [
        "rename R@0 {a -> x} constants {b = 7, c = -2}",
        "rename R@0 {} constants {a = true, b = \"fixed\"}",
        "rename R@0 {a -> x}",
    ] {
        let rename: Rename = syn::parse_str(text).unwrap();
        if let Some(constants) = &rename.constants {
            constants.check().unwrap();
        }
        assert_eq!(
            syn::parse2::<Rename>(rename.to_token_stream()).unwrap(),
            rename
        );
    }
    for text in [
        "constants {}",
        "constants {a = x}",
        "constants {a = 7, a = 8}",
    ] {
        let constants: cq_syntax::algebra::OccurrenceConstants = syn::parse_str(text).unwrap();
        assert!(constants.check().is_err());
    }
    assert!(syn::parse_str::<Rename>("rename R@0 {} constants {a = f()}").is_err());
}

#[test]
fn normalized_atoms_accept_literals_but_not_repeated_variables_or_expressions() {
    for text in ["R(x,7,-2)", "R(7,true,\"fixed\",7)", "R()"] {
        assert!(
            syn::parse_str::<cq_syntax::Atom>(text)
                .unwrap()
                .is_normalized()
        );
    }
    for text in ["R(x,x)", "R(x,_)", "R(x,f())"] {
        assert!(
            !syn::parse_str::<cq_syntax::Atom>(text)
                .unwrap()
                .is_normalized()
        );
    }
}
