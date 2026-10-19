//! Record the Boolean semi-naive cover and separate full/delta join layouts.

use crate::ensure;
use cq_lang_datalog::{BodyItem, Delta, Layout, Program, Variant, check_rewrite, contract};
use cq_syntax::{rust, symbol_name};
use proc_macro2::{Span, TokenStream, TokenTree};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use syn::Ident;

/// Concrete admission/key representation selected before scheduling.
/// Layout demands remain logical permutations; device consumers rebuild indexes per hole.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Representation {
    #[default]
    CpuHashSet,
    DeviceSet,
}

pub fn compile(source: &Program) -> syn::Result<Program> {
    compile_with_representation(source, Representation::CpuHashSet)
}

/// Schedule the same Boolean cover with an explicitly checked backend policy.
pub fn compile_with_representation(
    source: &Program,
    representation: Representation,
) -> syn::Result<Program> {
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
    let mut target = source.clone();
    target.desugar_if_let()?;
    for (rule, query) in target.rules.iter().enumerate() {
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
    target.deltas = deltas;
    target.variants = Some(variants);
    (target.layouts, target.delta_layouts) =
        choose_layouts(target.variants.as_ref().unwrap(), &target.rules);
    let (insertion, keys) = match representation {
        Representation::CpuHashSet => (
            syn::parse_quote!(hash_set_all_columns),
            syn::parse_quote!(hash_set_borrowed),
        ),
        Representation::DeviceSet => (
            syn::parse_quote!(device_set_all_columns),
            syn::parse_quote!(device_set_owned),
        ),
    };
    target.insertion = Some(insertion);
    target.negation_keys = Some(keys);
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
fn choose_layouts(
    variants: &[Variant],
    rules: &[cq_lang_datalog::Query],
) -> (Vec<Layout>, Vec<Layout>) {
    let mut layouts = Vec::new();
    let mut delta_layouts = Vec::new();
    let mut seen = BTreeSet::new();
    for variant in variants {
        let mut bound = BTreeSet::new();
        for (position, item) in variant.query.body.iter().enumerate() {
            let BodyItem::Positive { atom } = item else {
                continue;
            };
            let mut keys = Vec::new();
            let mut payload = Vec::new();
            for (column, argument) in atom.args.iter().enumerate() {
                let is_key = match rust::variable(argument) {
                    Some(id) => bound.contains(&symbol_name(id)),
                    None => !matches!(rust::unparen(argument), syn::Expr::Infer(_)),
                };
                if is_key {
                    keys.push(column);
                } else {
                    payload.push(column);
                }
            }
            let key = keys.len();
            keys.extend(payload);
            let is_delta = Some(position) == variant.delta;
            let relation = if is_delta {
                let BodyItem::Positive { atom: base } = &rules[variant.rule].body[position] else {
                    unreachable!("changing source position is a positive atom")
                };
                &base.relation
            } else {
                &atom.relation
            };
            if (!is_delta || key != 0 || position != 0)
                && seen.insert((is_delta, symbol_name(relation), keys.clone(), key))
            {
                let selected = if is_delta {
                    &mut delta_layouts
                } else {
                    &mut layouts
                };
                selected.push(Layout {
                    relation: relation.clone(),
                    columns: keys,
                    key,
                });
            }
            bound.extend(atom.variables().map(symbol_name));
        }
    }
    (layouts, delta_layouts)
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
