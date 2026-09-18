//! Repeated exact witness traversal, deferred until each probe.
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
    match operation {
        Operation::Build { plan, .. } => {
            access::contract::check(plan)?;
            let access_iter = required(options.access_iter.as_ref(), operation, "access_iter")?;
            // Neither source traversal nor observer evaluation belongs to
            // construction of a deferred witness traversal.
            Ok(quote!(move || #access_iter! { #plan }))
        }
        Operation::Probe { resource, key, .. } => {
            let work = work_binding(options.work.as_ref());
            let probe = options.work.as_ref().map(|_| {
                quote! {
                    __cq_work.add(|w| w.exact_probes += 1);
                }
            });
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
        _ => Err(syn::Error::new_spanned(
            operation,
            "nested witnesses support build and probe",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn construction_defers_traversal_and_does_not_evaluate_observer() {
        let operation = syn::parse_quote!(build { out(x) => (([x], row)) :- Rows(x) => for (x,) in (unique_scan()); });
        let options = Options {
            access_iter: Some(syn::parse_quote!(crate::rows)),
            work: Some(syn::parse_quote!(compile_error!("unused work"))),
        };
        let output = compile(&operation, &options).unwrap();
        assert!(matches!(
            syn::parse2::<syn::Expr>(output.clone()).unwrap(),
            syn::Expr::Closure(_)
        ));
        let text = output.to_string();
        assert!(text.contains("unique_scan"));
        for absent in ["collect", "HashMap", "index_rows", "unused work"] {
            assert!(!text.contains(absent), "{text}");
        }
        let invalid = syn::parse_quote!(build { out(missing) => (missing) :- Rows(x) => for (x,) in (rows); });
        assert!(compile(&invalid, &options).is_err());
        assert!(compile(&operation, &Options::default()).is_err());
    }
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
}
