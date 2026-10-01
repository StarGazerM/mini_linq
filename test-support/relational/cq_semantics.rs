use crate::{
    cq,
    values::{Binding, Database, Relation, Rows, evaluate_equality, evaluate_predicate},
};
use cq_syntax::symbol_name;
use std::collections::BTreeSet;
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
    let relations = database;
    let atoms = program
        .query
        .body
        .iter()
        .filter_map(|item| match item {
            cq::BodyItem::Positive { atom } => Some((
                symbol_name(&atom.relation),
                atom.variables().map(symbol_name).collect::<Vec<_>>(),
            )),
            cq::BodyItem::Filter(_) | cq::BodyItem::Equality(_) => None,
            cq::BodyItem::Let(_)
            | cq::BodyItem::Negation(_)
            | cq::BodyItem::Aggregate(_)
            | cq::BodyItem::Extension { .. } => {
                panic!("integer valuation oracle domain excludes let, negation, and aggregation")
            }
        })
        .collect::<Vec<_>>();
    let predicates = program
        .query
        .body
        .iter()
        .filter_map(|item| match item {
            cq::BodyItem::Filter(f) => Some(&f.condition),
            _ => None,
        })
        .collect::<Vec<_>>();
    let heading = atoms
        .iter()
        .flat_map(|(_, xs)| xs.iter().cloned())
        .collect::<BTreeSet<_>>();
    let variables = heading.iter().cloned().collect::<Vec<_>>();
    let domain = atoms
        .iter()
        .flat_map(|(r, _)| relations[r].iter().flatten().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut result = Relation {
        heading,
        rows: BTreeSet::new(),
    };
    enumerate(
        &variables,
        &domain,
        &mut Binding::new(),
        &mut |assignment| {
            if atoms.iter().all(|(r, xs)| {
                relations[r].contains(&xs.iter().map(|x| assignment[x]).collect::<Vec<_>>())
            }) && predicates.iter().all(|p| evaluate_predicate(p, assignment))
                && program.query.body.iter().all(|item| match item {
                    cq::BodyItem::Equality(e) => evaluate_equality(e, assignment),
                    _ => true,
                })
            {
                result.rows.insert(assignment.clone());
            }
        },
    );
    result
}

fn enumerate(
    remaining: &[String],
    domain: &[i32],
    assignment: &mut Binding,
    visit: &mut impl FnMut(&Binding),
) {
    let Some((x, tail)) = remaining.split_first() else {
        visit(assignment);
        return;
    };
    for value in domain {
        assignment.insert(x.clone(), *value);
        enumerate(tail, domain, assignment, visit);
    }
    assignment.remove(x);
}

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
