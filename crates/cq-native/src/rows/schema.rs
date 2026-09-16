#[cfg(any(
    feature = "rows-hash",
    feature = "rows-sorted",
    feature = "rows-tuple-hash",
    feature = "rows-tuple-sorted"
))]
use cq_ir::native::Operation;
#[cfg(any(
    feature = "rows-hash",
    feature = "rows-sorted",
    feature = "rows-tuple-hash",
    feature = "rows-tuple-sorted"
))]
use proc_macro2::TokenStream;
#[cfg(any(
    feature = "rows-hash",
    feature = "rows-sorted",
    feature = "rows-tuple-hash",
    feature = "rows-tuple-sorted"
))]
use quote::quote;

/// The supplied array representations have i64 payload columns. Keep this
/// restriction at the representation implementation, never at RA lowering.
#[cfg(any(feature = "rows-hash", feature = "rows-sorted", feature = "rows-block"))]
pub(super) fn array_width(schema: &syn::TypeTuple) -> syn::Result<usize> {
    for ty in &schema.elems {
        if ty != &syn::parse_quote!(i64)
            && ty != &syn::parse_quote!(::core::primitive::i64)
            && ty != &syn::parse_quote!(::std::primitive::i64)
        {
            return Err(syn::Error::new_spanned(
                ty,
                "this array representation requires i64 columns; select a tuple representation for heterogeneous fields",
            ));
        }
    }
    Ok(schema.elems.len())
}

/// Array syntax is selected by the array provider, outside the set algorithm.
#[cfg(any(feature = "rows-hash", feature = "rows-sorted"))]
pub(super) fn array(operation: &Operation, set: &TokenStream) -> syn::Result<Option<TokenStream>> {
    Ok(Some(match operation {
        Operation::Field { row, column, .. } => quote!((#row)[#column]),
        Operation::Record { fields, .. } => {
            let width = fields.len();
            quote!({ let row: [::core::primitive::i64; #width] = [#fields]; row })
        }
        Operation::Type { schema, .. } => {
            let width = array_width(schema)?;
            quote!(#set<[::core::primitive::i64; #width]>)
        }
        _ => return Ok(None),
    }))
}

/// Tuple syntax is selected by the tuple provider, outside the set algorithm.
#[cfg(any(feature = "rows-tuple-hash", feature = "rows-tuple-sorted"))]
pub(super) fn tuple(operation: &Operation, set: &TokenStream) -> Option<TokenStream> {
    Some(match operation {
        Operation::Field { row, column, .. } => quote!((#row).#column),
        Operation::Record { fields, .. } => {
            let fields = fields.iter();
            quote!((#(#fields,)*))
        }
        Operation::Type { schema, .. } => quote!(#set<#schema>),
        _ => return None,
    })
}
