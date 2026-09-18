//! Indexed set computations and their standalone or callable Rust hosts.
//! The query implementation is shared; host construction adds no query logic.
mod coverage;
mod query;
mod semijoin;

mod check;
pub use check::{callable as check_callable, check, query as check_query};

use crate::rust_bindings::local;
use cq_ir::{index_requirements as ir, relational_plan as ra, symbol_name};
use proc_macro2::Span;
use quote::{format_ident, quote};
use std::collections::BTreeMap;

fn emit_host(source: &ir::Module, access: &syn::Path) -> syn::Result<syn::File> {
    let ra = &source.relational_plan;
    let output_types = output_types(source)?;
    let inputs: BTreeMap<_, _> = ra
        .program
        .inputs()
        .enumerate()
        .map(|(i, d)| (symbol_name(&d.name), local("input", i)))
        .collect();
    let mut body = query::compile(source, access, query::Observation::Count)?;
    let Some(syn::Stmt::Expr(answer, None)) = body.stmts.pop() else {
        unreachable!("query block has an answer")
    };
    let statements = body.stmts;
    let parameters: Vec<_> = ra
        .program
        .inputs()
        .map(|d| {
            let name = &inputs[&symbol_name(&d.name)];
            let types = d.columns.iter().map(|c| &c.ty);
            quote!(#name: impl ::core::iter::IntoIterator<Item = (#(#types,)*)>)
        })
        .collect();
    let arguments: Vec<_> = ra
        .program
        .inputs()
        .map(|d| &inputs[&symbol_name(&d.name)])
        .collect();
    let visibility = &ra.program.visibility;
    let program = &ra.program.name;
    let storage = format_ident!("{}Storage", program, span = Span::mixed_site());
    let stats = format_ident!("{}ExecutionStats", program, span = Span::mixed_site());
    let row = quote!((#(#output_types,)*));
    syn::parse2(quote! {
        #visibility struct #program;
        #visibility struct #storage { answer: ::std::collections::HashSet<#row>, stats: #stats }
        #[derive(Debug, Default)]
        #visibility struct #stats {
            pub results: ::std::vec::Vec<(&'static str, usize, usize)>,
            pub index_rows: usize,
            pub join_pairs: usize,
            pub semijoin_probes: usize,
        }
        impl #program {
            pub fn build(#(#parameters),*) -> #storage {
                let mut __stats = #stats::default();
                let __pairs = ::core::cell::Cell::new(0usize);
                let __probes = ::core::cell::Cell::new(0usize);
                #(#statements)*
                __stats.join_pairs = __pairs.get();
                __stats.semijoin_probes = __probes.get();
                let answer = #answer;
                #storage { answer, stats: __stats }
            }
            pub fn run(#(#parameters),*) -> ::std::vec::Vec<#row> {
                Self::build(#(#arguments),*).materialize()
            }
            pub fn run_with_stats(#(#parameters),*) -> (::std::vec::Vec<#row>, #stats) {
                let storage = Self::build(#(#arguments),*);
                (storage.materialize(), storage.stats)
            }
        }
        impl #storage {
            pub fn query(&self) -> impl ::core::iter::Iterator<Item = #row> + '_ { self.answer.iter().cloned() }
            pub fn materialize(&self) -> ::std::vec::Vec<#row> {
                let mut rows: ::std::vec::Vec<_> = self.query().collect(); rows.sort_unstable(); rows
            }
            pub fn stats(&self) -> &#stats { &self.stats }
        }
    })
}

/// The same checked hash realization with a caller-selected Access continuation.
/// The continuation must return the exact `HashSet` denoted by each Access plan;
/// it owns distinctness and collection. Its path changes neither concrete
/// storage checks nor local relational access checks.
pub fn compile(source: &ir::Module, access: &syn::Path) -> syn::Result<syn::File> {
    ir::contract::check(source)?;
    coverage::check_coverage(source)?;
    let target = emit_host(source, access)?;
    check(source, &target, access)?;
    Ok(target)
}

/// Compile an indexed set computation into an ordinary Rust block expression.
/// Inputs are evaluated exactly once, in declaration order, and must provide
/// owned rows through `IntoIterator`. Borrowed hosts can pass `.iter().cloned()`.
/// The result is an owned `HashSet` of output tuples; no counters or host types
/// are generated. The Access continuation must return each region's exact set.
pub fn compile_query(
    source: &ir::Module,
    access: &syn::Path,
    inputs: &[syn::Expr],
) -> syn::Result<syn::Expr> {
    ir::contract::check(source)?;
    coverage::check_coverage(source)?;
    let count = source.relational_plan.program.inputs().count();
    if inputs.len() != count {
        return Err(syn::Error::new_spanned(
            source,
            format!("expected {count} input expressions, found {}", inputs.len()),
        ));
    }
    let names: Vec<_> = (0..count).map(|i| local("input", i)).collect();
    let mut body = query::compile(source, access, query::Observation::Silent)?;
    let Some(syn::Stmt::Expr(answer, None)) = body.stmts.pop() else {
        unreachable!("query block has an answer")
    };
    let statements = body.stmts;
    let types = output_types(source)?;
    let target = syn::parse2(quote!({
        let (#(#names,)*) = (#((#inputs),)*);
        #(#statements)*
        let answer: ::std::collections::HashSet<(#(#types,)*)> = #answer;
        answer
    }))?;
    check_query(source, &target, access, inputs)?;
    Ok(target)
}

/// Adapt the checked query expression to an anonymous Rust closure.
/// Inputs implement `IntoIterator` with the declared row types; each closure
/// value has the input types inferred by Rust. No generated item can shadow
/// names referenced by query expressions or their opaque macro expansions.
pub fn compile_callable(source: &ir::Module, access: &syn::Path) -> syn::Result<syn::Expr> {
    let names: Vec<_> = source
        .relational_plan
        .program
        .inputs()
        .enumerate()
        .map(|(i, _)| local("input", i))
        .collect();
    let inputs: Vec<_> = names.iter().map(|id| syn::parse_quote!(#id)).collect();
    let body = compile_query(source, access, &inputs)?;
    let types = output_types(source)?;
    let target = syn::parse2(quote!(
        |#(#names),*| -> ::std::collections::HashSet<(#(#types,)*)> #body
    ))?;
    check_callable(source, &target, access)?;
    Ok(target)
}

fn output_types(source: &ir::Module) -> syn::Result<Vec<syn::Type>> {
    let ra = &source.relational_plan;
    let info = ra::check_in(ra.program.inputs(), &ra.plan)?;
    ra::output_types(&ra.program, &info[&symbol_name(&ra.plan.output.input)])
}

#[cfg(test)]
mod tests;
