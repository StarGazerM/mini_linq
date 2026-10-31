//! Column demand of the checked Physical DAG under a Structures program.
//!
//! `carried_columns` is the set of logical columns each result must carry:
//! answer reads, canonical valuations, recorded keys and payload layouts, and
//! every consumer's demand, propagated backwards. Printers carry exactly these
//! columns; index selection reads them as the recorded read needs of a build.
use std::collections::{BTreeMap, BTreeSet};

use crate::physical::Operator;
use crate::symbol_name;
use crate::syntax::{algebra::Environment, algebra::Theta, rust};
use syn::{Expr, Ident};

/// Logical columns read by a declarative predicate.
pub fn columns_used(theta: &Theta) -> BTreeSet<String> {
    theta
        .constraints
        .iter()
        .flat_map(crate::syntax::Constraint::variables)
        .chain(
            theta
                .equalities
                .iter()
                .flat_map(crate::syntax::Equality::variables),
        )
        .map(symbol_name)
        .collect()
}

/// Logical columns read by Rust, restricted to its written environment.
/// Opaque syntax conservatively reads the whole lexical environment.
pub fn columns_used_rust(
    expression: &Expr,
    heading: &[String],
    environment: Option<&Environment>,
) -> BTreeSet<String> {
    let names = environment
        .map(|e| e.variables.iter().map(symbol_name).collect())
        .unwrap_or_else(|| heading.iter().cloned().collect());
    rust::free_variables(expression).dependencies(&names)
}

pub fn carried_columns(
    module: &crate::Program,
    analysis: &crate::Analysis,
) -> BTreeMap<String, Vec<String>> {
    let mut used: BTreeMap<String, BTreeSet<String>> = analysis
        .layouts
        .keys()
        .map(|n| (n.clone(), BTreeSet::new()))
        .collect();
    let returned = symbol_name(&module.physical.program.returned);
    for expression in &module.physical.program.head.args {
        used.get_mut(&returned).unwrap().extend(columns_used_rust(
            expression,
            &analysis.layouts[&returned],
            None,
        ));
    }
    // Canonical boundaries compare the complete logical valuation. Recorded
    // storage keys/payloads likewise remain independent of downstream projection.
    for definition in &module.physical.program.definitions {
        let name = symbol_name(&definition.name);
        if definition.canon.is_some() {
            used.get_mut(&name)
                .unwrap()
                .extend(analysis.layouts[&name].iter().cloned());
        }
        if let Some(builds) = analysis.built.get(&name) {
            for (levels, _) in builds {
                used.get_mut(&name).unwrap().extend(levels.concat());
            }
        }
        if let Some(filtered) = analysis.filtered.get(&name) {
            used.get_mut(&name)
                .unwrap()
                .extend(filtered.payload.columns.iter().map(symbol_name));
        }
        // Native host keyed protocols expose the declared payload layout. Their
        // lazy fallback must implement that identical layout, even for a reader
        // that ultimately projects some fields away.
        if analysis.host_read(&name)
            && (analysis.physical.key_reads.contains_key(&name)
                || analysis.physical.merge_reads.contains_key(&name))
        {
            used.get_mut(&name)
                .unwrap()
                .extend(analysis.layouts[&name].iter().cloned());
        }
    }
    for (name, key) in analysis.keysets.keys() {
        used.get_mut(name).unwrap().extend(key.iter().cloned());
    }
    propagate_columns(module, analysis, used)
}

/// Propagate demanded columns backwards through every consumer, from the
/// answer to the sources. Callers may add demands before propagation.
pub fn propagate_columns(
    module: &crate::Program,
    analysis: &crate::Analysis,
    mut used: BTreeMap<String, BTreeSet<String>>,
) -> BTreeMap<String, Vec<String>> {
    for definition in module.physical.program.definitions.iter().rev() {
        let name = symbol_name(&definition.name);
        let output = used[&name].clone();
        let mut demand = |input: &Ident, extra: BTreeSet<String>| {
            let input = symbol_name(input);
            let heading = &analysis.layouts[&input];
            used.get_mut(&input).unwrap().extend(
                output
                    .iter()
                    .chain(&extra)
                    .filter(|c| heading.contains(c))
                    .cloned(),
            );
        };
        match &definition.operator {
            Operator::Select(s) | Operator::Point(_, s) | Operator::Range(_, s) => {
                demand(&s.input, columns_used(&s.condition))
            }
            Operator::Guard(g) => demand(
                &g.input,
                columns_used_rust(
                    &g.condition,
                    &analysis.layouts[&symbol_name(&g.input)],
                    g.environment.as_ref(),
                ),
            ),
            Operator::Bind(b) => demand(
                &b.input,
                columns_used_rust(
                    &b.expression,
                    &analysis.layouts[&symbol_name(&b.input)],
                    b.environment.as_ref(),
                ),
            ),
            Operator::Probe(_, k) | Operator::Merge(_, k) => {
                let keys: BTreeSet<_> = k.columns.iter().map(symbol_name).collect();
                demand(&k.left, keys.clone());
                demand(&k.right, keys);
            }
            Operator::Semi(_, k) | Operator::Anti(_, k) => {
                let keys: BTreeSet<_> = k.columns.iter().map(symbol_name).collect();
                demand(&k.left, keys.clone());
                used.get_mut(&symbol_name(&k.right)).unwrap().extend(keys);
            }
            Operator::Project(p) => demand(&p.input, BTreeSet::new()),
            Operator::Group(group) => demand(
                &group.input,
                analysis.layouts[&symbol_name(&group.input)]
                    .iter()
                    .cloned()
                    .collect(),
            ),
            Operator::Union(u) => {
                for operand in &u.operands {
                    demand(operand, BTreeSet::new());
                }
            }
            Operator::Rename(_) | Operator::Unit(_) => {}
        }
    }
    used.into_iter()
        .map(|(n, cs)| (n, cs.into_iter().collect()))
        .collect()
}
