//! Record the exact Boolean semi-naive cover without storage decisions.
use cq_datalog_support::{Fresh, ensure};
use cq_lang_datalog_incremental::{Delta, Program, Variant, contract};
use cq_lang_datalog_strata as strata;
use cq_syntax::symbol_name;
use std::collections::{BTreeMap, BTreeSet};
use strata::source::BodyItem;
/// Authored scan-driver advice; the selected permutation is checked in the IR.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DriverPolicy {
    #[default]
    WrittenOrder,
    /// Lead with delta, then prefer connected reads in the first positive prefix.
    DeltaFirst,
}
pub fn compile(source: &strata::Program) -> syn::Result<Program> {
    compile_with_driver(source, DriverPolicy::WrittenOrder)
}

pub fn compile_with_driver(source: &strata::Program, driver_policy: DriverPolicy) -> syn::Result<Program> {
    let analysis = strata::contract::check(source)?;
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
            variants.push(Variant {
                rule,
                delta: None,
                order: identity.clone(),
                query: query.clone(),
            });
        }
        for &position in positions {
            let mut query = query.clone();
            let BodyItem::Positive { atom } = &mut query.body[position] else {
                unreachable!("checked changing position")
            };
            atom.relation = names[&symbol_name(&atom.relation)].clone();
            let order = match driver_policy {
                DriverPolicy::WrittenOrder => identity.clone(),
                DriverPolicy::DeltaFirst => delta_first_order(&query, position),
            };
            let mut body: Vec<_> = query.body.into_iter().collect();
            let mut current = identity.clone();
            for (target, &wanted) in order.iter().enumerate() {
                let source = current.iter().position(|&position| position == wanted).expect("complete order");
                body.swap(target, source);
                current.swap(target, source);
            }
            query.body = body.into_iter().collect();
            variants.push(Variant {
                rule,
                delta: Some(position),
                order,
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

fn delta_first_order(query: &strata::source::Query, delta: usize) -> Vec<usize> {
    let mut order: Vec<_> = (0..query.body.len()).collect();
    let Some(first) = query.body.iter().position(|item| matches!(item, BodyItem::Positive { .. })) else {
        return order;
    };
    let end = first + query.body.iter().skip(first)
        .take_while(|item| matches!(item, BodyItem::Positive { .. })).count();
    if !(first..end).contains(&delta) { return order; }
    order.swap(first, delta);
    let mut bound = BTreeSet::new();
    for position in first..end {
        if position != first {
            let next = (position..end).min_by_key(|&candidate| {
                let source = order[candidate];
                let BodyItem::Positive { atom } = &query.body[source] else { unreachable!("positive prefix") };
                (!atom.variables().any(|variable| bound.contains(&symbol_name(variable))), source)
            }).expect("nonempty suffix");
            order.swap(position, next);
        }
        let BodyItem::Positive { atom } = &query.body[order[position]] else { unreachable!("positive prefix") };
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
