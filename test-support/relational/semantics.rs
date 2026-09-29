//! Deliberately small, test-only semantics. CQ evaluates valuations against
//! base tables. RA separately interprets the parsed operator DAG using nested
//! loops. Neither evaluator calls compiler planning, heading, or key helpers.

use std::collections::{BTreeMap, BTreeSet};

use crate::ra;
use cq_syntax::symbol_name;

pub use crate::values::*;
pub struct Evaluation {
    pub definitions: BTreeMap<String, Relation>,
    pub output: Rows,
}

pub fn evaluate_ra(module: &ra::Program, database: &Database) -> Evaluation {
    evaluate_plan(module.plan.declarations.iter(), &module.plan, database)
}

pub fn evaluate_plan<'a>(
    inputs: impl Iterator<Item = &'a cq_syntax::RelationDecl> + Clone,
    plan: &ra::Plan,
    database: &Database,
) -> Evaluation {
    let mut definitions = BTreeMap::<String, Relation>::new();
    for definition in &plan.definitions {
        let relation = match &definition.operator {
            ra::Operator::Unit(_) => Relation {
                heading: BTreeSet::new(),
                rows: BTreeSet::from([Binding::new()]),
            },
            ra::Operator::Rename(rename) => {
                let input = inputs
                    .clone()
                    .find(|input| symbol_name(&input.name) == symbol_name(&rename.relation))
                    .unwrap();
                let mapping = rename
                    .mapping
                    .iter()
                    .map(|mapping| {
                        let column = input
                            .columns
                            .iter()
                            .position(|column| {
                                symbol_name(&column.name) == symbol_name(&mapping.source)
                            })
                            .unwrap();
                        (column, symbol_name(&mapping.target))
                    })
                    .collect::<Vec<_>>();
                Relation {
                    heading: mapping.iter().map(|(_, x)| x.clone()).collect(),
                    rows: database[&symbol_name(&rename.relation)]
                        .iter()
                        .map(|row| {
                            mapping
                                .iter()
                                .map(|(column, x)| (x.clone(), row[*column]))
                                .collect()
                        })
                        .collect(),
                }
            }
            ra::Operator::NaturalJoin(join) => {
                definitions[&symbol_name(&join.left)].join(&definitions[&symbol_name(&join.right)])
            }
            ra::Operator::Semijoin(join) => {
                let left = &definitions[&symbol_name(&join.left)];
                left.join(&definitions[&symbol_name(&join.right)])
                    .project(left.heading.clone())
            }
            ra::Operator::Select(select) => {
                let mut input = definitions[&symbol_name(&select.input)].clone();
                input
                    .rows
                    .retain(|row| evaluate_predicate(&select.condition, row));
                input
            }
            ra::Operator::Extend(_) => {
                panic!("Rust computations are tested by compiled end-to-end tests")
            }
            ra::Operator::Project(project) => definitions[&symbol_name(&project.input)]
                .project(project.attributes.iter().map(symbol_name).collect()),
        };
        assert!(
            definitions
                .insert(symbol_name(&definition.result), relation)
                .is_none()
        );
    }
    let output = definitions[&symbol_name(&plan.output.input)].output(
        &plan
            .output
            .head
            .variables()
            .map(symbol_name)
            .collect::<Vec<_>>(),
    );
    Evaluation {
        definitions,
        output,
    }
}
