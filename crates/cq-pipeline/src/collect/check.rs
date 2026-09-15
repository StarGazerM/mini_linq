use crate::observations::expected_rust_local;
use cq_ir::iterator_pipeline;

/// Independently check the eager set endpoint. Only the returned terminal
/// distinct operator is implemented by collection; preceding operators retain
/// their exact adapter expressions and source order.
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
    let result = &source.return_stream.stream;
    let expected: syn::Expr = syn::parse_quote! {{
        #(#locals)*
        let #result: ::std::collections::HashSet<_> = ::core::iter::Iterator::collect(#input);
        #result
    }};
    target == &expected
}
