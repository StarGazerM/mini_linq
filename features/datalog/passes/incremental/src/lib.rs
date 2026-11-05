//! Record the exact Boolean semi-naive cover without storage decisions.
use cq_datalog_support::{Fresh, ensure};
/// ADM-InPlan advice carried by [`RulePlan::known`].
pub use cq_lang_datalog_incremental::Known;
use cq_lang_datalog_incremental::{AdviceStep, Delta, Program, Variant, contract};
use cq_lang_datalog_strata as strata;
/// CQ `plan(order(...))` atom selectors, shared by rule plan advice.
pub use cq_order::OrderAtom;
use cq_syntax::symbol_name;
use std::collections::{BTreeMap, BTreeSet};
use strata::source::{BodyItem, Query};
/// Authored scan-driver policy, retained in Incremental and CQ/VO facts.
pub use cq_lang_datalog_incremental::DriverPolicy;
/// Authored CQ step order, Physical sites and admission placement for a
/// labelled rule or one version.
///
/// The selectors and ordering meaning are CQ `plan(order(...))` advice:
/// `Relation` or `Relation@clause` names a positive atom by its source-body
/// clause, named atoms run first in the given order, and unnamed atoms keep
/// written priority after them. `delta` selects the semi-naive version whose
/// changing occurrence is that atom; without it the advice targets every
/// version of the rule. A version-specific plan overrides a rule-wide plan.
/// An empty `order` (known-only advice) keeps the driver-policy order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RulePlan {
    pub rule: syn::Ident,
    pub delta: Option<OrderAtom>,
    pub order: Vec<OrderAtom>,
    /// Exact occurrence sets; each must name an admissible intersection after CQ lowering.
    pub smallest: Vec<Vec<OrderAtom>>,
    /// `known(admit)` or `known(at(level k))` (ADM-InPlan advice).
    pub known: Option<Known>,
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
            let plan = plans.get(&(rule, None));
            let authored = plan.filter(|plan| plan.order.is_some());
            let order = authored
                .and_then(|plan| plan.order.clone())
                .unwrap_or_else(|| identity.clone());
            variants.push(Variant {
                rule,
                delta: None,
                query: permuted(query.clone(), &order),
                authored: authored.is_some(),
                steps: authored.map_or_else(Vec::new, |plan| plan.steps.clone()),
                smallest: authored.map_or_else(Vec::new, |plan| plan.smallest.clone()),
                known: plan.and_then(|plan| plan.known),
                order,
            });
        }
        for &position in positions {
            let mut query = query.clone();
            let BodyItem::Positive { atom } = &mut query.body[position] else {
                unreachable!("checked changing position")
            };
            atom.relation = names[&symbol_name(&atom.relation)].clone();
            let plan = plans
                .get(&(rule, Some(position)))
                .or_else(|| plans.get(&(rule, None)));
            let authored = plan.filter(|plan| plan.order.is_some());
            let order = match authored.and_then(|plan| plan.order.clone()) {
                Some(order) => order,
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
                steps: authored.map_or_else(Vec::new, |plan| plan.steps.clone()),
                smallest: authored.map_or_else(Vec::new, |plan| plan.smallest.clone()),
                known: plan.and_then(|plan| plan.known),
                query,
            });
        }
    }
    let target = Program {
        strata: source.clone(),
        variants,
        deltas,
        driver: driver_policy,
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

struct ResolvedPlan {
    /// `None` for known-only advice: the version keeps its driver order.
    order: Option<Vec<usize>>,
    steps: Vec<AdviceStep>,
    smallest: Vec<Vec<usize>>,
    known: Option<Known>,
}

/// Resolve advice to source-occurrence certificates keyed by `(rule, delta
/// position)`. Incremental checks erasure; CQ checks the eventual step plan.
fn resolve_plans(
    source: &strata::Program,
    analysis: &strata::Analysis,
    plans: &[RulePlan],
) -> syn::Result<BTreeMap<(usize, Option<usize>), ResolvedPlan>> {
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
            None => None,
        };
        ensure(
            !plan.order.is_empty() || plan.known.is_some(),
            &plan.rule,
            "plan advice needs order(...) or known(...)",
        )?;
        ensure(
            !plan.order.is_empty() || plan.smallest.is_empty(),
            &plan.rule,
            "smallest(...) requires order(...) advice",
        )?;
        let prefix = positive_prefix(query);
        let mut order = Vec::with_capacity(query.body.len());
        let explicit_steps = plan.order.iter().any(|selector| selector.columns.is_some());
        let mut steps = Vec::new();
        for selector in &plan.order {
            // Whole-occurrence advice retains its established permutation.
            // Step advice stays attached to original positions until CQ lowering.
            let position = selector.resolve_in(&query.body)?;
            ensure(
                matches!(query.body[position], BodyItem::Positive { .. })
                    && prefix.contains(&position),
                &selector.relation,
                "plan order can only name atoms of the rule's first contiguous positive prefix",
            )?;
            if !explicit_steps {
                ensure(
                    !order.contains(&position),
                    &selector.relation,
                    "duplicate atom occurrence in order advice",
                )?;
                order.push(position);
            }
            steps.push(AdviceStep {
                occurrence: position,
                columns: selector.columns.clone(),
            });
        }
        let mut complete: Vec<_> = (0..prefix.start).collect();
        complete.extend(&order);
        complete.extend(prefix.clone().filter(|position| !order.contains(position)));
        complete.extend(prefix.end..query.body.len());
        if !explicit_steps {
            steps.clear();
        }
        let smallest = plan
            .smallest
            .iter()
            .map(|site| {
                ensure(
                    site.len() >= 2,
                    &plan.rule,
                    "smallest requires at least two occurrences",
                )?;
                let mut positions = Vec::new();
                for selector in site {
                    ensure(
                        selector.columns.is_none(),
                        &selector.relation,
                        "smallest selects whole occurrences",
                    )?;
                    let position = selector.resolve_in(&query.body)?;
                    ensure(
                        prefix.contains(&position),
                        &selector.relation,
                        "smallest requires positive prefix occurrences",
                    )?;
                    ensure(
                        !positions.contains(&position),
                        &selector.relation,
                        "duplicate smallest occurrence",
                    )?;
                    positions.push(position);
                }
                Ok(positions)
            })
            .collect::<syn::Result<Vec<_>>>()?;
        ensure(
            resolved
                .insert(
                    (rule, delta),
                    ResolvedPlan {
                        order: (!plan.order.is_empty()).then_some(complete),
                        steps,
                        smallest,
                        known: plan.known,
                    },
                )
                .is_none(),
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
