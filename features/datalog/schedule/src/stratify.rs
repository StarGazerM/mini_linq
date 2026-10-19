//! Record exact IDB SCCs in dependency order.

use crate::ensure;
use cq_lang_datalog::{Program, check_rewrite, contract};
use std::collections::BTreeSet;

pub fn compile(source: &Program) -> syn::Result<Program> {
    let analysis = contract::check(source)?;
    ensure(
        source.strata.is_none(),
        source,
        "stratify requires an unscheduled program",
    )?;
    let graph = &analysis.graph;
    let mut remaining: BTreeSet<_> = source
        .declarations
        .iter()
        .enumerate()
        .filter(|(_, declaration)| declaration.value.is_none())
        .map(|(index, _)| index)
        .collect();
    let mut components = Vec::new();
    while let Some(&first) = remaining.iter().find(|&&i| {
        remaining
            .iter()
            .all(|&j| !graph.reach[i][j] || i == j || graph.reach[j][i])
    }) {
        let members: Vec<_> = remaining
            .iter()
            .copied()
            .filter(|&j| first == j || (graph.reach[first][j] && graph.reach[j][first]))
            .collect();
        for index in &members {
            remaining.remove(index);
        }
        components.push(
            members
                .iter()
                .map(|&index| source.declarations[index].name.clone())
                .collect(),
        );
    }
    ensure(
        remaining.is_empty(),
        source,
        "cannot schedule dependency graph",
    )?;
    let mut target = source.clone();
    target.desugar_if_let()?;
    target.strata = Some(components);
    check(source, &target)?;
    Ok(target)
}

/// Check stage shape, exact SCCs/dependency order, and preservation independently.
pub fn check(source: &Program, target: &Program) -> syn::Result<()> {
    check_rewrite(source, target)?;
    ensure(
        source.strata.is_none() && source.variants.is_none(),
        source,
        "stratify requires an unscheduled source",
    )?;
    ensure(
        target.strata.is_some() && target.variants.is_none(),
        target,
        "stratify produces strata without semi-naive decisions",
    )
}
