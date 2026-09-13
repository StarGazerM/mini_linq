//! Exact-key projection and semijoin realization from checked RA definitions.
use crate::RecipeOptions;
use cq_ir::{physical_context as context, relational_plan as ra, rust_access_plan as access};
use proc_macro2::TokenStream;
use quote::quote;

/// CP-Build / CP-Exact: realize a key projection or reduce current left handles.
pub fn compile_exact_semijoin(
    source: &context::Context,
    options: &RecipeOptions,
) -> syn::Result<syn::Expr> {
    semijoin(source, options, false)
}

/// CP-Build / CP-Zone: realize a summarized projection or prune before reduction.
pub fn compile_zone_semijoin(
    source: &context::Context,
    options: &RecipeOptions,
) -> syn::Result<syn::Expr> {
    semijoin(source, options, true)
}

fn semijoin(
    source: &context::Context,
    options: &RecipeOptions,
    zone: bool,
) -> syn::Result<syn::Expr> {
    if matches!(source.source.operator, ra::Operator::Project(_)) {
        context::contract::check_project(source)?;
        let projection = context::analyze_project(source)?;
        let keys = options.keys.as_ref().ok_or_else(|| {
            syn::Error::new_spanned(
                &source.source,
                "key projection requires a keys implementation",
            )
        })?;
        return syn::parse2(build_keys(&projection, options, keys, zone));
    }
    context::contract::check_semijoin(source)?;
    let analysis = context::analyze(source)?;
    let (storage, next) = (&options.storage, &options.access);
    let (provider, left) = (
        &analysis.left.provider.as_ref().unwrap().path,
        &analysis.left.value,
    );
    let resource = &analysis.right.value;
    let columns = &analysis.left_columns;
    let skipped = options.work.as_ref().map(|_| {
        quote!(if !__cq_pass {
            __cq_work.add(|w| w.blocks_skipped += 1);
        })
    });
    let probes = options
        .work
        .as_ref()
        .map(|_| quote!(__cq_work.add(|w| w.exact_probes += 1);));
    let mut clauses: Vec<access::Clause> = if zone {
        vec![
            syn::parse_quote! {
                Blocks(__cq_block) => for (__cq_block,) in (
                    (#provider! { blocks (__cq_left) }).map(|block| (block,))
                )
            },
            syn::parse_quote! {
                if #storage::RangeSummary::may_match(__cq_summary,
                    #provider! { bounds (__cq_block) }, &[#(#columns),*]
                ) => if ({
                    let __cq_pass = #storage::RangeSummary::may_match(__cq_summary,
                        #provider! { bounds (__cq_block) }, &[#(#columns),*]
                    );
                    #skipped
                    __cq_pass
                })
            },
            syn::parse_quote! {
                Rows(__cq_handle) => for (__cq_handle,) in (
                    (#provider! { block_rows (__cq_block) (__cq_left) }).map(|row| (row,))
                )
            },
        ]
    } else {
        vec![syn::parse_quote! {
            Rows(__cq_handle) => for (__cq_handle,) in (
                (#provider! { rows (__cq_left) }).map(|row| (row,))
            )
        }]
    };
    clauses.push(syn::parse_quote! {
        if #storage::ExactMembership::contains(__cq_keys,&[
            #((#provider! { row (__cq_handle) })[#columns]),*
        ]) => if ({
            #probes
            #storage::ExactMembership::contains(__cq_keys,&[
                #((#provider! { row (__cq_handle) })[#columns]),*
            ])
        })
    });
    let plan: access::Plan = syn::parse_quote! {
        out(__cq_handle) => (__cq_handle) :- #(#clauses),*;
    };
    access::contract::check(&plan)?;
    let work = options
        .work
        .as_ref()
        .map(|work| quote!(let __cq_work = (#work);));
    let summary = zone.then(|| quote!(let __cq_summary = &__cq_resource.1;));
    syn::parse2(quote!({
        let __cq_left = &(#left);
        let __cq_resource = &(#resource);
        let __cq_keys = &__cq_resource.0;
        #work
        #summary
        #provider! { select (__cq_left) (#next! { #plan }) }
    }))
}

fn build_keys(
    projection: &context::Projection<'_>,
    options: &RecipeOptions,
    keys: &syn::Path,
    zone: bool,
) -> TokenStream {
    let storage = &options.storage;
    let (provider, source) = (
        &projection.input.provider.as_ref().unwrap().path,
        &projection.input.value,
    );
    let columns = &projection.columns;
    let arity = columns.len();
    let work = options
        .work
        .as_ref()
        .map(|work| quote!(let __cq_work = (#work);));
    let index_rows = options
        .work
        .as_ref()
        .map(|_| quote!(__cq_work.add(|w| w.index_rows += 1);));
    // The selected projection recipe owns source access. Another implementation
    // may borrow a matching existing index instead of enumerating payload rows.
    let scan = quote!(#provider! { scan (__cq_right) });
    if zone {
        quote!({
            let __cq_right = &(#source);
            #work
            let mut __cq_summary = <#storage::ZoneSummary<#arity> as ::core::default::Default>::default();
            let mut __cq_rows = (#scan).map(|__cq_row| {
                #index_rows
                let __cq_key = [#(__cq_row[#columns]),*];
                #storage::RangeSummary::observe(&mut __cq_summary, &__cq_key);
                __cq_key
            });
            let __cq_keys: #keys<[::core::primitive::i64; #arity]> =
                __cq_rows.by_ref().collect();
            // Finish summary construction even for a custom lazy collector.
            __cq_rows.for_each(|_| {});
            (__cq_keys, __cq_summary)
        })
    } else {
        quote!({
            let __cq_right = &(#source);
            #work
            let __cq_keys: #keys<[::core::primitive::i64; #arity]> = (#scan)
                .map(|__cq_row| {
                    #index_rows
                    [#(__cq_row[#columns]),*]
                }).collect();
            (__cq_keys,)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(work: Option<syn::Expr>) -> RecipeOptions {
        RecipeOptions {
            storage: syn::parse_quote!(crate::data),
            access: syn::parse_quote!(crate::collect),
            keys: Some(syn::parse_quote!(::std::collections::HashSet)),
            work,
        }
    }

    #[test]
    fn semijoin_recipes_produce_checked_access_composition() {
        let input = syn::parse_quote! {
            reduced = semijoin l with q; {
                l(a,b,c) => crate::rows { left };
                q(a,c) { resource };
            }
        };
        for compile in [compile_exact_semijoin, compile_zone_semijoin] {
            compile(&input, &options(Some(syn::parse_quote!(&work)))).unwrap();
        }
    }

    #[test]
    fn uninstrumented_recipes_emit_no_counter_bindings_or_operations() {
        for input in [
            syn::parse_quote! {
                q = project r keep {a,c}; { r(c,a,b) => crate::rows { right }; }
            },
            syn::parse_quote! {
                reduced = semijoin l with q; {
                    l(a,b,c) => crate::rows { left };
                    q(a,c) { resource };
                }
            },
        ] {
            for compile in [compile_exact_semijoin, compile_zone_semijoin] {
                let result = compile(&input, &options(None)).unwrap();
                let text = quote!(#result).to_string();
                assert!(!text.contains("__cq_work"), "{text}");
                assert!(!text.contains(". add"), "{text}");
            }
        }
    }
}
