//! Independent SQL rendering of recursive abstract blocks. Each branch reads
//! the same prefix CTE. Union uses UNION; fallback uses UNION ALL and NOT EXISTS
//! against the original earlier branch CTEs on the prefix columns.

use super::model::{BlockKind, Case, Query, Step, Term, sql_constraint, sql_expression, sql_guard};
use std::collections::{BTreeMap, BTreeSet};

struct Printer<'a> {
    query: &'a Query,
    ctes: Vec<String>,
}
impl Printer<'_> {
    fn define(&mut self, sql: String) -> String {
        let name = format!("q{}", self.ctes.len());
        self.ctes.push(format!("{name} AS ({sql})"));
        name
    }
    fn steps(
        &mut self,
        steps: &[Step],
        mut input: String,
        mut bound: BTreeSet<usize>,
    ) -> (String, BTreeSet<usize>) {
        for step in steps {
            let env: BTreeMap<_, _> = bound.iter().map(|v| (*v, format!("p.v{v}"))).collect();
            match *step {
                Step::Atom(index) => {
                    let atom = &self.query.atoms[index];
                    let mut columns = vec!["p.*".into()];
                    let mut conditions = Vec::new();
                    let mut names = env;
                    for (column, term) in atom.terms.iter().enumerate() {
                        let field = format!("a.c{column}");
                        match term {
                            Term::Var(v) => {
                                if let Some(old) = names.get(v) {
                                    conditions.push(format!("{field} = {old}"));
                                } else {
                                    columns.push(format!("{field} AS v{v}"));
                                    names.insert(*v, field);
                                    bound.insert(*v);
                                }
                            }
                            Term::Constant(v) => conditions.push(format!("{field} = {v}")),
                            Term::Wildcard => (),
                            Term::Expression(_) => (),
                        }
                    }
                    // Resolve local expression dependencies after all variable
                    // positions, including variables written to their right.
                    for (column, term) in atom.terms.iter().enumerate() {
                        if let Term::Expression(expression) = term {
                            conditions.push(format!(
                                "a.c{column} = {}",
                                sql_expression(expression, &names)
                            ));
                        }
                    }
                    let condition = if conditions.is_empty() {
                        String::new()
                    } else {
                        format!(" WHERE {}", conditions.join(" AND "))
                    };
                    input = self.define(format!(
                        "SELECT {} FROM {input} p CROSS JOIN \"mini-linq/raw/{}\" a{condition}",
                        columns.join(", "),
                        atom.relation
                    ));
                }
                Step::Binding(index) => {
                    let binding = &self.query.bindings[index];
                    input = self.define(format!(
                        "SELECT p.*, {} AS v{} FROM {input} p",
                        sql_expression(&binding.expression, &env),
                        binding.variable
                    ));
                    bound.insert(binding.variable);
                }
                Step::Guard(index) => {
                    input = self.define(format!(
                        "SELECT p.* FROM {input} p WHERE {}",
                        sql_guard(&self.query.guards[index], &env)
                    ))
                }
                Step::Constraint(index) => {
                    input = self.define(format!(
                        "SELECT p.* FROM {input} p WHERE {}",
                        sql_constraint(&self.query.constraints[index], &env)
                    ))
                }
                Step::Block(index) => {
                    let block = &self.query.blocks[index];
                    let kind = block.kind;
                    let branches = block.branches.clone();
                    let mut common = bound.clone();
                    common.extend(&block.interface);
                    let projection = std::iter::once("p.__unit".into())
                        .chain(common.iter().map(|v| format!("p.v{v}")))
                        .collect::<Vec<String>>()
                        .join(", ");
                    let mut originals = Vec::new();
                    let mut operands = Vec::new();
                    for branch in &branches {
                        let (raw, _) = self.steps(branch, input.clone(), bound.clone());
                        let mut select = format!("SELECT {projection} FROM {raw} p");
                        if kind == BlockKind::Fallback && !originals.is_empty() {
                            let predicates = originals
                                .iter()
                                .map(|previous| {
                                    let matches = if bound.is_empty() {
                                        "TRUE".into()
                                    } else {
                                        bound
                                            .iter()
                                            .map(|v| format!("n.v{v} = p.v{v}"))
                                            .collect::<Vec<_>>()
                                            .join(" AND ")
                                    };
                                    format!(
                                        "NOT EXISTS (SELECT 1 FROM {previous} n WHERE {matches})"
                                    )
                                })
                                .collect::<Vec<_>>();
                            select.push_str(&format!(" WHERE {}", predicates.join(" AND ")));
                        }
                        originals.push(raw);
                        operands.push(select);
                    }
                    input = self.define(operands.join(if kind == BlockKind::Union {
                        " UNION "
                    } else {
                        " UNION ALL "
                    }));
                    bound = common;
                }
            }
        }
        (input, bound)
    }
}
impl Case {
    pub(super) fn block_sql(&self) -> String {
        let mut printer = Printer {
            query: &self.query,
            ctes: Vec::new(),
        };
        let unit = printer.define("SELECT 1 AS __unit".into());
        let (result, bound) = printer.steps(&self.query.body_order(), unit, BTreeSet::new());
        let env = bound.iter().map(|v| (*v, format!("p.v{v}"))).collect();
        let projection = self
            .query
            .head
            .iter()
            .enumerate()
            .map(|(i, e)| format!("CAST({} AS INTEGER) AS out{i}", sql_expression(e, &env)))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "WITH {} SELECT DISTINCT {projection} FROM {result} p ORDER BY {};\n",
            printer.ctes.join(",\n"),
            (1..=self.output_arity())
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}
