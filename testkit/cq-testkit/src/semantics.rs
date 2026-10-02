//! Independent RA DAG interpretation with nested-loop integer relations.
//!
//! Assumes a contract-valid DAG, declared-width finite tables, Set semantics,
//! Leibniz equality, and the integer fragment documented at crate level.
//! Bind computations are unsupported. No compiler planning, heading analysis,
//! or key helpers are called; intermediate definitions remain inspectable.

use std::collections::{BTreeMap, BTreeSet};

use crate::ra;
use cq_syntax::symbol_name;

pub use crate::values::*;
/// All interpreted named definitions and the final ordered answer tuples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evaluation {
    pub definitions: BTreeMap<String, Relation>,
    pub output: Rows,
}

/// Interpret a complete RA program under the documented integer premises.
pub fn evaluate_ra(module: &ra::Program, database: &Database) -> Evaluation {
    evaluate_plan(module.plan.declarations.iter(), &module.plan, database)
}

/// Interpret an RA plan with externally supplied input declarations.
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
                input.rows.retain(|row| {
                    select
                        .condition
                        .equalities
                        .iter()
                        .all(|e| evaluate_equality(e, row))
                        && select
                            .condition
                            .constraints
                            .iter()
                            .all(|c| evaluate_constraint(c, row))
                });
                input
            }
            ra::Operator::Guard(guard) => {
                let mut input = definitions[&symbol_name(&guard.input)].clone();
                input
                    .rows
                    .retain(|row| evaluate_predicate(&guard.condition, row));
                input
            }
            ra::Operator::Bind(_) => {
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
