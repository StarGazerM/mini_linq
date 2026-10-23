//! Independent active-domain valuation enumeration against original CQ tables.
//!
//! Assumes contract-valid positive/negative atoms with variables and integer literals,
//! finite declared-width integer tables, Set semantics, and relational comparison guards.
//! Exhaustive enumeration is exponential in the number of query variables.
//! No compiler normalizer, planner, or pass is used. Unsupported clauses panic.

use crate::{
    cq,
    values::{
        Binding, Database, Relation, Rows, evaluate_constraint, evaluate_equality,
        evaluate_integer, evaluate_predicate,
    },
};
use cq_syntax::symbol_name;
use std::collections::BTreeSet;
/// Assert that every input table exists and has the declared tuple width.
pub fn validate_database(program: &cq::Program, database: &Database) {
    for input in program.inputs() {
        let name = symbol_name(&input.name);
        let rows = database
            .get(&name)
            .unwrap_or_else(|| panic!("missing table {name}"));
        assert!(rows.iter().all(|row| row.len() == input.columns.len()));
    }
}

/// Enumerate complete assignments over the active domain, then check each
/// ordered atom tuple directly in its original input relation.
pub fn valuations(program: &cq::Program, database: &Database) -> Relation {
    validate_database(program, database);
    body(
        &program.query.body,
        database,
        Relation {
            heading: BTreeSet::new(),
            rows: BTreeSet::from([Binding::new()]),
        },
    )
}

/// Written nested-loop reference over the original clauses, without compiler
/// normalization, planning, or heading analysis.
pub fn body(
    items: &cq_syntax::CommaList<cq::BodyItem>,
    database: &Database,
    mut prefix: Relation,
) -> Relation {
    for item in items {
        match item {
            cq::BodyItem::Positive { atom } => {
                let heading: BTreeSet<_> = atom.variables().map(symbol_name).collect();
                let table = Relation {
                    heading,
                    rows: database[&symbol_name(&atom.relation)]
                        .iter()
                        .filter_map(|row| {
                            let mut binding = Binding::new();
                            for (argument, value) in atom.args.iter().zip(row) {
                                if let Some(variable) = cq::rust::variable(argument) {
                                    if binding
                                        .insert(symbol_name(variable), *value)
                                        .is_some_and(|previous| previous != *value)
                                    {
                                        return None;
                                    }
                                } else if evaluate_integer(argument, &Binding::new()) != *value {
                                    return None;
                                }
                            }
                            Some(binding)
                        })
                        .collect(),
                };
                prefix = prefix.join(&table);
            }
            cq::BodyItem::Negation(negative) => prefix.rows.retain(|binding| {
                !database[&symbol_name(&negative.atom.relation)]
                    .iter()
                    .any(|row| {
                        negative.atom.args.iter().zip(row).all(|(argument, value)| {
                            matches!(cq::rust::unparen(argument), syn::Expr::Infer(_))
                                || evaluate_integer(argument, binding) == *value
                        })
                    })
            }),
            cq::BodyItem::Equality(e) => prefix.rows.retain(|row| evaluate_equality(e, row)),
            cq::BodyItem::Constraint(c) => prefix.rows.retain(|row| evaluate_constraint(c, row)),
            cq::BodyItem::Filter(f) => prefix
                .rows
                .retain(|row| evaluate_predicate(&f.condition, row)),
            cq::BodyItem::Block(block) => {
                let mut result = Relation {
                    heading: prefix.heading.clone(),
                    rows: BTreeSet::new(),
                };
                let mut common: Option<BTreeSet<String>> = None;
                // Headings are obtained even on an empty context.
                for branch in &block.branches {
                    let relation = body(
                        &branch.body,
                        database,
                        Relation {
                            heading: prefix.heading.clone(),
                            rows: BTreeSet::new(),
                        },
                    );
                    common = Some(match common {
                        None => relation.heading,
                        Some(names) => names.intersection(&relation.heading).cloned().collect(),
                    });
                }
                result.heading.extend(common.unwrap_or_default());
                for row in &prefix.rows {
                    let context = Relation {
                        heading: prefix.heading.clone(),
                        rows: BTreeSet::from([row.clone()]),
                    };
                    for branch in &block.branches {
                        let branch_result = body(&branch.body, database, context.clone())
                            .project(result.heading.clone());
                        let nonempty = !branch_result.rows.is_empty();
                        result.rows.extend(branch_result.rows);
                        if block.kind == cq::blocks::Kind::Fallback && nonempty {
                            break;
                        }
                    }
                }
                prefix = result;
            }
            cq::BodyItem::Let(_) | cq::BodyItem::For(_) => {
                panic!("computed columns and generators use compiled written-order Rust")
            }
            _ => panic!("integer oracle excludes computed bindings and extensions"),
        }
    }
    prefix
}

/// Enumerate satisfying valuations and project in authored output order.
pub fn evaluate_cq(program: &cq::Program, database: &Database) -> Rows {
    valuations(program, database).output(
        &program
            .query
            .head
            .variables()
            .map(symbol_name)
            .collect::<Vec<_>>(),
    )
}
