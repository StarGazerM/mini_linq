//! Rust syntax helpers shared by physical realizations.

use cq_ir::{cq, relational_plan as ra, symbol_name};
use proc_macro2::{Span, TokenStream};
use quote::{format_ident, quote};
use std::collections::BTreeSet;
use syn::spanned::Spanned;

pub fn local(prefix: &str, i: usize) -> syn::Ident {
    format_ident!("{prefix}{i}", span = Span::mixed_site())
}

/// Bind the logical environment at the original Rust syntax contexts instead
/// of rewriting paths or macro tokens. This also supports implicit format!
/// captures and callers that wrap the query in macro_rules!.
pub fn expression_aliases(
    expr: &syn::Expr,
    input: &ra::RelationInfo,
    value: impl Fn(&str) -> TokenStream,
) -> TokenStream {
    struct References<'a> {
        names: &'a BTreeSet<String>,
        ids: Vec<syn::Ident>,
    }
    impl<'ast> syn::visit::Visit<'ast> for References<'_> {
        fn visit_expr_path(&mut self, e: &'ast syn::ExprPath) {
            if e.qself.is_none() {
                if let Some(id) = e.path.get_ident() {
                    if self.names.contains(&symbol_name(id)) {
                        self.ids.push(id.clone());
                    }
                }
            }
        }
        fn visit_macro(&mut self, m: &'ast syn::Macro) {
            self.ids.extend(
                self.names
                    .iter()
                    .map(|name| format_ident!("r#{name}", span = m.path.span())),
            );
        }
    }
    let used = cq::rust::free_variables(expr).dependencies(&input.heading);
    let mut refs = References {
        names: &used,
        ids: Vec::new(),
    };
    syn::visit::Visit::visit_expr(&mut refs, expr);
    let bindings = refs.ids.iter().map(|id| {
        let name = symbol_name(id);
        let value = value(&name);
        if input.computed.contains(&name) {
            quote!(#[allow(unused_variables)] let #id = ::core::clone::Clone::clone(#value);)
        } else {
            quote!(#[allow(unused_variables)] let #id = #value;)
        }
    });
    quote!(#(#bindings)*)
}
