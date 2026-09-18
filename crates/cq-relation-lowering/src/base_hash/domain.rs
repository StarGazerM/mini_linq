//! Independent domain and required coverage of the input-indexed realization.
use cq_ir::{index_requirements, relational_plan, symbol_name as symbol};
use std::collections::{BTreeMap, BTreeSet};
/// Whether the input-index strategy can consume this relational plan.
///
/// The base-index route accepts a root Rename, earlier right-hand Renames
/// consumed one-by-one by left-deep NaturalJoins, and a final Project.
/// Result-index selection has its own correspondence and accepts broader RA.
pub fn supported_linear_plan(source: &relational_plan::Module) -> bool {
    if !relational_plan::contract::well_formed(source) {
        return false;
    }

    let mut earlier = BTreeMap::<String, (BTreeSet<String>, bool)>::new();
    let mut current = None::<String>;
    let mut unconsumed_renames = BTreeSet::<String>::new();
    let mut projected = false;

    for definition in &source.plan.definitions {
        let result = symbol(&definition.result);
        let heading = match &definition.operator {
            relational_plan::Operator::Unit(_)
            | relational_plan::Operator::Semijoin(_)
            | relational_plan::Operator::Extend(_)
            | relational_plan::Operator::Select(_) => return false,
            relational_plan::Operator::Rename(rename) => {
                if projected {
                    return false;
                }
                if current.is_none() {
                    current = Some(result.clone());
                } else {
                    unconsumed_renames.insert(result.clone());
                }
                let heading = rename
                    .mapping
                    .iter()
                    .map(|mapping| symbol(&mapping.target))
                    .collect();
                earlier.insert(result, (heading, true));
                continue;
            }
            relational_plan::Operator::NaturalJoin(operator) => {
                let right = symbol(&operator.right);
                let Some(left) = current.as_deref() else {
                    return false;
                };
                let Some((left_heading, _)) = earlier.get(left) else {
                    return false;
                };
                let Some((right_heading, true)) = earlier.get(&right) else {
                    return false;
                };
                if projected || symbol(&operator.left) != left || !unconsumed_renames.remove(&right)
                {
                    return false;
                }
                let mut heading = left_heading.clone();
                heading.extend(right_heading.iter().cloned());
                current = Some(result.clone());
                heading
            }
            relational_plan::Operator::Project(operator) => {
                if projected
                    || !unconsumed_renames.is_empty()
                    || current.as_deref() != Some(symbol(&operator.input).as_str())
                {
                    return false;
                }
                projected = true;
                current = Some(result.clone());
                operator.attributes.iter().map(symbol).collect()
            }
        };
        earlier.insert(result, (heading, false));
    }

    projected
        && unconsumed_renames.is_empty()
        && current
            .as_deref()
            .is_some_and(|current| current == symbol(&source.plan.output.input))
}

/// Whether a locally valid module contains every canonical required index.
///
/// Local IndexRequirements validation deliberately permits extra indexes and
/// does not prove that the written list covers the relational plan's named
/// right-hand input occurrences. The physical entry also checks coverage while
/// lowering and reports a missing key at its corresponding occurrence.
pub fn covers_required_indexes(source: &index_requirements::Module) -> bool {
    let available = source
        .indexes
        .iter()
        .map(index_signature)
        .collect::<BTreeSet<_>>();
    canonical_indexes(&source.relational_plan).is_some_and(|required| {
        required
            .into_iter()
            .all(|required| available.contains(&required))
    })
}

pub fn index_signature(index: &index_requirements::IndexRequirement) -> (String, Vec<usize>) {
    (
        cq_ir::symbol_name(&index.relation),
        index
            .key_columns
            .iter()
            .map(|column| column.index as usize)
            .collect(),
    )
}

pub fn canonical_indexes(source: &relational_plan::Module) -> Option<Vec<(String, Vec<usize>)>> {
    let declarations = source
        .program
        .inputs()
        .map(|input| (symbol(&input.name), input))
        .collect::<BTreeMap<_, _>>();
    let mut earlier = BTreeMap::<String, (&relational_plan::Definition, BTreeSet<String>)>::new();
    let mut seen = BTreeSet::new();
    let mut requirements = Vec::new();

    for definition in &source.plan.definitions {
        let heading = match &definition.operator {
            relational_plan::Operator::Unit(_)
            | relational_plan::Operator::Semijoin(_)
            | relational_plan::Operator::Extend(_)
            | relational_plan::Operator::Select(_) => return None,
            relational_plan::Operator::Rename(rename) => rename
                .mapping
                .iter()
                .map(|mapping| symbol(&mapping.target))
                .collect(),
            relational_plan::Operator::NaturalJoin(join) => {
                let (_, left_heading) = earlier.get(&symbol(&join.left))?;
                let (right_definition, right_heading) = earlier.get(&symbol(&join.right))?;
                let relational_plan::Operator::Rename(rename) = &right_definition.operator else {
                    return None;
                };
                let declaration = declarations.get(&symbol(&rename.relation))?;
                let shared = left_heading
                    .intersection(right_heading)
                    .cloned()
                    .collect::<BTreeSet<_>>();
                let mapping = rename
                    .mapping
                    .iter()
                    .map(|attribute| (symbol(&attribute.source), symbol(&attribute.target)))
                    .collect::<BTreeMap<_, _>>();
                let columns = declaration
                    .columns
                    .iter()
                    .enumerate()
                    .filter_map(|(column, declaration)| {
                        mapping
                            .get(&symbol(&declaration.name))
                            .is_some_and(|target| shared.contains(target))
                            .then_some(column)
                    })
                    .collect::<Vec<_>>();
                let signature = (symbol(&rename.relation), columns);
                if !signature.1.is_empty() && seen.insert(signature.clone()) {
                    requirements.push(signature);
                }

                let mut heading = left_heading.clone();
                heading.extend(right_heading.iter().cloned());
                heading
            }
            relational_plan::Operator::Project(project) => {
                project.attributes.iter().map(symbol).collect()
            }
        };
        earlier.insert(symbol(&definition.result), (definition, heading));
    }

    Some(requirements)
}
