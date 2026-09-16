//! Literal HashSet/HashMap shape checks for the two indexed representations.
use super::*;

#[cfg(any(feature = "hash-prefix", feature = "hash-materialized"))]
pub fn tuple_set_type(local: &syn::Local, types: &[syn::Type]) -> Option<()> {
    let syn::Pat::Type(p) = &local.pat else {
        return None;
    };
    (p.ty.as_ref() == &syn::parse_quote!(::std::collections::HashSet<(#(#types,)*)>)).then_some(())
}

/// Check the literal collection type and binding pattern, returning its key type.
/// Tuple-key versus inferred-key requirements belong to the concrete caller.
#[cfg(any(feature = "hash-prefix", feature = "hash-materialized"))]
pub fn collection_key_type<'a>(
    local: &'a syn::Local,
    index: &Ident,
    payload_arity: Option<usize>,
) -> Option<&'a syn::Type> {
    let syn::Pat::Type(typed) = &local.pat else {
        return None;
    };
    let expected: syn::Path = if payload_arity.is_some() {
        syn::parse_quote!(::std::collections::HashMap)
    } else {
        syn::parse_quote!(::std::collections::HashSet)
    };
    let syn::Type::Path(actual) = typed.ty.as_ref() else {
        return None;
    };
    let actual_path: Vec<_> = actual.path.segments.iter().map(|s| &s.ident).collect();
    let expected_path: Vec<_> = expected.segments.iter().map(|s| &s.ident).collect();
    if actual.qself.is_some()
        || actual.path.leading_colon != expected.leading_colon
        || actual_path != expected_path
    {
        return None;
    }
    let pattern: syn::Pat = if payload_arity.is_some() {
        syn::parse_quote!(mut #index)
    } else {
        syn::parse_quote!(#index)
    };
    if typed.pat.as_ref() != &pattern {
        return None;
    }
    let syn::PathArguments::AngleBracketed(arguments) = &actual.path.segments.last()?.arguments
    else {
        return None;
    };
    if arguments.args.len() != if payload_arity.is_some() { 2 } else { 1 } {
        return None;
    }
    let syn::GenericArgument::Type(key_type) = &arguments.args[0] else {
        return None;
    };
    if let Some(arity) = payload_arity {
        let syn::GenericArgument::Type(syn::Type::Path(value)) = &arguments.args[1] else {
            return None;
        };
        let path: Vec<_> = value
            .path
            .segments
            .iter()
            .map(|s| symbol_name(&s.ident))
            .collect();
        if value.qself.is_some()
            || value.path.leading_colon.is_none()
            || path != ["std", "vec", "Vec"]
        {
            return None;
        }
        let syn::PathArguments::AngleBracketed(element) = &value.path.segments.last()?.arguments
        else {
            return None;
        };
        if element.args.len() != 1 {
            return None;
        }
        let syn::GenericArgument::Type(syn::Type::Tuple(tuple)) = &element.args[0] else {
            return None;
        };
        if tuple.elems.len() != arity {
            return None;
        }
    }
    Some(key_type)
}
