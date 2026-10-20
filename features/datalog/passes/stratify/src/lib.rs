//! Choose deterministic dependency-ordered SCCs; the checker does not replay selection.
use cq_lang_datalog_source as source;
use cq_lang_datalog_strata as strata;
use source::ensure;
use std::collections::BTreeSet;
pub fn compile(source: &source::Program) -> syn::Result<strata::Program> {
    let analysis = source::contract::check(source)?;
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
    let target = strata::Program {
        source: source.clone(),
        components,
    };
    check(source, &target)?;
    Ok(target)
}

/// Check the target contract and exact source preservation independently.
pub fn check(source: &source::Program, target: &strata::Program) -> syn::Result<()> {
    source::contract::check(source)?;
    strata::contract::check(target)?;
    ensure(source == &target.source, target, "stratify changed source")
}
