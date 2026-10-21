//! Erasure and component obligations for source frozen-filter certificates.
use super::*;
use std::collections::BTreeSet;

/// Remove certified helpers without changing the original component partition.
/// As with source erasure, callers must admit the certificate before relying on it.
pub fn erase(program: &Program) -> syn::Result<Program> {
    let helpers: BTreeSet<_> = program.source.invariants.iter()
        .map(|entry| symbol_name(&entry.helper)).collect();
    let mut components = program.components.clone();
    for members in &mut components {
        members.retain(|member| !helpers.contains(&symbol_name(member)));
    }
    components.retain(|members| !members.is_empty());
    Ok(Program {
        source: source::invariants::erase(&program.source)?,
        components,
    })
}

// Source certificates and the factored partition have already been checked.
pub(crate) fn check(program: &Program, graph: &Graph) -> syn::Result<()> {
    if program.source.invariants.is_empty() {
        return Ok(());
    }
    let original = erase(program)?;
    let original_graph = Graph::new(&original.source)?;
    check_strata(&original, &original_graph)?;
    let mut component = vec![None; graph.names.len()];
    for (index, members) in program.components.iter().enumerate() {
        for member in members {
            component[graph.index(member)?] = Some(index);
        }
    }
    for entry in &program.source.invariants {
        let consumer = component[graph.index(&entry.source.head.relation)?];
        let helper = component[graph.index(&entry.helper)?];
        ensure(
            helper.is_some() && helper < consumer,
            program,
            "invariant helper must finish before its consumer",
        )?;
        let definition = program.source.rules.iter()
            .find(|rule| rule.head.relation == entry.helper)
            .expect("source certificate checked the helper definition");
        for item in &definition.body {
            let input = match item {
                BodyItem::Positive { atom } => &atom.relation,
                BodyItem::Negation(negative) => &negative.atom.relation,
                _ => unreachable!("source certificate checked the bounded filter"),
            };
            let input = component[graph.index(input)?];
            ensure(
                input.is_none() || input < consumer,
                program,
                "invariant input must be frozen before its consumer",
            )?;
        }
    }
    Ok(())
}
