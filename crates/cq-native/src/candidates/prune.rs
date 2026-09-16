//! Conservative block rejection before candidate payload traversal.
use crate::work_binding;
use cq_ir::{native::Operation, rust_access_plan as access};
use proc_macro2::TokenStream;
use quote::quote;

#[derive(Clone, Debug)]
pub struct Options {
    pub iterator: syn::Path,
    pub work: Option<syn::Expr>,
}

/// Refine candidate traversal into metadata enumeration, rejection and payload.
///
/// The enclosing exact guard must reject every row in each rejected block.
/// Bounds enclose the immutable payload, so the law also covers pruned views;
/// the provider enumerates only handles selected by the current view.
pub fn compile(operation: &Operation, options: &Options) -> syn::Result<TokenStream> {
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
        let __cq_predicate = ({
            fn __cq_bind_predicate<B: ?::core::marker::Sized, F: ::core::ops::Fn(&B) -> ::core::primitive::bool>(predicate: F) -> F {
                predicate
            }
            __cq_bind_predicate
        })(#predicate);
        #work
        #iterator! { #plan }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn traversal_evaluates_operands_once_without_storage_configuration() {
        let operation =
            syn::parse_quote!(candidates crate::rows { unique_view() } (unique_predicate()));
        let options = Options {
            iterator: syn::parse_quote!(crate::iterate),
            work: Some(syn::parse_quote!(unique_work())),
        };
        let output = compile(&operation, &options).unwrap().to_string();
        for expression in ["unique_view", "unique_predicate", "unique_work"] {
            assert_eq!(output.matches(expression).count(), 1);
        }
    }
    #[test]
    fn plain_traversal_contains_no_observer() {
        let operation =
            syn::parse_quote!(candidates crate::rows { &view } (|bounds| accepts(bounds)));
        let options = Options {
            iterator: syn::parse_quote!(crate::iterate),
            work: None,
        };
        let output = compile(&operation, &options).unwrap();
        assert!(!output.to_string().contains("__cq_work"));
        syn::parse2::<syn::Expr>(output).unwrap();
    }
}
