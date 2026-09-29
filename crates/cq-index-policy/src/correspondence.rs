//! RA preservation and canonical first-use naming, independently of construction.

use cq_ir::{index_requirements as ix, relational_plan as ra, symbol_name};
use std::collections::BTreeSet;

/// IX checks that each demand is used. This relation additionally requires
/// unchanged RA, complete demands, and deterministic first-use serialization.
pub fn check(source: &ra::Module, target: &ix::Module) -> bool {
    if target.relational_plan != *source || !ix::contract::well_formed(target) {
        return false;
    }
    let Ok(info) = ra::check_in(source.program.inputs(), &source.plan) else {
        return false;
    };
    let mut seen = BTreeSet::new();
    let demands = source
        .plan
        .definitions
        .iter()
        .filter_map(|definition| {
            let (left, right) = match &definition.operator {
                ra::Operator::NaturalJoin(join) => (&join.left, &join.right),
                ra::Operator::Semijoin(join) => (&join.left, &join.right),
                _ => return None,
            };
            Some((
                symbol_name(right),
                ix::result_join_columns(&info[&symbol_name(left)], &info[&symbol_name(right)]),
            ))
        })
        .filter(|demand| seen.insert(demand.clone()));
    target
        .indexes
        .iter()
        .map(|index| ix::signature(&index.requirement))
        .eq(demands)
        && target
            .indexes
            .iter()
            .enumerate()
            .all(|(i, index)| symbol_name(&index.name) == format!("i{i}"))
}
