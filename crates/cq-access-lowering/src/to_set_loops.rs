//! Eager Access execution with a set sink and lexical Rust control flow.
//!
//! This is an alternative to pull lowering at the Access boundary, not a
//! Pipeline endpoint: no iterator operators or intermediate collection are
//! constructed. The returned expression evaluates the entire region. It is
//! suitable for a consumer requiring an exact completed set; it cannot replace
//! an iterator-demand continuation without changing that consumer's contract.
//!
//! Clauses execute in their written order. As in pull's dependent join, each
//! inner row receives clones of its live enclosing bindings so owned values can be
//! moved into the output. Rust checks ownership and leaf-expression types.
//! Full-consumption equality requires the usual value semantics for these
//! clones and the Rust leaves; arbitrary destructor/capture observations are
//! not claimed equivalent across execution strategies.

use cq_ir::{
    cq::rust,
    rust_access_plan::{self, RustAccess},
    symbol_name,
};
use proc_macro2::Span;
use quote::quote;
use std::collections::BTreeSet;

/// Compile a checked Access region into nested loops and one `HashSet` sink.
pub fn compile(source: &rust_access_plan::Plan) -> syn::Result<syn::Expr> {
    rust_access_plan::contract::check(source)?;
    let result = syn::Ident::new("__cq_output", Span::mixed_site());
    let output = &source.head.output;
    let mut statement = quote! { #result.insert(#output); };

    // Record each clause's incoming binding before nesting scopes backwards.
    // Clone only bindings used after the inner source has been evaluated.
    // Opaque Rust expressions conservatively retain the entire environment.
    // This is scheduling metadata for this lowering, not another plan language.
    let mut binding = Vec::new();
    let clauses = source
        .body
        .iter()
        .map(|clause| {
            let incoming = binding.clone();
            if let RustAccess::Bind { pattern, .. } = &clause.access {
                binding.extend(cq_ir::cq::rust::bindings(pattern));
            }
            (&clause.access, incoming)
        })
        .collect::<Vec<_>>();
    let environment: BTreeSet<_> = binding.iter().map(symbol_name).collect();
    let mut live = rust::free_variables(output).dependencies(&environment);
    for (access, incoming) in clauses.into_iter().rev() {
        statement = match access {
            RustAccess::Bind {
                pattern, source, ..
            } => {
                let copies = incoming
                    .iter()
                    .filter(|variable| live.contains(&symbol_name(variable)))
                    .map(|variable| {
                        quote! { let #variable = ::core::clone::Clone::clone(&#variable); }
                    });
                let emitted = quote! {
                    for #pattern in (#source) {
                        #(#copies)*
                        #statement
                    }
                };
                for variable in rust::bindings(pattern) {
                    live.remove(&symbol_name(&variable));
                }
                live.extend(rust::free_variables(source).dependencies(&environment));
                emitted
            }
            RustAccess::If { condition, .. } => {
                live.extend(rust::free_variables(condition).dependencies(&environment));
                quote! { if #condition { #statement } }
            }
        };
    }
    syn::parse2(quote! {{
        let mut #result = ::std::collections::HashSet::new();
        #statement
        #result
    }})
}
