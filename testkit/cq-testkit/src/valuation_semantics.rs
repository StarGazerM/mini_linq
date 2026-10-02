//! Interpret VO subatoms, schedule steps, and reductions without compilation.
//!
//! Assumes contract-valid variable-only atoms with distinct variables within
//! each base atom, finite declared-width integer tables, Set semantics, Leibniz
//! equality, and total comparison guards. Computed columns are unsupported.
use crate::{values::*, vo};
use std::collections::{BTreeMap, BTreeSet};
use vo::{cq, syntax::symbol_name};
/// Execute the authored valuation schedule and return its ordered answer set.
pub fn evaluate(program: &vo::Program, database: &Database) -> Rows {
    crate::cq_semantics::validate_database(&program.cq, database);
    let mut atoms: BTreeMap<usize, Relation> = program
        .cq
        .query
        .body
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let cq::BodyItem::Positive { atom } = item else {
                return None;
            };
            let columns: Vec<_> = atom.variables().map(symbol_name).collect();
            Some((
                i,
                Relation {
                    heading: columns.iter().cloned().collect(),
                    rows: database[&symbol_name(&atom.relation)]
                        .iter()
                        .map(|row| columns.iter().cloned().zip(row.iter().copied()).collect())
                        .collect(),
                },
            ))
        })
        .collect();
    let mut result = Relation {
        heading: BTreeSet::new(),
        rows: BTreeSet::from([Binding::new()]),
    };
    for step in &program.valuation.steps {
        match step {
            vo::Step::Atom(a, _) => {
                result = result.join(
                    &atoms[&a.clause()]
                        .project(a.variables().map(|(_, v)| symbol_name(v)).collect()),
                )
            }
            vo::Step::Guard(g, _) => result
                .rows
                .retain(|row| evaluate_predicate(&g.condition, row)),
            vo::Step::Reduce(r, _) => {
                let relation = match &r.reduction {
                    vo::Reduction::With(_, support) => {
                        let left = &atoms[&r.clause()];
                        let mut reduced = left.clone();
                        let same = |a: &str, b: &str| {
                            let mut reached = BTreeSet::from([a.to_owned()]);
                            loop {
                                let previous = reached.len();
                                for item in &program.cq.query.body {
                                    if let cq::BodyItem::Equality(e) = item {
                                        if let cq::EqualityValue::Variable(v) = &e.right {
                                            let (l, r) = (symbol_name(&e.left), symbol_name(v));
                                            if reached.contains(&l) || reached.contains(&r) {
                                                reached.insert(l);
                                                reached.insert(r);
                                            }
                                        }
                                    }
                                }
                                if reached.len() == previous {
                                    return reached.contains(b);
                                }
                            }
                        };
                        reduced.rows.retain(|row| {
                            atoms[&support.clause()].rows.iter().any(|witness| {
                                support.variables().all(|(_, v)| {
                                    let v = symbol_name(v);
                                    row.iter()
                                        .filter(|(c, _)| same(c, &v))
                                        .all(|(_, value)| *value == witness[&v])
                                })
                            })
                        });
                        reduced
                    }
                };
                atoms.insert(r.clause(), relation);
            }
            vo::Step::Equality(e, _) => result.rows.retain(|row| evaluate_equality(e, row)),
            vo::Step::Constraint(c, _) => result.rows.retain(|row| evaluate_constraint(c, row)),
            vo::Step::Let(..) => panic!("computed columns are checked by generated Rust execution"),
        }
    }
    result.output(
        &program
            .cq
            .query
            .head
            .variables()
            .map(symbol_name)
            .collect::<Vec<_>>(),
    )
}
