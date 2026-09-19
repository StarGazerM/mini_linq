//! RA preservation and canonical first-use naming, independently of construction.

use cq_ir::{index_requirements as ix, relational_plan as ra, symbol_name};
use std::collections::BTreeSet;

/// The IX contract verifies exact columns/versions/use coverage. This relation
/// additionally requires unchanged RA and deterministic first-use serialization.
pub fn check(source: &ra::Module, target: &ix::Module) -> bool {
    if target.relational_plan != *source || !ix::contract::well_formed(target) {
        return false;
    }
    let positions = source
        .plan
        .definitions
        .iter()
        .filter(|definition| {
            matches!(
                definition.operator,
                ra::Operator::NaturalJoin(_) | ra::Operator::Semijoin(_)
            )
        })
        .map(|definition| symbol_name(&definition.result));
    if !target
        .uses
        .iter()
        .map(|u| symbol_name(&u.result))
        .eq(positions)
    {
        return false;
    }
    let mut seen = BTreeSet::new();
    let first_uses = target
        .uses
        .iter()
        .filter_map(|u| {
            let name = symbol_name(&u.index);
            seen.insert(name.clone()).then_some(name)
        })
        .collect::<Vec<_>>();
    first_uses
        .iter()
        .enumerate()
        .all(|(i, name)| *name == format!("i{i}"))
        && target
            .indexes
            .iter()
            .map(|i| symbol_name(&i.name))
            .eq(first_uses)
}
