//! Record the Boolean semi-naive cover and separate full/delta join layouts.

use crate::ensure;
use cq_lang_datalog::{BodyItem, Delta, Layout, Program, Variant, check_rewrite, contract};
use cq_syntax::{rust, symbol_name};
use proc_macro2::{Span, TokenStream, TokenTree};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};
use syn::Ident;

/// Concrete admission/key representation selected before scheduling.
/// Layout demands remain logical permutations; the fork records their lifecycle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Representation {
    #[default]
    CpuHashSet,
    DeviceSet,
}

/// Explicit per-variant outer-stream policy. CPU callers retain written order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DriverPolicy {
    #[default]
    WrittenOrder,
    DeltaFirst,
}

pub fn compile(source: &Program) -> syn::Result<Program> {
    compile_with_representation(source, Representation::CpuHashSet)
}

/// Schedule the same Boolean cover with an explicitly checked backend policy.
pub fn compile_with_representation(
    source: &Program,
    representation: Representation,
) -> syn::Result<Program> {
    compile_with_driver(source, representation, DriverPolicy::WrittenOrder)
}

pub fn compile_with_driver(
    source: &Program,
    representation: Representation,
    driver_policy: DriverPolicy,
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
        let first = query
            .body
            .iter()
            .position(|item| matches!(item, BodyItem::Positive { .. }));
        if positions.is_empty() {
            variants.push(Variant {
                rule,
                delta: None,
                driver: first,
                query: query.clone(),
            });
        }
        for &position in positions {
            let mut query = query.clone();
            let BodyItem::Positive { atom } = &mut query.body[position] else {
                unreachable!("contract changing positions are positive atoms")
            };
            atom.relation = names[&symbol_name(&atom.relation)].clone();
            let driver = if driver_policy == DriverPolicy::DeltaFirst
                && first.is_some_and(|first| {
                    query
                        .body
                        .iter()
                        .skip(first)
                        .take(position - first + 1)
                        .all(|item| matches!(item, BodyItem::Positive { .. }))
                }) {
                Some(position)
            } else {
                first
            };
            if let (Some(first), Some(driver)) = (first, driver) {
                let mut body: Vec<_> = query.body.into_iter().collect();
                let item = body.remove(driver);
                body.insert(first, item);
                query.body = body.into_iter().collect();
            }
            variants.push(Variant {
                rule,
                delta: Some(position),
                driver,
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
        let order = cq_lang_datalog::variant_order(&rules[variant.rule], variant.driver)
            .expect("new driver preserves positive-prefix Before constraints");
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
            let source_position = order[position];
            let is_delta = Some(source_position) == variant.delta;
            let relation = if is_delta {
                let BodyItem::Positive { atom: base } = &rules[variant.rule].body[source_position]
                else {
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
