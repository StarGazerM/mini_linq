//! Deliberately small, test-only semantics. CQ evaluates valuations against
//! base tables. RA separately interprets the parsed operator DAG using nested
//! loops. Neither evaluator calls compiler planning, heading, or key helpers.

use std::collections::{BTreeMap, BTreeSet};

use cq_ir::{cq, relational_plan, symbol_name};

pub type Rows = BTreeSet<Vec<i32>>;
pub type Database = BTreeMap<String, Rows>;
pub type Binding = BTreeMap<String, i32>;

/// Independent small-query oracle for future GYO tests: enumerate trees and
/// check running intersection directly, without any hypergraph reductions.
pub fn has_join_tree(program: &cq::Program) -> bool {
    let headings = program
        .query
        .body
        .iter()
        .filter_map(cq::BodyItem::atom)
        .map(|atom| atom.variables().map(symbol_name).collect::<BTreeSet<_>>())
        .collect::<Vec<_>>();
    has_join_tree_for(&headings)
}

pub fn has_join_tree_for(headings: &[BTreeSet<String>]) -> bool {
    let n = headings.len();
    assert!(
        (1..=7).contains(&n),
        "bounded oracle supports one to seven occurrences"
    );
    if n == 1 {
        return true;
    }
    // Prüfer codes enumerate exactly the n^(n-2) labeled trees. At seven
    // occurrences this checks 16,807 trees instead of 2^21 edge subsets.
    let variables = headings
        .iter()
        .flat_map(|h| h.iter())
        .collect::<BTreeSet<_>>();
    for mut code in 0..n.pow((n - 2) as u32) {
        let mut sequence = Vec::new();
        let mut degree = vec![1; n];
        for _ in 0..n - 2 {
            let vertex = code % n;
            sequence.push(vertex);
            degree[vertex] += 1;
            code /= n;
        }
        let mut adjacent = vec![BTreeSet::new(); n];
        for vertex in sequence {
            let leaf = degree.iter().position(|d| *d == 1).unwrap();
            adjacent[leaf].insert(vertex);
            adjacent[vertex].insert(leaf);
            degree[leaf] -= 1;
            degree[vertex] -= 1;
        }
        let leaves = (0..n).filter(|i| degree[*i] == 1).collect::<Vec<_>>();
        adjacent[leaves[0]].insert(leaves[1]);
        adjacent[leaves[1]].insert(leaves[0]);
        if variables.iter().all(|x| {
            let containing = (0..n).filter(|i| headings[*i].contains(*x)).collect();
            connected(&containing, &adjacent)
        }) {
            return true;
        }
    }
    false
}

fn connected(nodes: &BTreeSet<usize>, adjacent: &[BTreeSet<usize>]) -> bool {
    let mut pending = vec![*nodes.first().unwrap()];
    let mut visited = BTreeSet::new();
    while let Some(i) = pending.pop() {
        if visited.insert(i) {
            pending.extend(adjacent[i].intersection(nodes));
        }
    }
    visited == *nodes
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub heading: BTreeSet<String>,
    pub rows: BTreeSet<Binding>,
}

impl Relation {
    pub fn project(&self, heading: BTreeSet<String>) -> Self {
        assert!(heading.is_subset(&self.heading));
        Self {
            rows: self
                .rows
                .iter()
                .map(|row| heading.iter().map(|x| (x.clone(), row[x])).collect())
                .collect(),
            heading,
        }
    }

    pub fn join(&self, right: &Self) -> Self {
        let shared = self
            .heading
            .intersection(&right.heading)
            .collect::<Vec<_>>();
        let mut rows = BTreeSet::new();
        for left_row in &self.rows {
            for right_row in &right.rows {
                if shared.iter().all(|x| left_row[*x] == right_row[*x]) {
                    let mut row = left_row.clone();
                    row.extend(right_row.iter().map(|(x, v)| (x.clone(), *v)));
                    rows.insert(row);
                }
            }
        }
        Self {
            heading: self.heading.union(&right.heading).cloned().collect(),
            rows,
        }
    }

    pub fn output(&self, variables: &[String]) -> Rows {
        variables
            .iter()
            .for_each(|x| assert!(self.heading.contains(x)));
        self.rows
            .iter()
            .map(|row| variables.iter().map(|x| row[x]).collect())
            .collect()
    }
}

pub fn validate_database(program: &cq::Program, database: &Database) {
    for input in &program.inputs {
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
            cq::BodyItem::Filter(_) => None,
            cq::BodyItem::Let(_) => {
                panic!("integer valuation oracle domain excludes Rust let clauses")
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

pub struct Evaluation {
    pub definitions: BTreeMap<String, Relation>,
    pub output: Rows,
}

pub fn evaluate_ra(module: &relational_plan::Module, database: &Database) -> Evaluation {
    validate_database(&module.program, database);
    evaluate_plan(&module.program.inputs, &module.plan, database)
}

pub fn evaluate_plan(
    inputs: &[cq::RelationDecl],
    plan: &relational_plan::Plan,
    database: &Database,
) -> Evaluation {
    let mut definitions = BTreeMap::<String, Relation>::new();
    for definition in &plan.definitions {
        let relation = match &definition.operator {
            relational_plan::Operator::Unit(_) => Relation {
                heading: BTreeSet::new(),
                rows: BTreeSet::from([Binding::new()]),
            },
            relational_plan::Operator::Rename(rename) => {
                let input = inputs
                    .iter()
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
            relational_plan::Operator::NaturalJoin(join) => {
                definitions[&symbol_name(&join.left)].join(&definitions[&symbol_name(&join.right)])
            }
            relational_plan::Operator::Semijoin(join) => {
                let left = &definitions[&symbol_name(&join.left)];
                left.join(&definitions[&symbol_name(&join.right)])
                    .project(left.heading.clone())
            }
            relational_plan::Operator::Select(select) => {
                let mut input = definitions[&symbol_name(&select.input)].clone();
                input
                    .rows
                    .retain(|row| evaluate_predicate(&select.condition, row));
                input
            }
            relational_plan::Operator::Extend(_) => {
                panic!("Rust computations are tested by compiled end-to-end tests")
            }
            relational_plan::Operator::Project(project) => definitions
                [&symbol_name(&project.input)]
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

pub fn evaluate_predicate(expr: &syn::Expr, row: &Binding) -> bool {
    use quote::ToTokens;
    let mut expr = expr;
    while let syn::Expr::Paren(p) = expr {
        expr = &p.expr;
    }
    let predicate: cq::Predicate =
        syn::parse2(expr.to_token_stream()).expect("integer comparison oracle domain");
    use cq::{Comparison, Term};
    let value = |term: &Term| match term {
        Term::Variable(v) => row[&symbol_name(v)],
        Term::Integer(i) => *i,
    };
    let l = value(&predicate.left);
    let r = value(&predicate.right);
    match predicate.comparison {
        Comparison::Eq => l == r,
        Comparison::Ne => l != r,
        Comparison::Lt => l < r,
        Comparison::Le => l <= r,
        Comparison::Gt => l > r,
        Comparison::Ge => l >= r,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relation(heading: &[&str], rows: &[&[i32]]) -> Relation {
        Relation {
            heading: heading.iter().map(|x| (*x).to_owned()).collect(),
            rows: rows
                .iter()
                .map(|row| {
                    heading
                        .iter()
                        .zip(*row)
                        .map(|(x, v)| ((*x).to_owned(), *v))
                        .collect()
                })
                .collect(),
        }
    }

    #[test]
    fn join_requires_one_witness_for_the_entire_composite_key() {
        let left = relation(&["a", "b"], &[&[1, 2]]);
        let right = relation(&["a", "b", "c"], &[&[1, 9, 3], &[8, 2, 4]]);
        assert!(left.join(&right).rows.is_empty());
    }

    #[test]
    fn empty_heading_distinguishes_existence_from_empty_input() {
        let nonempty = relation(&["x"], &[&[1], &[2]]).project(BTreeSet::new());
        let empty = relation(&["x"], &[]).project(BTreeSet::new());
        let left = relation(&["a"], &[&[3]]);
        assert_eq!(nonempty.rows, BTreeSet::from([Binding::new()]));
        assert_eq!(left.join(&nonempty), left);
        assert!(left.join(&empty).rows.is_empty());
    }

    #[test]
    fn projection_collapses_witnesses_and_output_obeys_head_order() {
        let source = relation(&["x", "y", "z"], &[&[1, 2, 3], &[1, 2, 4]]);
        let result = source.project(BTreeSet::from(["x".into(), "y".into()]));
        assert_eq!(
            result.output(&["y".into(), "x".into()]),
            Rows::from([vec![2, 1]])
        );
    }

    #[test]
    fn tree_oracle_handles_degenerate_duplicate_and_seven_node_cases() {
        let h = |xs: &[&str]| xs.iter().map(|x| (*x).to_owned()).collect::<BTreeSet<_>>();
        assert!(has_join_tree_for(&[h(&["a", "b"])]));
        assert!(has_join_tree_for(&[h(&["a", "b"]), h(&["a", "b"])]));
        assert!(has_join_tree_for(&[h(&["a"]), h(&["b"])]));
        let path = (0..7)
            .map(|i| BTreeSet::from([format!("x{i}"), format!("x{}", i + 1)]))
            .collect::<Vec<_>>();
        assert!(has_join_tree_for(&path));
        let cycle = (0..7)
            .map(|i| BTreeSet::from([format!("x{i}"), format!("x{}", (i + 1) % 7)]))
            .collect::<Vec<_>>();
        assert!(!has_join_tree_for(&cycle));
    }
}
