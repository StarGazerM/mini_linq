//! Exact key resources with independently selected conservative metadata.

use super::work_binding;
use crate::physical::options::AccessOptions;
use cq_ir::{physical_provider::Operation, rust_access_plan as access};
use proc_macro2::TokenStream;
use quote::quote;

pub fn compile_exact_keys(
    operation: &Operation,
    options: &AccessOptions,
) -> syn::Result<TokenStream> {
    compile(operation, options, false)
}

pub fn compile_zone_keys(
    operation: &Operation,
    options: &AccessOptions,
) -> syn::Result<TokenStream> {
    compile(operation, options, true)
}

fn compile(operation: &Operation, options: &AccessOptions, zone: bool) -> syn::Result<TokenStream> {
    let storage = &options.storage;
    let work = work_binding(options.work.as_ref());
    match operation {
        Operation::Build { plan, .. } => {
            access::contract::check(plan)?;
            let iterator = &options.iterator;
            let keys = options.keys.as_ref().ok_or_else(|| {
                syn::Error::new_spanned(
                    operation,
                    "key construction requires a collection implementation",
                )
            })?;
            let index_rows = options.work.as_ref().map(|_| {
                quote! {
                    __cq_work.add(|w| w.index_rows += 1);
                }
            });
            if zone {
                Ok(quote!({
                    #work
                    let mut __cq_summary = <#storage::ZoneSummary<_> as ::core::default::Default>::default();
                    let mut __cq_rows = (#iterator! { #plan }).map(|__cq_key| {
                        #index_rows
                        #storage::RangeSummary::observe(&mut __cq_summary, &__cq_key);
                        __cq_key
                    });
                    let __cq_keys: #keys<_> = __cq_rows.by_ref().collect();
                    // A custom collector may stop early; metadata must cover
                    // every key denoted by the construction plan.
                    __cq_rows.for_each(|_| {});
                    (__cq_keys, __cq_summary)
                }))
            } else {
                let observation = options.work.as_ref().map(|_| {
                    quote! {
                        .inspect(|_| { #index_rows })
                    }
                });
                Ok(quote!({
                    #work
                    let __cq_keys: #keys<_> = (#iterator! { #plan }) #observation .collect();
                    __cq_keys
                }))
            }
        }
        Operation::Contains { resource, key, .. } => {
            let keys = if zone {
                quote!(&__cq_resource.0)
            } else {
                quote!(__cq_resource)
            };
            let probe = options.work.as_ref().map(|_| {
                quote! {
                    __cq_work.add(|w| w.exact_probes += 1);
                }
            });
            Ok(quote!({
                let __cq_resource = (#resource);
                let __cq_key = (#key);
                #work
                #probe
                #storage::ExactMembership::contains(#keys, &__cq_key)
            }))
        }
        Operation::MayMatch {
            resource,
            bounds,
            columns,
            ..
        } if zone => Ok(quote!({
            let __cq_resource = (#resource);
            let __cq_bounds = (#bounds);
            let __cq_columns = (#columns);
            #storage::RangeSummary::may_match(&__cq_resource.1, __cq_bounds, __cq_columns)
        })),
        _ => Err(syn::Error::new_spanned(
            operation,
            if zone {
                "zone keys support build, contains and may_match"
            } else {
                "exact keys support build and contains; metadata rejection requires a summary implementation"
            },
        )),
    }
}
