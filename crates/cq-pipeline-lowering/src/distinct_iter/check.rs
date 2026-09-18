use crate::observations::expected_rust_local;
use cq_ir::iterator_pipeline;
use quote::quote;

/// Whether `target` is the exact ordinary-Rust implementation denoted by the
/// named operator plan.
///
/// This relation independently constructs the required target shape. It does
/// not call the lowering under contract: it checks the fixed deferred wrapper,
/// one local per definition in source order, every exact adapter initializer,
/// and the recorded final stream.
pub fn check(source: &iterator_pipeline::Pipeline, target: &syn::Expr) -> bool {
    if !iterator_pipeline::contract::well_formed(source) {
        return false;
    }
    let locals = source
        .definitions
        .iter()
        .map(expected_rust_local)
        .collect::<Vec<_>>();
    let return_stream = &source.return_stream.stream;
    let Ok(expected) = syn::parse2::<syn::Expr>(quote! {
        ::core::iter::Iterator::flatten(
            ::core::iter::once_with(move || {
                #(#locals)*
                #return_stream
            })
        )
    }) else {
        return false;
    };
    target == &expected
}
