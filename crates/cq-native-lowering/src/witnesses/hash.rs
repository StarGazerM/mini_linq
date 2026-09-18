//! Hash buckets retain all exact witnesses for each key.
use crate::{required, work_binding};
use cq_ir::{native::Operation, rust_access_plan as access};
use proc_macro2::TokenStream;
use quote::quote;

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub access_iter: Option<syn::Path>,
    pub work: Option<syn::Expr>,
}

pub fn compile(operation: &Operation, options: &Options) -> syn::Result<TokenStream> {
    let work = work_binding(options.work.as_ref());
    match operation {
        Operation::Build { plan, .. } => {
            access::contract::check(plan)?;
            let access_iter = required(options.access_iter.as_ref(), operation, "access_iter")?;
            let index_rows = options.work.as_ref().map(|_| {
                quote! {
                    __cq_work.add(|w| w.index_rows += 1);
                }
            });
            Ok(quote!({
                #work
                let mut __cq_index = ::std::collections::HashMap::<_, ::std::vec::Vec<_>>::new();
                for (__cq_key, __cq_row) in #access_iter! { #plan } {
                    #index_rows
                    __cq_index.entry(__cq_key).or_default().push(__cq_row);
                }
                __cq_index
            }))
        }
        Operation::Probe { resource, key, .. } => {
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
                __cq_resource.get(&__cq_key).into_iter().flatten().copied()
            }))
        }
        _ => Err(syn::Error::new_spanned(
            operation,
            "hash witnesses support build and probe",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_needs_no_build_iterator_and_evaluates_once() {
        let operation = syn::parse_quote!(probe(unique_resource())(unique_key()));
        let options = Options {
            access_iter: None,
            work: Some(syn::parse_quote!(unique_work())),
        };
        let output = compile(&operation, &options).unwrap().to_string();
        for expression in ["unique_resource", "unique_key", "unique_work"] {
            assert_eq!(output.matches(expression).count(), 1);
        }
    }
    #[test]
    fn builder_validates_access_without_mandatory_observer() {
        let options = Options {
            access_iter: Some(syn::parse_quote!(crate::rows)),
            work: None,
        };
        let valid =
            syn::parse_quote!(build { out(x) => (([x], row)) :- Rows(x) => for (x,) in (rows); });
        let output = compile(&valid, &options).unwrap();
        assert!(!output.to_string().contains("__cq_work"));
        syn::parse2::<syn::Expr>(output).unwrap();
        let invalid = syn::parse_quote!(build { out(missing) => (missing) :- Rows(x) => for (x,) in (rows); });
        assert!(compile(&invalid, &options).is_err());
        assert!(compile(&valid, &Options::default()).is_err());
    }
}
