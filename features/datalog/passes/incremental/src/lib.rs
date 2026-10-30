//! Record the exact Boolean semi-naive cover without storage decisions.
use cq_datalog_support::{Fresh, ensure};
use cq_lang_datalog_incremental::{Delta, Program, Variant, contract};
use cq_lang_datalog_strata as strata;
/// CQ `plan(order(...))` atom selectors, shared by rule plan advice.
pub use cq_order::OrderAtom;
use cq_syntax::symbol_name;
use std::collections::{BTreeMap, BTreeSet};
use strata::source::{BodyItem, Query};
/// Authored scan-driver advice; the selected permutation is checked in the IR.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DriverPolicy {
    #[default]
    WrittenOrder,
    /// Lead with delta, then prefer connected reads in the first positive prefix.
    DeltaFirst,
}
/// Authored join order for one version of one labelled rule.
///
/// The selectors and ordering meaning are CQ `plan(order(...))` advice:
/// `Relation` or `Relation@clause` names a positive atom by its source-body
/// clause, named atoms run first in the given order, and unnamed atoms keep
/// written priority after them. `delta` selects the semi-naive version whose
/// changing occurrence is that atom; without it the advice targets the
/// rule's only, non-recursive version.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RulePlan {
    pub rule: syn::Ident,
    pub delta: Option<OrderAtom>,
    pub order: Vec<OrderAtom>,
}
pub fn compile(source: &strata::Program) -> syn::Result<Program> {
    compile_with_driver(source, DriverPolicy::WrittenOrder)
}

pub fn compile_with_driver(
    source: &strata::Program,
    driver_policy: DriverPolicy,
) -> syn::Result<Program> {
    compile_with_advice(source, driver_policy, &[])
}

/// Advised versions use their plan; every other version uses `driver_policy`.
pub fn compile_with_advice(
    source: &strata::Program,
    driver_policy: DriverPolicy,
    plans: &[RulePlan],
) -> syn::Result<Program> {
    let analysis = strata::contract::check(source)?;
    let plans = resolve_plans(source, &analysis, plans)?;
    let mut fresh = Fresh::new(source);
    let mut deltas = Vec::new();
    let mut names = BTreeMap::new();
    for declaration in source
        .source
        .declarations
        .iter()
        .filter(|d| d.value.is_none())
    {
        let name = fresh.id("delta");
        names.insert(symbol_name(&declaration.name), name.clone());
        deltas.push(Delta {
            relation: declaration.name.clone(),
            name,
        });
    }
    let mut variants = Vec::new();
    for (rule, query) in source.source.rules.iter().enumerate() {
        let positions = &analysis.changing_positions[rule];
        let identity: Vec<_> = (0..query.body.len()).collect();
        if positions.is_empty() {
            let authored = plans.get(&(rule, None));
            let order = authored.cloned().unwrap_or_else(|| identity.clone());
            variants.push(Variant {
                rule,
                delta: None,
                query: permuted(query.clone(), &order),
                authored: authored.is_some(),
                order,
            });
        }
        for &position in positions {
            let mut query = query.clone();
            let BodyItem::Positive { atom } = &mut query.body[position] else {
                unreachable!("checked changing position")
            };
            atom.relation = names[&symbol_name(&atom.relation)].clone();
            let authored = plans.get(&(rule, Some(position)));
            let order = match authored {
                Some(order) => order.clone(),
                None => match driver_policy {
                    DriverPolicy::WrittenOrder => identity.clone(),
                    DriverPolicy::DeltaFirst => delta_first_order(&query, position),
                },
            };
            let query = permuted(query, &order);
            variants.push(Variant {
                rule,
                delta: Some(position),
                order,
                authored: authored.is_some(),
                query,
            });
        }
    }
    let target = Program {
        strata: source.clone(),
        variants,
        deltas,
    };
    check(source, &target)?;
    Ok(target)
}

fn permuted(mut query: Query, order: &[usize]) -> Query {
    let mut body: Vec<_> = query.body.into_iter().collect();
    let mut current: Vec<_> = (0..order.len()).collect();
    for (target, &wanted) in order.iter().enumerate() {
        let source = current
            .iter()
            .position(|&position| position == wanted)
            .expect("complete order");
        body.swap(target, source);
        current.swap(target, source);
    }
    query.body = body.into_iter().collect();
    query
}

/// Conservative producer policy; the contract also admits dependency-safe crossings.
fn positive_prefix(query: &Query) -> std::ops::Range<usize> {
    let length = query.body.len();
    let first = query
        .body
        .iter()
        .position(|item| matches!(item, BodyItem::Positive { .. }))
        .unwrap_or(length);
    let end = first
        + query
            .body
            .iter()
            .skip(first)
            .take_while(|item| matches!(item, BodyItem::Positive { .. }))
            .count();
    first..end
}

/// Resolve advice to complete source-occurrence permutations keyed by
/// `(rule, delta position)`. The incremental contract checks the result.
fn resolve_plans(
    source: &strata::Program,
    analysis: &strata::Analysis,
    plans: &[RulePlan],
) -> syn::Result<BTreeMap<(usize, Option<usize>), Vec<usize>>> {
    let mut resolved = BTreeMap::new();
    for plan in plans {
        let rule = source
            .source
            .labels
            .iter()
            .position(|label| label.as_ref().is_some_and(|label| *label == plan.rule))
            .ok_or_else(|| {
                syn::Error::new_spanned(&plan.rule, "plan advice names no labelled rule")
            })?;
        ensure(
            source
                .source
                .invariants
                .iter()
                .all(|entry| entry.rule != rule),
            &plan.rule,
            "plan advice cannot order a rule rewritten by invariant hoisting",
        )?;
        let query = &source.source.rules[rule];
        let changing = &analysis.changing_positions[rule];
        let delta = match &plan.delta {
            Some(selector) => {
                ensure(
                    selector.columns.is_none(),
                    &selector.relation,
                    "delta advice selects a whole occurrence, not subatom columns",
                )?;
                let position = selector.resolve_in(&query.body)?;
                ensure(
                    changing.contains(&position),
                    &selector.relation,
                    "delta advice must select a changing occurrence of this rule",
                )?;
                Some(position)
            }
            None => {
                ensure(
                    changing.is_empty(),
                    &plan.rule,
                    "this rule has delta versions; select one with delta(Relation@clause)",
                )?;
                None
            }
        };
        let prefix = positive_prefix(query);
        let mut order = Vec::with_capacity(query.body.len());
        for selector in &plan.order {
            ensure(
                selector.columns.is_none(),
                &selector.relation,
                "incremental order advice is an occurrence permutation; use CQ plan order for subatoms",
            )?;
            let position = selector.resolve_in(&query.body)?;
            ensure(
                matches!(query.body[position], BodyItem::Positive { .. })
                    && prefix.contains(&position),
                &selector.relation,
                "plan order can only name atoms of the rule's first contiguous positive prefix",
            )?;
            ensure(
                !order.contains(&position),
                &selector.relation,
                "duplicate atom occurrence in order advice",
            )?;
            order.push(position);
        }
        let mut complete: Vec<_> = (0..prefix.start).collect();
        complete.extend(&order);
        complete.extend(prefix.clone().filter(|position| !order.contains(position)));
        complete.extend(prefix.end..query.body.len());
        ensure(
            resolved.insert((rule, delta), complete).is_none(),
            &plan.rule,
            "duplicate plan advice for this rule version",
        )?;
    }
    Ok(resolved)
}

fn delta_first_order(query: &strata::source::Query, delta: usize) -> Vec<usize> {
    let mut order: Vec<_> = (0..query.body.len()).collect();
    let Some(first) = query
        .body
        .iter()
        .position(|item| matches!(item, BodyItem::Positive { .. }))
    else {
        return order;
    };
    let end = first
        + query
            .body
            .iter()
            .skip(first)
            .take_while(|item| matches!(item, BodyItem::Positive { .. }))
            .count();
    if !(first..end).contains(&delta) {
        return order;
    }
    order.swap(first, delta);
    let mut bound = BTreeSet::new();
    for position in first..end {
        if position != first {
            let next = (position..end)
                .min_by_key(|&candidate| {
                    let source = order[candidate];
                    let BodyItem::Positive { atom } = &query.body[source] else {
                        unreachable!("positive prefix")
                    };
                    (
                        !atom
                            .variables()
                            .any(|variable| bound.contains(&symbol_name(variable))),
                        source,
                    )
                })
                .expect("nonempty suffix");
            order.swap(position, next);
        }
        let BodyItem::Positive { atom } = &query.body[order[position]] else {
            unreachable!("positive prefix")
        };
        bound.extend(atom.variables().map(symbol_name));
    }
    order
}
/// Independently check cover, erasure, fresh schemas, and the embedded Strata source.
/// Formal rule DL-OccurrenceOrder: independently validate the supplied occurrence permutation.
pub fn check(source: &strata::Program, target: &Program) -> syn::Result<()> {
    contract::check(target)?;
    ensure(
        source == &target.strata,
        target,
        "incremental changed strata",
    )
}
