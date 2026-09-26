//! Eager Access execution with a set sink and lexical Rust control flow.
//!
//! This is an alternative to pull lowering at the Access boundary, not a
//! Pipeline endpoint: no iterator operators or intermediate collection are
//! constructed. The returned expression evaluates the entire region. It is
//! suitable for a consumer requiring an exact completed set; it cannot replace
//! an iterator-demand continuation without changing that consumer's contract.
//!
//! Clauses execute in their written order. Nothing is copied: a binding is
//! used inside its inner loops as the value the source produced, so an owned
//! value bound by an outer source and used by more than one inner row is a
//! Rust move error, and the head expression alone decides what the result
//! owns. Rust checks ownership and leaf-expression types; arbitrary
//! destructor/capture observations are not claimed equivalent across
//! execution strategies.

use cq_ir::rust_access_plan::{self, RustAccess};
use proc_macro2::Span;
use quote::quote;

/// Compile a checked Access region into nested loops and one explicitly selected exact-set sink.
pub fn compile(source: &rust_access_plan::Plan) -> syn::Result<syn::Expr> {
    rust_access_plan::contract::check(source)?;
    let result = syn::Ident::new("__cq_output", Span::mixed_site());
    let output = &source.head.output;
    let operator = &source.head.atom.relation;
    let mut statement =
        quote! { mini_linq::set_sink! { operator #operator; insert(#result, #output) }; };

    for clause in source.body.iter().rev() {
        statement = match &clause.access {
            RustAccess::Bind {
                pattern, source, ..
            } => quote! { for #pattern in (#source) { #statement } },
            RustAccess::If { condition, .. } => quote! { if #condition { #statement } },
        };
    }
    syn::parse2(quote! {{
        let mut #result = mini_linq::set_sink! { operator #operator; new };
        #statement
        mini_linq::set_sink! { operator #operator; finish(#result) }
    }})
}
