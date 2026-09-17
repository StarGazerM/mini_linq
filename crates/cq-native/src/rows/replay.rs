//! Retain an Access region's demanded support through a replay data structure.
//!
//! The selected Access continuation owns execution lowering. This provider
//! only binds its resulting iterator to storage and requests owning cursors.

use cq_ir::{native::Operation, rust_access_plan as access};
use proc_macro2::TokenStream;
use quote::quote;

#[derive(Clone, Debug)]
pub struct Options {
    pub storage: syn::Path,
    pub iterator: syn::Path,
}

pub fn compile(operation: &Operation, options: &Options) -> syn::Result<TokenStream> {
    let storage = &options.storage;
    match operation {
        Operation::Build { plan, .. } => {
            access::contract::check(plan)?;
            let iterator = &options.iterator;
            Ok(quote! {
                <#storage::Replay<_> as #storage::BuildReplay<_>>::replay(
                    #iterator! { #plan }
                )
            })
        }
        Operation::Scan { value, .. } => Ok(quote!(#storage::ReplayRead::cursor(&(#value)))),
        _ => Err(syn::Error::new_spanned(
            operation,
            "replay rows support only build and scan",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Options {
        Options {
            storage: syn::parse_quote!(crate::storage),
            iterator: syn::parse_quote!(crate::access),
        }
    }

    #[test]
    fn build_preserves_an_independent_checked_access_continuation() {
        let operation = syn::parse_quote! {
            build { out(x) => ((x,)) :- Rows(x) => for (x,) in (source()); }
        };
        let expression: syn::Expr = syn::parse2(compile(&operation, &options()).unwrap()).unwrap();
        let syn::Expr::Call(call) = expression else {
            panic!("expected storage construction through its trait");
        };
        let syn::Expr::Macro(invocation) = &call.args[0] else {
            panic!("construction must retain the next-stage macro invocation");
        };
        assert_eq!(invocation.mac.path, options().iterator);
        let actual: access::Plan = syn::parse2(invocation.mac.tokens.clone()).unwrap();
        let Operation::Build { plan, .. } = operation else {
            unreachable!();
        };
        assert_eq!(actual, *plan);
        let syn::Expr::Path(function) = &*call.func else {
            panic!("expected a qualified trait call");
        };
        assert!(function.qself.is_some());
        assert_eq!(function.path.segments.last().unwrap().ident, "replay");
    }

    #[test]
    fn invalid_access_is_rejected_before_storage_emission() {
        let operation = syn::parse_quote! {
            build { out(missing) => (missing) :- Rows(x) => for (x,) in (source()); }
        };
        assert!(compile(&operation, &options()).is_err());
    }

    #[test]
    fn scan_uses_the_access_trait_and_evaluates_its_operand_once() {
        let operation = syn::parse_quote!(scan(unique_source()));
        let output = compile(&operation, &options()).unwrap();
        let expression: syn::Expr = syn::parse2(output.clone()).unwrap();
        assert!(matches!(expression, syn::Expr::Call(_)));
        let text = output.to_string();
        assert!(text.contains("ReplayRead :: cursor"));
        assert_eq!(text.matches("unique_source").count(), 1);
        assert!(!text.contains("crate :: access"));
    }

    #[test]
    fn an_unsupported_protocol_operation_is_rejected() {
        let operation = syn::parse_quote!(materialize owner value (source()));
        assert!(
            compile(&operation, &options())
                .unwrap_err()
                .to_string()
                .contains("only build and scan")
        );
    }
}
