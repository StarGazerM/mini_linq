//! Exact key collection construction and membership.
use crate::{required, work_binding};
use cq_ir::{native::Operation, rust_access_plan as access};
use proc_macro2::TokenStream;
use quote::quote;

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub storage: Option<syn::Path>,
    pub iterator: Option<syn::Path>,
    pub keys: Option<syn::Path>,
    pub work: Option<syn::Expr>,
}

pub fn compile(operation: &Operation, options: &Options) -> syn::Result<TokenStream> {
    let work = work_binding(options.work.as_ref());
    match operation {
        Operation::Key { fields, .. } => Ok(quote!((#fields))),
        Operation::Build { plan, .. } => {
            access::contract::check(plan)?;
            let iterator = required(options.iterator.as_ref(), operation, "iterator")?;
            let keys = required(options.keys.as_ref(), operation, "keys")?;
            let observation = options.work.as_ref().map(|_| {
                quote! {
                    .inspect(|_| { __cq_work.add(|w| w.index_rows += 1); })
                }
            });
            Ok(quote!({
                #work
                let __cq_keys: #keys<_> = (#iterator! { #plan }) #observation .collect();
                __cq_keys
            }))
        }
        Operation::Contains { resource, key, .. } => {
            let storage = required(options.storage.as_ref(), operation, "storage")?;
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
                #storage::ExactMembership::contains(__cq_resource, &__cq_key)
            }))
        }
        _ => Err(syn::Error::new_spanned(
            operation,
            "exact keys support build and contains; metadata rejection requires a summary implementation",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn membership_requires_no_construction_configuration_and_evaluates_once() {
        let operation = syn::parse_quote!(contains(unique_resource())(unique_key()));
        let options = Options {
            storage: Some(syn::parse_quote!(crate::data)),
            work: Some(syn::parse_quote!(unique_work())),
            ..Options::default()
        };
        let output = compile(&operation, &options).unwrap().to_string();
        for expression in ["unique_resource", "unique_key", "unique_work"] {
            assert_eq!(output.matches(expression).count(), 1, "{output}");
        }
    }

    #[test]
    fn build_requires_only_its_dependencies_and_checks_access() {
        let options = Options {
            iterator: Some(syn::parse_quote!(crate::rows)),
            keys: Some(syn::parse_quote!(::std::collections::HashSet)),
            ..Options::default()
        };
        let valid = syn::parse_quote!(build { out(x) => ([x]) :- Rows(x) => for (x,) in (rows); });
        let output = compile(&valid, &options).unwrap();
        assert!(!output.to_string().contains("__cq_work"));
        syn::parse2::<syn::Expr>(output).unwrap();
        let invalid = syn::parse_quote!(build { out(missing) => (missing) :- Rows(x) => for (x,) in (rows); });
        assert!(compile(&invalid, &options).is_err());
        assert!(compile(&valid, &Options::default()).is_err());
    }
}
