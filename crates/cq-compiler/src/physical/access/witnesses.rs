//! Witness access selects indexing or repeated traversal without changing RA.

use super::work_binding;
use crate::physical::options::AccessOptions;
use cq_ir::{physical_provider::Operation, rust_access_plan as access};
use proc_macro2::TokenStream;
use quote::quote;

pub fn compile_hash_witnesses(
    operation: &Operation,
    options: &AccessOptions,
) -> syn::Result<TokenStream> {
    compile(operation, options, true)
}

pub fn compile_nested_witnesses(
    operation: &Operation,
    options: &AccessOptions,
) -> syn::Result<TokenStream> {
    compile(operation, options, false)
}

fn compile(operation: &Operation, options: &AccessOptions, hash: bool) -> syn::Result<TokenStream> {
    let work = work_binding(options.work.as_ref());
    match operation {
        Operation::Build { plan, .. } => {
            access::contract::check(plan)?;
            let iterator = &options.iterator;
            if hash {
                let index_rows = options.work.as_ref().map(|_| {
                    quote! {
                        __cq_work.add(|w| w.index_rows += 1);
                    }
                });
                Ok(quote!({
                    #work
                    let mut __cq_index = ::std::collections::HashMap::<_, ::std::vec::Vec<_>>::new();
                    for (__cq_key, __cq_row) in #iterator! { #plan } {
                        #index_rows
                        __cq_index.entry(__cq_key).or_default().push(__cq_row);
                    }
                    __cq_index
                }))
            } else {
                // The plan and its native source leaves execute at each probe,
                // never during resource construction.
                Ok(quote!(move || #iterator! { #plan }))
            }
        }
        Operation::Probe { resource, key, .. } => {
            let probe = options.work.as_ref().map(|_| {
                quote! {
                    __cq_work.add(|w| w.exact_probes += 1);
                }
            });
            if hash {
                Ok(quote!({
                    let __cq_resource = (#resource);
                    let __cq_key = (#key);
                    #work
                    #probe
                    __cq_resource.get(&__cq_key).into_iter().flatten().copied()
                }))
            } else {
                Ok(quote!({
                    let __cq_resource = (#resource);
                    let __cq_key = (#key);
                    #work
                    (__cq_resource)().filter_map(move |(__cq_candidate, __cq_row)| {
                        #probe
                        (__cq_candidate == __cq_key).then_some(__cq_row)
                    })
                }))
            }
        }
        _ => Err(syn::Error::new_spanned(
            operation,
            "witness access supports build and probe",
        )),
    }
}
