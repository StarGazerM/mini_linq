//! Rust syntax helpers shared by physical realizations.

#[cfg(any(feature = "hash-prefix", feature = "hash-materialized"))]
use cq_ir::relational_plan as ra;
use cq_ir::{cq, symbol_name};
#[cfg(any(feature = "hash-prefix", feature = "hash-materialized"))]
use proc_macro2::Span;
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::BTreeSet;
use syn::spanned::Spanned;

/// Preserve primitive type spellings when generated host names share their
/// identifiers. User aliases, raw identifiers, and already qualified paths keep
/// their ordinary Rust resolution. This changes type syntax, not row layout.
#[cfg(feature = "relations")]
pub fn qualify_type(source: &syn::Type) -> syn::Type {
    struct Qualify;
    impl syn::visit_mut::VisitMut for Qualify {
        fn visit_type_path_mut(&mut self, ty: &mut syn::TypePath) {
            syn::visit_mut::visit_type_path_mut(self, ty);
            if ty.qself.is_some() {
                return;
            }
            if let Some(name) = ty.path.get_ident() {
                if matches!(
                    name.to_string().as_str(),
                    "bool"
                        | "char"
                        | "str"
                        | "i8"
                        | "i16"
                        | "i32"
                        | "i64"
                        | "i128"
                        | "isize"
                        | "u8"
                        | "u16"
                        | "u32"
                        | "u64"
                        | "u128"
                        | "usize"
                        | "f32"
                        | "f64"
                ) {
                    ty.path = syn::parse_quote!(::core::primitive::#name);
                }
            }
        }
    }
    let mut target = source.clone();
    syn::visit_mut::VisitMut::visit_type_mut(&mut Qualify, &mut target);
    target
}

#[cfg(any(feature = "hash-prefix", feature = "hash-materialized"))]
pub fn local(prefix: &str, i: usize) -> syn::Ident {
    format_ident!("{prefix}{i}", span = Span::mixed_site())
}

/// The fixed hash realizations retain their owned computed-variable convention.
#[cfg(any(feature = "hash-prefix", feature = "hash-materialized"))]
pub fn expression_aliases(
    expr: &syn::Expr,
    input: &ra::RelationInfo,
    value: impl Fn(&str) -> TokenStream,
) -> TokenStream {
    read_aliases(expr, &input.heading, |name| {
        let value = value(name);
        if input.computed.contains(name) {
            quote!(::core::clone::Clone::clone(#value))
        } else {
            value
        }
    })
}

/// Bind reads at the original Rust syntax contexts without taking ownership.
/// The caller supplies each field borrow. Opaque macros conservatively bind
/// all possible dependencies, so unused aliases must not clone their payloads.
pub fn read_aliases(
    expr: &syn::Expr,
    heading: &BTreeSet<String>,
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
    let used = cq::rust::free_variables(expr).dependencies(heading);
    let mut refs = References {
        names: &used,
        ids: Vec::new(),
    };
    syn::visit::Visit::visit_expr(&mut refs, expr);
    let bindings = refs.ids.iter().map(|id| {
        let name = symbol_name(id);
        let value = value(&name);
        quote!(#[allow(unused_variables)] let #id = #value;)
    });
    quote!(#(#bindings)*)
}
