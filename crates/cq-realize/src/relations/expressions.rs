//! Bind ordinary Rust expressions to fields without exposing row representation.
use super::{layout::View, wiring::Wiring};
use cq_ir::{cq, symbol_name};
use proc_macro2::TokenStream;
use quote::quote;

pub(super) fn field(view: &View, row: TokenStream, column: usize, wiring: &Wiring) -> TokenStream {
    let provider = wiring.provider(&view.provider);
    let column = syn::Index::from(column);
    quote!(#provider! { field (#row) (#column) })
}

pub(super) fn expression(
    expression: &syn::Expr,
    view: &View,
    row: TokenStream,
    wiring: &Wiring,
) -> TokenStream {
    let aliases = crate::rust_bindings::read_aliases(expression, &view.info.heading, |name| {
        let column = view.layout.iter().position(|v| v == name).unwrap();
        let field = field(view, row.clone(), column, wiring);
        quote!(&(#field))
    });
    quote!({ #aliases #expression })
}

pub(super) fn condition(
    expression: &syn::Expr,
    view: &View,
    row: TokenStream,
    wiring: &Wiring,
) -> TokenStream {
    if let Some(predicate) = cq::Predicate::from_expr(expression).filter(|p| {
        p.variables()
            .iter()
            .all(|name| view.info.heading.contains(name))
    }) {
        let term = |term: &cq::Term| match term {
            cq::Term::Integer(value) => quote!(&#value),
            cq::Term::Variable(name) => {
                let index = view
                    .layout
                    .iter()
                    .position(|v| *v == symbol_name(name))
                    .unwrap();
                let field = field(view, row.clone(), index, wiring);
                quote!(&(#field))
            }
        };
        let left = term(&predicate.left);
        let right = term(&predicate.right);
        let comparison = &predicate.comparison;
        quote!(#left #comparison #right)
    } else {
        self::expression(expression, view, row, wiring)
    }
}

pub(super) fn output(
    expression: &syn::Expr,
    view: &View,
    row: TokenStream,
    wiring: &Wiring,
) -> TokenStream {
    if let Some(variable) =
        cq::rust::variable(expression).filter(|v| view.info.heading.contains(&symbol_name(v)))
    {
        let index = view
            .layout
            .iter()
            .position(|v| *v == symbol_name(variable))
            .unwrap();
        let field = field(view, row, index, wiring);
        quote!(::core::clone::Clone::clone(&(#field)))
    } else {
        self::expression(expression, view, row, wiring)
    }
}
