//! Interpret subatoms and reductions independently of compilation.
use crate::{valuation as vo, values::*};
use std::collections::{BTreeMap, BTreeSet};
use vo::{cq, syntax::symbol_name};
pub fn evaluate(program: &vo::Program, database: &Database) -> Rows {
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
                        left.join(&atoms[&support.clause()])
                            .project(left.heading.clone())
                    }
                    vo::Reduction::Guard(g) => {
                        let mut left = atoms[&r.clause()].clone();
                        left.rows
                            .retain(|row| evaluate_predicate(&g.condition, row));
                        left
                    }
                };
                atoms.insert(r.clause(), relation);
            }
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
