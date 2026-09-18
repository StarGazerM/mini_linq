use crate::observations::expected_rust_local;
use cq_ir::iterator_pipeline;

/// Independently check the support-stream endpoint. The local Pipeline contract
/// establishes a sole returned terminal `Distinct`; only its implementation is
/// omitted. The result can repeat values but has exactly the source set support.
pub fn check(source: &iterator_pipeline::Pipeline, target: &syn::Expr) -> bool {
    if !iterator_pipeline::contract::well_formed(source) {
        return false;
    }
    let Some((last, prefix)) = source.definitions.split_last() else {
        return false;
    };
    let iterator_pipeline::Operator::Distinct(distinct) = &last.operator else {
        return false;
    };
    let locals = prefix.iter().map(expected_rust_local);
    let input = &distinct.input_stream;
    let expected: syn::Expr = syn::parse_quote! {
        ::core::iter::Iterator::flatten(
            ::core::iter::once_with(move || {
                #(#locals)*
                #input
            })
        )
    };
    target == &expected
}
