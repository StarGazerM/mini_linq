//! Raw-token if-let expansion. Its meaning is the checked generator translation.
use cq_datalog_support::ensure;
use cq_lang_datalog_source::{self as source, BodyItem, Query};
use cq_syntax::rust;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::Expr;
fn expand_query(query: &mut Query) -> syn::Result<()> {
    let mut changed = false;
    for item in &mut query.body {
        let BodyItem::Filter(filter) = item else {
            continue;
        };
        let Expr::Let(binding) = rust::unparen(&filter.condition) else {
            continue;
        };
        let pattern = &binding.pat;
        let expression = &binding.expr;
        let bindings = rust::bindings(pattern);
        *item = syn::parse2(quote! {
            for (#(#bindings,)*) in match #expression {
                #pattern => ::core::option::Option::Some((#(#bindings,)*)),
                _ => ::core::option::Option::None,
            }
        })?;
        changed = true;
    }
    if changed {
        // CQ's initial parse did not know the refutable pattern's bindings.
        // Reclassify guards with the complete logical-variable environment.
        *query = syn::parse2(query.to_token_stream())?;
    }
    Ok(())
}

pub fn compile(tokens: &TokenStream) -> syn::Result<TokenStream> {
    let mut program: source::Program = syn::parse2(tokens.clone())?;
    for query in &mut program.rules {
        expand_query(query)?;
    }
    source::contract::check(&program)?;
    let target = program.to_token_stream();
    check(tokens, &target)?;
    Ok(target)
}
/// Validate each clause against the surface relation, without invoking expansion.
pub fn check(tokens: &TokenStream, target: &TokenStream) -> syn::Result<()> {
    let original: source::Program = syn::parse2(tokens.clone())?;
    let target: source::Program = syn::parse2(target.clone())?;
    source::contract::check(&target)?;
    ensure(
        original.name == target.name
            && original.declarations == target.declarations
            && original.outputs == target.outputs
            && original.observer == target.observer
            && original.models == target.models
            && original.rules.len() == target.rules.len(),
        &target,
        "surface expansion changed declarations or rule count",
    )?;
    for (before, after) in original.rules.iter().zip(&target.rules) {
        ensure(
            before.head == after.head && before.body.len() == after.body.len(),
            after,
            "surface expansion changed head or clause count",
        )?;
        let mut expected = before.clone();
        for (item, actual) in expected.body.iter_mut().zip(&after.body) {
            if let BodyItem::Filter(filter) = item {
                if let Expr::Let(binding) = rust::unparen(&filter.condition) {
                    let pattern = &binding.pat;
                    let expression = &binding.expr;
                    let bindings = rust::bindings(pattern);
                    let generator: BodyItem = syn::parse2(quote!(
                        for (#(#bindings,)*) in match #expression {
                            #pattern => ::core::option::Option::Some((#(#bindings,)*)),
                            _ => ::core::option::Option::None,
                        }
                    ))?;
                    ensure(
                        &generator == actual,
                        actual,
                        "if-let generator does not match pattern and expression",
                    )?;
                    *item = generator;
                }
            }
        }
        // Reclassification is syntax, not expansion: new binders can turn
        // subsequent CQ conditions into equalities or constraints.
        let expected: Query = syn::parse2(expected.to_token_stream())?;
        ensure(
            &expected == after,
            after,
            "surface expansion changed written clauses",
        )?;
    }
    Ok(())
}
