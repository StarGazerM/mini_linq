//! G3 reference: a direct interpreter of the abstract model.
//!
//! It never parses printed MiniLinq or SQL and uses no compiler pass. Steps run
//! in written order over sets of partial valuations (Set semantics), using a
//! per-step hash lookup on bound columns. It covers every model feature:
//! opaque total Rust operations, generators, blocks, negation and folds.

use super::model::*;
use crate::values::{TypedRows, Value};
use std::collections::{BTreeSet, HashMap};

/// A partial valuation indexed by variable.
type Env = Vec<Option<Value>>;

impl Case {
    /// The typed answer set.
    pub fn reference(&self) -> TypedRows {
        let query = &self.query;
        let width = query.variable_count();
        let tables = self
            .data
            .iter()
            .map(|rows| rows.iter().cloned().collect::<BTreeSet<_>>())
            .collect::<Vec<_>>();
        let evaluator = Evaluator { query, tables };
        let (envs, _) = evaluator.steps(
            &query.body_order(),
            BTreeSet::from([vec![None; width]]),
            BTreeSet::new(),
        );
        if query.folds.is_empty() {
            return envs
                .iter()
                .map(|env| query.head.iter().map(|e| head_value(e, env)).collect())
                .collect();
        }
        let mut groups: std::collections::BTreeMap<Vec<Value>, Vec<&Env>> = Default::default();
        for env in &envs {
            let key = query.head.iter().map(|e| head_value(e, env)).collect();
            groups.entry(key).or_default().push(env);
        }
        groups
            .into_iter()
            .map(|(mut row, members)| {
                for fold in &query.folds {
                    row.push(fold_value(fold, &members));
                }
                row
            })
            .collect()
    }
}

impl Query {
    /// One more than the largest variable index anywhere in the query.
    pub fn variable_count(&self) -> usize {
        let mut variables = BTreeSet::new();
        for atom in self.atoms.iter().chain(&self.negations) {
            for term in &atom.terms {
                match term {
                    Term::Var(v) => {
                        variables.insert(*v);
                    }
                    Term::Expression(e) => e.variables(&mut variables),
                    Term::Constant(_) | Term::Wildcard => (),
                }
            }
        }
        for binding in &self.bindings {
            variables.insert(binding.variable);
            binding.expression.variables(&mut variables);
        }
        for expression in &self.head {
            expression.variables(&mut variables);
        }
        variables.last().map_or(0, |v| v + 1)
    }
}

struct Evaluator<'a> {
    query: &'a Query,
    tables: Vec<BTreeSet<Vec<Value>>>,
}

impl Evaluator<'_> {
    fn steps(
        &self,
        steps: &[Step],
        mut envs: BTreeSet<Env>,
        mut bound: BTreeSet<usize>,
    ) -> (BTreeSet<Env>, BTreeSet<usize>) {
        for step in steps {
            if envs.is_empty() {
                // Headings still advance so block projections stay correct.
                bound = self.bound_after(*step, bound);
                continue;
            }
            match *step {
                Step::Atom(index) => {
                    envs = self.atom(&self.query.atoms[index], &envs, &bound);
                    bound = self.bound_after(*step, bound);
                }
                Step::Negation(index) => {
                    let atom = &self.query.negations[index];
                    // The negated pattern's non-wildcard positions, indexed.
                    let positions = atom
                        .terms
                        .iter()
                        .enumerate()
                        .filter(|(_, term)| !matches!(term, Term::Wildcard))
                        .map(|(column, _)| column)
                        .collect::<Vec<_>>();
                    let present = self.tables[atom.relation]
                        .iter()
                        .map(|row| positions.iter().map(|c| &row[*c]).collect::<Vec<_>>())
                        .collect::<std::collections::HashSet<_>>();
                    envs.retain(|env| {
                        let probe = positions
                            .iter()
                            .map(|c| match &atom.terms[*c] {
                                Term::Var(v) => env[*v].as_ref().expect("bound negated variable"),
                                Term::Constant(value) => value,
                                _ => unreachable!("negated atoms"),
                            })
                            .collect::<Vec<_>>();
                        !present.contains(&probe)
                    });
                }
                Step::Binding(index) => {
                    let binding = &self.query.bindings[index];
                    envs = envs
                        .into_iter()
                        .flat_map(|env| {
                            let value = expression(&binding.expression, &env);
                            let values = match binding.generator {
                                None => vec![value],
                                Some(generator) => generator.values(value),
                            };
                            values.into_iter().map(move |value| {
                                let mut env = env.clone();
                                env[binding.variable] = Some(Value::I32(value));
                                env
                            })
                        })
                        .collect();
                    bound.insert(binding.variable);
                }
                Step::Guard(index) => {
                    let guard = &self.query.guards[index];
                    envs.retain(|env| guard_value(guard, env));
                }
                Step::Constraint(index) => {
                    let constraint = &self.query.constraints[index];
                    envs.retain(|env| relational_holds(constraint, env));
                }
                Step::Block(index) => {
                    let block = &self.query.blocks[index];
                    let mut keep = bound.clone();
                    keep.extend(&block.interface);
                    let mut result = BTreeSet::new();
                    for env in &envs {
                        for branch in &block.branches {
                            let (answers, _) =
                                self.steps(branch, BTreeSet::from([env.clone()]), bound.clone());
                            let nonempty = !answers.is_empty();
                            result.extend(answers.into_iter().map(|mut answer| {
                                for (v, slot) in answer.iter_mut().enumerate() {
                                    if !keep.contains(&v) {
                                        *slot = None;
                                    }
                                }
                                answer
                            }));
                            if block.kind == BlockKind::Fallback && nonempty {
                                break;
                            }
                        }
                    }
                    envs = result;
                    bound = keep;
                }
            }
        }
        (envs, bound)
    }

    fn bound_after(&self, step: Step, mut bound: BTreeSet<usize>) -> BTreeSet<usize> {
        match step {
            Step::Atom(index) => {
                for term in &self.query.atoms[index].terms {
                    if let Term::Var(v) = term {
                        bound.insert(*v);
                    }
                }
            }
            Step::Binding(index) => {
                bound.insert(self.query.bindings[index].variable);
            }
            Step::Block(index) => bound.extend(&self.query.blocks[index].interface),
            Step::Negation(_) | Step::Guard(_) | Step::Constraint(_) => (),
        }
        bound
    }

    /// Join with one atom: index rows on the bound and constant positions.
    fn atom(&self, atom: &Atom, envs: &BTreeSet<Env>, bound: &BTreeSet<usize>) -> BTreeSet<Env> {
        let probe = atom
            .terms
            .iter()
            .enumerate()
            .filter(|(_, term)| match term {
                Term::Var(v) => bound.contains(v),
                Term::Constant(_) => true,
                Term::Wildcard | Term::Expression(_) => false,
            })
            .map(|(column, _)| column)
            .collect::<Vec<_>>();
        let mut index: HashMap<Vec<&Value>, Vec<&Vec<Value>>> = HashMap::new();
        for row in &self.tables[atom.relation] {
            index
                .entry(probe.iter().map(|c| &row[*c]).collect())
                .or_default()
                .push(row);
        }
        let mut output = BTreeSet::new();
        for env in envs {
            let key = probe
                .iter()
                .map(|c| match &atom.terms[*c] {
                    Term::Var(v) => env[*v].as_ref().expect("bound variable"),
                    Term::Constant(value) => value,
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>();
            let Some(rows) = index.get(&key) else {
                continue;
            };
            'rows: for row in rows {
                let mut extended = env.clone();
                for (term, value) in atom.terms.iter().zip(row.iter()) {
                    if let Term::Var(v) = term {
                        match &extended[*v] {
                            Some(previous) if previous != value => continue 'rows,
                            Some(_) => (),
                            None => extended[*v] = Some(value.clone()),
                        }
                    }
                }
                for (term, value) in atom.terms.iter().zip(row.iter()) {
                    if let Term::Expression(e) = term {
                        if Value::I32(expression(e, &extended)) != *value {
                            continue 'rows;
                        }
                    }
                }
                output.insert(extended);
            }
        }
        output
    }
}

fn head_value(expr: &Expr, env: &Env) -> Value {
    match expr {
        Expr::Var(v) => env[*v].clone().expect("bound head variable"),
        other => Value::I32(expression(other, env)),
    }
}

fn fold_value(fold: &Fold, members: &[&Env]) -> Value {
    let argument = |env: &Env| match &fold.argument {
        Some(FoldArg::Var(v)) => env[*v].clone().expect("bound fold argument"),
        Some(FoldArg::Parameter(p)) => p.clone(),
        None => unreachable!("count has no argument"),
    };
    match fold.kind {
        FoldKind::Count => Value::Usize(members.len() as u64),
        FoldKind::Min => members.iter().map(|env| argument(env)).min().unwrap(),
        FoldKind::Max => members.iter().map(|env| argument(env)).max().unwrap(),
        FoldKind::Sum => {
            let sample = argument(members[0]);
            let total: i128 = members
                .iter()
                .map(|env| argument(env).as_i128().expect("integer sum"))
                .sum();
            fn fit<T: TryFrom<i128>>(total: i128) -> T {
                T::try_from(total)
                    .unwrap_or_else(|_| panic!("sum {total} overflows: outside the total premise"))
            }
            match sample {
                Value::I32(_) => Value::I32(fit(total)),
                Value::I64(_) => Value::I64(fit(total)),
                Value::U32(_) => Value::U32(fit(total)),
                Value::Usize(_) => Value::Usize(fit(total)),
                Value::Str(_) => unreachable!("string sum"),
            }
        }
    }
}

/// i32 evaluation of the printed Rust; overflow is outside the total premise.
fn expression(expr: &Expr, env: &Env) -> i32 {
    checked_expression(expr, env)
        .unwrap_or_else(|| panic!("i32 overflow in {expr:?}: outside the total premise"))
}

/// `None` on i32 overflow (or unbound variables).
pub(super) fn checked_expression(expr: &Expr, env: &[Option<Value>]) -> Option<i32> {
    Some(match expr {
        Expr::Var(v) => match env[*v].as_ref()? {
            Value::I32(v) => *v,
            _ => return None,
        },
        Expr::Constant(c) | Expr::Parameter(c) => *c,
        Expr::Negate(inner) => checked_expression(inner, env)?.checked_neg()?,
        Expr::OpaqueCall(inner) => checked_expression(inner, env)?,
        Expr::Binary(op, left, right) => {
            let (l, r) = (
                checked_expression(left, env)?,
                checked_expression(right, env)?,
            );
            match op {
                Arithmetic::Add => l.checked_add(r)?,
                Arithmetic::Subtract => l.checked_sub(r)?,
                Arithmetic::Multiply => l.checked_mul(r)?,
                Arithmetic::Remainder => l.checked_rem(r)?,
            }
        }
        Expr::Rust(op, inner) => {
            let v = checked_expression(inner, env)?;
            match op {
                RustOp::Divide => 10 / ((v % 3) + 4),
                RustOp::Remainder => 10 % ((v % 3) + 4),
                RustOp::Add => (i32::MAX - 3) + (v % 4),
                RustOp::Subtract => (i32::MIN + 3) - (v % 4),
                RustOp::Multiply => (i32::MAX / 4) * (v % 4),
                RustOp::Unwrap => v,
            }
        }
    })
}

fn guard_value(guard: &Guard, env: &Env) -> bool {
    match guard {
        Guard::Compare(op, l, r) => op.holds(&expression(l, env), &expression(r, env)),
        Guard::And(l, r) => guard_value(l, env) && guard_value(r, env),
        Guard::Or(l, r) => guard_value(l, env) || guard_value(r, env),
        Guard::Not(inner) => !guard_value(inner, env),
    }
}

pub(super) fn relational_holds(c: &Relational, env: &[Option<Value>]) -> bool {
    let term = |t: &RelTerm| match t {
        RelTerm::Var(v) => env[*v].clone().expect("bound constraint variable"),
        RelTerm::Literal(v) | RelTerm::Parameter(v) => v.clone(),
    };
    match c {
        Relational::Compare(op, l, r) => op.holds(&term(l), &term(r)),
        Relational::Between(t, l, h) => {
            let t = term(t);
            term(l) <= t && t <= term(h)
        }
        Relational::In(t, values) => values.contains(&term(t)),
        Relational::Or(branches) => branches
            .iter()
            .any(|branch| branch.iter().all(|c| relational_holds(c, env))),
    }
}
