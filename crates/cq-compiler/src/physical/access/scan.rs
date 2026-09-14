//! Candidate traversal is refined independently of the enclosing RA operator.

use super::work_binding;
use crate::physical::options::AccessOptions;
use cq_ir::{physical_provider::Operation, rust_access_plan as access};
use proc_macro2::TokenStream;
use quote::quote;

/// Enumerate every current handle; the unused metadata predicate is not emitted.
pub fn compile_scan_rows(
    operation: &Operation,
    _options: &AccessOptions,
) -> syn::Result<TokenStream> {
    let Operation::Candidates {
        provider, value, ..
    } = operation
    else {
        return Err(syn::Error::new_spanned(
            operation,
            "row scan requires candidates",
        ));
    };
    Ok(quote!(#provider! { rows (#value) }))
}

/// Refine candidate traversal into metadata enumeration, rejection and payload.
///
/// The enclosing exact guard must reject every row in each rejected block.
/// Bounds enclose the immutable payload, so the law also covers pruned views;
/// the provider enumerates only handles selected by the current view.
pub fn compile_prune_rows(
    operation: &Operation,
    options: &AccessOptions,
) -> syn::Result<TokenStream> {
    let Operation::Candidates {
        provider,
        value,
        predicate,
        ..
    } = operation
    else {
        return Err(syn::Error::new_spanned(
            operation,
            "block pruning requires candidates",
        ));
    };
    let iterator = &options.iterator;
    let work = work_binding(options.work.as_ref());
    let skipped = options.work.as_ref().map(|_| {
        quote! {
            if !__cq_pass { __cq_work.add(|w| w.blocks_skipped += 1); }
        }
    });
    let plan: access::Plan = syn::parse_quote! {
        out(__cq_handle) => (__cq_handle) :-
            Blocks(__cq_block) => for (__cq_block,) in (
                (#provider! { blocks (__cq_view) }).map(|block| (block,))
            ),
            if __cq_predicate(#provider! { bounds (__cq_block) }) => if ({
                let __cq_pass = __cq_predicate(#provider! { bounds (__cq_block) });
                #skipped
                __cq_pass
            }),
            Rows(__cq_handle) => for (__cq_handle,) in (
                (#provider! { block_rows (__cq_block) (__cq_view) }).map(|row| (row,))
            );
    };
    access::contract::check(&plan)?;
    Ok(quote!({
        let __cq_view = (#value);
        // Give the closure a universally borrowed argument without choosing
        // the provider's metadata type in the enclosing RA lowering.
        fn __cq_bind_predicate<B: ?::core::marker::Sized, F: ::core::ops::Fn(&B) -> ::core::primitive::bool>(predicate: F) -> F {
            predicate
        }
        let __cq_predicate = __cq_bind_predicate(#predicate);
        #work
        #iterator! { #plan }
    }))
}
