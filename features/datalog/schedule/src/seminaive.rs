//! Record the Boolean semi-naive cover and maintained delta-first join layouts.

use crate::ensure;
use cq_lang_datalog::{BodyItem, Delta, Layout, Program, Variant, check_rewrite, contract};
use cq_syntax::{rust, symbol_name};
use proc_macro2::{Span, TokenStream, TokenTree};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use syn::Ident;

pub fn compile(source: &Program) -> syn::Result<Program> {
    let analysis = contract::check(source)?;
    ensure(
        source.strata.is_some() && source.variants.is_none(),
        source,
        "seminaive requires strata and no variants",
    )?;
    let mut used = BTreeSet::new();
    collect_names(source.to_token_stream(), &mut used);
    let mut next = 0;
    let mut deltas = Vec::new();
    let mut names = BTreeMap::new();
    for declaration in source.declarations.iter().filter(|d| d.value.is_none()) {
        let name = loop {
            let name = format!("__cq_datalog_delta_{next}");
            next += 1;
            if used.insert(name.clone()) {
                break Ident::new(&name, Span::mixed_site());
            }
        };
        names.insert(symbol_name(&declaration.name), name.clone());
        deltas.push(Delta {
            relation: declaration.name.clone(),
            name,
        });
    }
    let mut variants = Vec::new();
    for (rule, query) in source.rules.iter().enumerate() {
        let positions = &analysis.changing_positions[rule];
        if positions.is_empty() {
            variants.push(Variant {
                rule,
                delta: None,
                query: query.clone(),
            });
        }
        for &position in positions {
            let mut query = query.clone();
            let BodyItem::Positive { atom } = &mut query.body[position] else {
                unreachable!("contract changing positions are positive atoms")
            };
            atom.relation = names[&symbol_name(&atom.relation)].clone();
            variants.push(Variant {
                rule,
                delta: Some(position),
                query,
            });
        }
    }
    let mut target = source.clone();
    target.deltas = deltas;
    target.variants = Some(variants);
    target.layouts = choose_layouts(target.variants.as_ref().unwrap());
    target.insertion = Some(syn::parse_quote!(hash_set_all_columns));
    target.negation_keys = Some(syn::parse_quote!(hash_set_borrowed));
    check(source, &target)?;
    Ok(target)
}

/// Check stage shape, declaration freshness, complete variant contracts/erasure,
/// exact changing-position coverage, and preservation without regenerating names.
pub fn check(source: &Program, target: &Program) -> syn::Result<()> {
    check_rewrite(source, target)?;
    ensure(
        source.strata.is_some() && source.variants.is_none(),
        source,
        "seminaive requires strata and no source variants",
    )?;
    ensure(
        target.strata.is_some() && target.variants.is_some(),
        target,
        "seminaive requires target variants",
    )
}

// Called only on newly emitted variants, which are in rule/position order.
// The language checker derives the same obligations independently from source rules.
fn choose_layouts(variants: &[Variant]) -> Vec<Layout> {
    let mut layouts = Vec::new();
    let mut seen = BTreeSet::new();
    for variant in variants {
        let mut bound = BTreeSet::new();
        if let Some(position) = variant.delta {
            let BodyItem::Positive { atom } = &variant.query.body[position] else {
                unreachable!("delta positions are positive")
            };
            bound.extend(atom.variables().map(symbol_name));
        }
        for (position, item) in variant.query.body.iter().enumerate() {
            if Some(position) == variant.delta {
                continue;
            }
            let BodyItem::Positive { atom } = item else {
                continue;
            };
            let mut keys = Vec::new();
            let mut payload = Vec::new();
            for (column, argument) in atom.args.iter().enumerate() {
                if rust::variable(argument).is_some_and(|id| bound.contains(&symbol_name(id))) {
                    keys.push(column);
                } else {
                    payload.push(column);
                }
            }
            let key = keys.len();
            keys.extend(payload);
            if seen.insert((symbol_name(&atom.relation), keys.clone(), key)) {
                layouts.push(Layout {
                    relation: atom.relation.clone(),
                    columns: keys,
                    key,
                });
            }
            bound.extend(atom.variables().map(symbol_name));
        }
    }
    layouts
}

fn collect_names(tokens: TokenStream, used: &mut BTreeSet<String>) {
    for token in tokens {
        match token {
            TokenTree::Ident(id) => {
                used.insert(symbol_name(&id));
            }
            TokenTree::Group(group) => collect_names(group.stream(), used),
            _ => {}
        }
    }
}
