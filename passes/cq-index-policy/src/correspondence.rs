//! RA preservation and canonical first-use naming, independently of construction.

use cq_lang_ix as ix;
use cq_lang_ra as ra;
use cq_lang_ra::syntax::symbol_name;
use std::collections::BTreeSet;

/// IX checks that each demand is used. This relation additionally requires
/// unchanged RA, complete demands, and deterministic first-use serialization.
pub fn check(source: &ra::Module, target: &ix::Section) -> bool {
    let Ok(env) = ix::reference::Env::new(source) else {
        return false;
    };
    if !ix::contract::well_formed(target, &env) {
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
                ix::result_join_columns(
                    &info[&symbol_name(left)]
                        .heading
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>(),
                    &info[&symbol_name(right)]
                        .heading
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>(),
                ),
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
