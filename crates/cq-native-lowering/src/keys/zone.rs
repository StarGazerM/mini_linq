//! Exact keys accompanied by a conservative coordinate summary.
use crate::{required, work_binding};
use cq_ir::{native::Operation, rust_access_plan as access};
use proc_macro2::TokenStream;
use quote::quote;

#[derive(Clone, Debug)]
pub struct Options {
    pub storage: syn::Path,
    pub access_iter: Option<syn::Path>,
    pub keys: Option<syn::Path>,
    pub work: Option<syn::Expr>,
}

pub fn compile(operation: &Operation, options: &Options) -> syn::Result<TokenStream> {
    let storage = &options.storage;
    let work = work_binding(options.work.as_ref());
    match operation {
        Operation::Key { fields, .. } => {
            let coordinates = fields.iter().map(|field| quote!(*(#field)));
            Ok(quote!([#(#coordinates,)*]))
        }
        Operation::Build { plan, .. } => {
            access::contract::check(plan)?;
            let access_iter = required(options.access_iter.as_ref(), operation, "access_iter")?;
            let keys = required(options.keys.as_ref(), operation, "keys")?;
            let index_rows = options.work.as_ref().map(|_| {
                quote! {
                    __cq_work.add(|w| w.index_rows += 1);
                }
            });
            Ok(quote!({
                #work
                let mut __cq_summary = <#storage::ZoneSummary<_> as ::core::default::Default>::default();
                let mut __cq_rows = (#access_iter! { #plan }).map(|__cq_key| {
                    #index_rows
                    #storage::RangeSummary::observe(&mut __cq_summary, &__cq_key);
                    __cq_key
                });
                let __cq_keys: #keys<_> = __cq_rows.by_ref().collect();
                // A custom collector may stop early; the summary must still
                // cover every key denoted by the construction plan.
                __cq_rows.for_each(|_| {});
                (__cq_keys, __cq_summary)
            }))
        }
        Operation::Contains { resource, key, .. } => {
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
                #storage::ExactMembership::contains(&__cq_resource.0, &__cq_key)
            }))
        }
        Operation::MayMatch {
            resource,
            bounds,
            columns,
            ..
        } => Ok(quote!({
            let __cq_resource = (#resource);
            let __cq_bounds = (#bounds);
            let __cq_columns = (#columns);
            #storage::RangeSummary::may_match(&__cq_resource.1, __cq_bounds, __cq_columns)
        })),
        _ => Err(syn::Error::new_spanned(
            operation,
            "zone keys support build, contains and may_match",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn options() -> Options {
        Options {
            storage: syn::parse_quote!(crate::data),
            access_iter: None,
            keys: None,
            work: None,
        }
    }
    #[test]
    fn tests_need_no_construction_configuration_and_evaluate_arguments_once() {
        let options = options();
        let operation = syn::parse_quote!(may_match(unique_resource())(unique_bounds())(
            unique_columns()
        ));
        let output = compile(&operation, &options).unwrap().to_string();
        for expression in ["unique_resource", "unique_bounds", "unique_columns"] {
            assert_eq!(output.matches(expression).count(), 1);
        }
        let operation = syn::parse_quote!(contains(unique_resource())(unique_key()));
        let mut options = options;
        options.work = Some(syn::parse_quote!(unique_work()));
        let output = compile(&operation, &options).unwrap().to_string();
        for expression in ["unique_resource", "unique_key", "unique_work"] {
            assert_eq!(output.matches(expression).count(), 1);
        }
    }
    #[test]
    fn builder_checks_access_and_requires_construction_dependencies() {
        let mut options = options();
        let valid = syn::parse_quote!(build { out(x) => ([x]) :- Rows(x) => for (x,) in (rows); });
        assert!(compile(&valid, &options).is_err());
        options.access_iter = Some(syn::parse_quote!(crate::rows));
        options.keys = Some(syn::parse_quote!(::std::collections::HashSet));
        let output = compile(&valid, &options).unwrap();
        assert!(!output.to_string().contains("__cq_work"));
        syn::parse2::<syn::Expr>(output).unwrap();
        let invalid = syn::parse_quote!(build { out(missing) => (missing) :- Rows(x) => for (x,) in (rows); });
        assert!(compile(&invalid, &options).is_err());
    }
}
