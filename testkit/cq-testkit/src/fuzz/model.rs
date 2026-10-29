//! Random query specifications, independent of MiniLinq's syntax and IR.
//!
//! The Rust and SQL printers each consume this model directly. Neither printer
//! calls the other, parses its output, or uses compiler normalization/planning.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub name: String,
    pub arity: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    Var(usize),
    Constant(i32),
    Wildcard,
    Expression(Expr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Atom {
    pub relation: usize,
    pub terms: Vec<Term>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arithmetic {
    Add,
    Subtract,
    Multiply,
    Remainder,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Var(usize),
    Constant(i32),
    Binary(Arithmetic, Box<Expr>, Box<Expr>),
    Negate(Box<Expr>),
    Rust(RustOp, Box<Expr>),
    OpaqueCall(Box<Expr>),
}

/// Opaque Rust operations authored to be total for every i32 input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RustOp {
    Divide,
    Remainder,
    Add,
    Subtract,
    Multiply,
    Unwrap,
}

impl RustOp {
    fn name(self) -> &'static str {
        match self {
            Self::Divide => "divide",
            Self::Remainder => "remainder",
            Self::Add => "bounded-add",
            Self::Subtract => "bounded-subtract",
            Self::Multiply => "bounded-multiply",
            Self::Unwrap => "unwrap",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Atom(usize),
    Binding(usize),
    Guard(usize),
    Constraint(usize),
    Block(usize),
}

/// Ordered fallback or unordered union of authored branch step sequences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Union,
    Fallback,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub kind: BlockKind,
    pub branches: Vec<Vec<Step>>,
    pub interface: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Guard {
    Compare(Comparison, Expr, Expr),
    And(Box<Guard>, Box<Guard>),
    Or(Box<Guard>, Box<Guard>),
    Not(Box<Guard>),
}

/// Relational operands are query columns, literals, or closed host captures.
/// Capture names are never query binders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelTerm {
    Var(usize),
    Literal(i32),
    Parameter(i32),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Relational {
    Compare(Comparison, RelTerm, RelTerm),
    Between(RelTerm, RelTerm, RelTerm),
    In(RelTerm, Vec<i32>),
    Or(Vec<Vec<Relational>>),
}
fn parameter_name(value: i32) -> String {
    format!(
        "parameter_{}{}",
        if value < 0 { "negative_" } else { "positive_" },
        value.unsigned_abs()
    )
}
impl Relational {
    fn terms(&self) -> Vec<&RelTerm> {
        match self {
            Self::Compare(_, l, r) => vec![l, r],
            Self::Between(t, l, h) => vec![t, l, h],
            Self::In(t, _) => vec![t],
            Self::Or(bs) => bs.iter().flatten().flat_map(Self::terms).collect(),
        }
    }
    fn variables(&self, output: &mut BTreeSet<usize>) {
        output.extend(self.terms().into_iter().filter_map(|t| match t {
            RelTerm::Var(v) => Some(*v),
            _ => None,
        }));
    }
    fn features(&self, output: &mut BTreeSet<String>) {
        output.insert("relational-constraint".into());
        if self
            .terms()
            .iter()
            .any(|t| matches!(t, RelTerm::Parameter(_)))
        {
            output.insert("relational-parameter".into());
        }
        match self {
            Self::Compare(op, _, _) => {
                output.insert(format!(
                    "relational-{}",
                    match op {
                        Comparison::Equal => "eq",
                        Comparison::NotEqual => "ne",
                        Comparison::Less => "lt",
                        Comparison::LessEqual => "le",
                        Comparison::Greater => "gt",
                        Comparison::GreaterEqual => "ge",
                    }
                ));
            }
            Self::Between(..) => {
                output.insert("relational-between".into());
            }
            Self::In(..) => {
                output.insert("relational-in".into());
            }
            Self::Or(bs) => {
                output.insert("relational-or".into());
                for c in bs.iter().flatten() {
                    c.features(output);
                }
            }
        }
    }
}

/// Finite, total Rust iterables use the compiled oracle because SQL does not
/// model arbitrary IntoIterator values. Range endpoints are bounded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Generator {
    Range,
    InclusiveRange,
    Duplicates,
    Empty,
    Mapped,
}

impl Generator {
    fn feature(self) -> &'static str {
        match self {
            Self::Range => "for-range",
            Self::InclusiveRange => "for-inclusive-range",
            Self::Duplicates => "for-duplicates",
            Self::Empty => "for-empty",
            Self::Mapped => "for-mapped",
        }
    }

    fn rust(self, expression: &Expr) -> String {
        let value = rust_expression(expression);
        match self {
            Self::Range => format!("(0i32..(({value}) % 4))"),
            Self::InclusiveRange => format!("(0i32..=(({value}) % 4))"),
            Self::Duplicates => format!("{{ let value: i32 = {value}; vec![value, value, 0i32] }}"),
            Self::Empty => format!("{{ let _: i32 = {value}; std::iter::empty::<i32>() }}"),
            Self::Mapped => format!("{{ let value: i32 = {value}; (0..2).map(move |_| value) }}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub variable: usize,
    pub expression: Expr,
    /// `None` is a scalar let; `Some` enumerates the finite iterable.
    pub generator: Option<Generator>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub schema: Vec<Relation>,
    pub atoms: Vec<Atom>,
    pub bindings: Vec<Binding>,
    pub guards: Vec<Guard>,
    pub constraints: Vec<Relational>,
    pub blocks: Vec<Block>,
    pub head: Vec<Expr>,
    /// Empty uses the total lane's atoms/bindings/guards order.
    pub order: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    pub seed: u64,
    pub query: Query,
    /// Schema order; raw duplicates are deliberately retained.
    pub data: Vec<Vec<Vec<i32>>>,
}

impl Case {
    // DESIGN contract7-rust-oracle
    // Owns oracle routing: opaque Rust/generators use the compiled written-order Rust reference.
    // Contract 7; source: doc/PLANNING-REDESIGN.md Contract and Plan P0; doc/formal/CONFORMANCE.md.
    // Both lanes assume total expressions; SQL covers only its independently modeled fragment.
    // DO NOT make SQL simulate arbitrary Rust, silently discard Rust cases, or weaken the total premise.
    /// Select the executable Rust oracle for operations and iterables not
    /// modeled by the independent SQL printer. Both lanes are total.
    pub fn requires_rust_oracle(&self) -> bool {
        if self
            .query
            .bindings
            .iter()
            .any(|binding| binding.generator.is_some())
        {
            return true;
        }
        let mut features = BTreeSet::new();
        for atom in &self.query.atoms {
            for term in &atom.terms {
                if let Term::Expression(expression) = term {
                    expression.features(&mut features);
                }
            }
        }
        for binding in &self.query.bindings {
            binding.expression.features(&mut features);
        }
        for guard in &self.query.guards {
            guard.features(&mut features);
        }
        for expression in &self.query.head {
            expression.features(&mut features);
        }
        features.contains("rust-term")
    }

    pub fn parameters(&self) -> BTreeMap<String, i32> {
        self.query
            .constraints
            .iter()
            .flat_map(Relational::terms)
            .filter_map(|t| match t {
                RelTerm::Parameter(v) => Some((parameter_name(*v), *v)),
                _ => None,
            })
            .collect()
    }

    pub fn output_arity(&self) -> usize {
        self.query.head.len()
    }

    /// Emit a full macro input, with explicitly typed computed output columns.
    pub fn cq(&self) -> String {
        self.print_cq()
    }

    /// The written planner consumes the same opaque Rust and explicit
    /// relational syntax as every optimized planner.
    pub fn written_cq(&self) -> String {
        self.print_cq()
    }

    fn print_cq(&self) -> String {
        let mut source = String::from("struct Fuzz;\n");
        for relation in &self.query.schema {
            writeln!(
                source,
                "relation {}({});",
                relation.name,
                (0..relation.arity)
                    .map(|column| format!("c{column}: i32"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .unwrap();
        }
        writeln!(
            source,
            "relation answer({});",
            (0..self.output_arity())
                .map(|column| format!("c{column}: i32"))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
        let clauses = self.query.cq_steps(&self.query.body_order());
        writeln!(
            source,
            "answer({}) :- {};",
            self.query
                .head
                .iter()
                .map(|expression| match expression {
                    Expr::Var(variable) => format!("v{variable}"),
                    _ => rust_expression(expression),
                })
                .collect::<Vec<_>>()
                .join(", "),
            clauses
        )
        .unwrap();
        source
    }

    /// Emit SQL directly from abstract atoms and expressions. Raw duplicates
    /// need no input deduplication: positive CQs with a DISTINCT output have
    /// set semantics, including self-joins and existential wildcard columns.
    pub fn sql(&self) -> String {
        if self.requires_rust_oracle() {
            return "-- Opaque Rust terms/generators: oracle is the compiled written-order Rust program.\n"
                .into();
        }
        if !self.query.blocks.is_empty() {
            return self.block_sql();
        }
        let mut environment = BTreeMap::new();
        let mut conditions = Vec::new();
        let mut from = Vec::new();
        for (occurrence, atom) in self.query.atoms.iter().enumerate() {
            from.push(format!(
                "\"mini-linq/raw/{}\" AS a{occurrence}",
                atom.relation
            ));
            for (column, term) in atom.terms.iter().enumerate() {
                let field = format!("a{occurrence}.c{column}");
                match term {
                    Term::Var(variable) => {
                        if let Some(previous) = environment.get(variable) {
                            conditions.push(format!("{field} = {previous}"));
                        } else {
                            environment.insert(*variable, field);
                        }
                    }
                    Term::Constant(value) => conditions.push(format!("{field} = {value}")),
                    Term::Wildcard => (),
                    Term::Expression(_) => (),
                }
            }
            // Same-atom dependencies may occur to the right of an expression.
            // Resolve expressions only after every variable position is known.
            for (column, term) in atom.terms.iter().enumerate() {
                if let Term::Expression(expression) = term {
                    conditions.push(format!(
                        "a{occurrence}.c{column} = {}",
                        sql_expression(expression, &environment)
                    ));
                }
            }
        }
        for binding in &self.query.bindings {
            let expression = sql_expression(&binding.expression, &environment);
            environment.insert(binding.variable, expression);
        }
        for guard in &self.query.guards {
            conditions.push(sql_guard(guard, &environment));
        }
        for c in &self.query.constraints {
            conditions.push(sql_constraint(c, &environment));
        }
        let projection = self
            .query
            .head
            .iter()
            .enumerate()
            .map(|(column, expression)| {
                format!(
                    "CAST({} AS INTEGER) AS out{column}",
                    sql_expression(expression, &environment)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let mut sql = format!(
            "SELECT DISTINCT {projection}\nFROM {}\n",
            from.join(" CROSS JOIN ")
        );
        if !conditions.is_empty() {
            writeln!(sql, "WHERE {}", conditions.join(" AND ")).unwrap();
        }
        writeln!(
            sql,
            "ORDER BY {};",
            (1..=self.output_arity())
                .map(|position| position.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
        sql
    }

    /// An atom may disappear only if every remaining expression stays bound.
    /// The runner accepts the candidate only after recompiling and reproducing
    /// the same failure against DuckDB. Rows are shrunk separately by the runner.
    pub fn without_atom(&self, index: usize) -> Option<Self> {
        if !self.query.blocks.is_empty()
            || self.query.atoms.len() <= 1
            || index >= self.query.atoms.len()
        {
            return None;
        }
        let mut candidate = self.clone();
        candidate.query.atoms.remove(index);
        candidate
            .query
            .order
            .retain(|step| *step != Step::Atom(index));
        for step in &mut candidate.query.order {
            if let Step::Atom(occurrence) = step {
                if *occurrence > index {
                    *occurrence -= 1;
                }
            }
        }
        candidate.query.well_bound().then_some(candidate)
    }
}

fn rust_expression(expression: &Expr) -> String {
    match expression {
        // Atom values are borrowed and let values are owned. This ordinary
        // Rust operation yields i32 in either case, including after a planner
        // has materialized an intermediate relation.
        Expr::Var(variable) => format!("v{variable}.to_owned()"),
        Expr::Constant(value) => value.to_string(),
        Expr::Negate(inner) => format!("(-({}))", rust_expression(inner)),
        Expr::OpaqueCall(inner) => format!("std::convert::identity({})", rust_expression(inner)),
        Expr::Rust(operator, inner) => {
            let value = rust_expression(inner);
            match operator {
                RustOp::Divide => format!("(10i32 / ((({value}) % 3) + 4))"),
                RustOp::Remainder => format!("(10i32 % ((({value}) % 3) + 4))"),
                RustOp::Add => format!("((i32::MAX - 3) + (({value}) % 4))"),
                RustOp::Subtract => format!("((i32::MIN + 3) - (({value}) % 4))"),
                RustOp::Multiply => format!("((i32::MAX / 4) * (({value}) % 4))"),
                RustOp::Unwrap => format!("Some({value}).unwrap()"),
            }
        }
        Expr::Binary(operator, left, right) => {
            let token = match operator {
                Arithmetic::Add => "+",
                Arithmetic::Subtract => "-",
                Arithmetic::Multiply => "*",
                Arithmetic::Remainder => "%",
            };
            format!(
                "({} {token} {})",
                rust_expression(left),
                rust_expression(right)
            )
        }
    }
}

fn rust_guard(guard: &Guard) -> String {
    match guard {
        Guard::Compare(comparison, left, right) => {
            let token = match comparison {
                Comparison::Equal => "==",
                Comparison::NotEqual => "!=",
                Comparison::Less => "<",
                Comparison::LessEqual => "<=",
                Comparison::Greater => ">",
                Comparison::GreaterEqual => ">=",
            };
            format!(
                "({} {token} {})",
                rust_expression(left),
                rust_expression(right)
            )
        }
        Guard::And(left, right) => format!("({} && {})", rust_guard(left), rust_guard(right)),
        Guard::Or(left, right) => format!("({} || {})", rust_guard(left), rust_guard(right)),
        Guard::Not(inner) => format!("!({})", rust_guard(inner)),
    }
}

fn cq_constraint(c: &Relational) -> String {
    let term = |t: &RelTerm| match t {
        RelTerm::Var(v) => format!("v{v}"),
        RelTerm::Literal(v) => v.to_string(),
        RelTerm::Parameter(v) => parameter_name(*v),
    };
    match c {
        Relational::Compare(op, l, r) => format!(
            "{} {} {}",
            term(l),
            match op {
                Comparison::Equal => "==",
                Comparison::NotEqual => "!=",
                Comparison::Less => "<",
                Comparison::LessEqual => "<=",
                Comparison::Greater => ">",
                Comparison::GreaterEqual => ">=",
            },
            term(r)
        ),
        Relational::Between(t, l, h) => {
            format!("{} in {}..={}", term(t), term(l), term(h))
        }
        Relational::In(t, ls) => format!(
            "{} in [{}]",
            term(t),
            ls.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Relational::Or(bs) => bs
            .iter()
            .map(|b| {
                format!(
                    "({})",
                    b.iter().map(cq_constraint).collect::<Vec<_>>().join(" && ")
                )
            })
            .collect::<Vec<_>>()
            .join(" || "),
    }
}

/// The SQL printer independently interprets abstract relational nodes.
pub(super) fn sql_constraint(c: &Relational, environment: &BTreeMap<usize, String>) -> String {
    let term = |t: &RelTerm| match t {
        RelTerm::Var(v) => environment[v].clone(),
        RelTerm::Literal(v) | RelTerm::Parameter(v) => v.to_string(),
    };
    match c {
        Relational::Compare(op, l, r) => format!(
            "({} {} {})",
            term(l),
            match op {
                Comparison::Equal => "=",
                Comparison::NotEqual => "<>",
                Comparison::Less => "<",
                Comparison::LessEqual => "<=",
                Comparison::Greater => ">",
                Comparison::GreaterEqual => ">=",
            },
            term(r)
        ),
        Relational::Between(t, l, h) => {
            format!("({} BETWEEN {} AND {})", term(t), term(l), term(h))
        }
        Relational::In(t, ls) => format!(
            "({} IN ({}))",
            term(t),
            ls.iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Relational::Or(bs) => format!(
            "({})",
            bs.iter()
                .map(|b| format!(
                    "({})",
                    b.iter()
                        .map(|c| sql_constraint(c, environment))
                        .collect::<Vec<_>>()
                        .join(" AND ")
                ))
                .collect::<Vec<_>>()
                .join(" OR ")
        ),
    }
}

pub(super) fn sql_expression(expression: &Expr, environment: &BTreeMap<usize, String>) -> String {
    match expression {
        Expr::Rust(..) => unreachable!("opaque Rust operations use the compiled reference"),
        Expr::Var(variable) => environment[variable].clone(),
        Expr::Constant(value) => value.to_string(),
        Expr::Negate(inner) => format!("(-({}))", sql_expression(inner, environment)),
        Expr::OpaqueCall(inner) => sql_expression(inner, environment),
        Expr::Binary(operator, left, right) => {
            let token = match operator {
                Arithmetic::Add => "+",
                Arithmetic::Subtract => "-",
                Arithmetic::Multiply => "*",
                // Both Rust i32 and DuckDB INTEGER remainder keep the dividend
                // sign. Divisors are generated as strictly positive constants.
                Arithmetic::Remainder => "%",
            };
            format!(
                "({} {token} {})",
                sql_expression(left, environment),
                sql_expression(right, environment)
            )
        }
    }
}

pub(super) fn sql_guard(guard: &Guard, environment: &BTreeMap<usize, String>) -> String {
    match guard {
        Guard::Compare(comparison, left, right) => {
            let token = match comparison {
                Comparison::Equal => "=",
                Comparison::NotEqual => "<>",
                Comparison::Less => "<",
                Comparison::LessEqual => "<=",
                Comparison::Greater => ">",
                Comparison::GreaterEqual => ">=",
            };
            format!(
                "({} {token} {})",
                sql_expression(left, environment),
                sql_expression(right, environment)
            )
        }
        Guard::And(left, right) => format!(
            "({} AND {})",
            sql_guard(left, environment),
            sql_guard(right, environment)
        ),
        Guard::Or(left, right) => format!(
            "({} OR {})",
            sql_guard(left, environment),
            sql_guard(right, environment)
        ),
        Guard::Not(inner) => format!("(NOT {})", sql_guard(inner, environment)),
    }
}

impl Expr {
    fn features(&self, output: &mut BTreeSet<String>) {
        match self {
            Self::Var(_) | Self::Constant(_) => (),
            Self::Rust(operator, inner) => {
                output.insert("rust-term".into());
                output.insert(format!("rust-{}", operator.name()));
                inner.features(output);
            }
            Self::Binary(operator, left, right) => {
                output.insert(
                    match operator {
                        Arithmetic::Add => "expression-add",
                        Arithmetic::Subtract => "expression-subtract",
                        Arithmetic::Multiply => "expression-multiply",
                        Arithmetic::Remainder => "expression-remainder",
                    }
                    .into(),
                );
                left.features(output);
                right.features(output);
            }
            Self::OpaqueCall(inner) => {
                output.insert("opaque-call".into());
                inner.features(output);
            }
            Self::Negate(inner) => {
                output.insert("expression-negate".into());
                inner.features(output);
            }
        }
    }

    fn variables(&self, output: &mut BTreeSet<usize>) {
        match self {
            Self::Var(variable) => {
                output.insert(*variable);
            }
            Self::Constant(_) => (),
            Self::Binary(_, left, right) => {
                left.variables(output);
                right.variables(output);
            }
            Self::Negate(inner) | Self::Rust(_, inner) | Self::OpaqueCall(inner) => {
                inner.variables(output)
            }
        }
    }

    fn has_arithmetic(&self) -> bool {
        matches!(self, Self::Binary(..) | Self::Negate(_) | Self::Rust(..))
    }

    fn value(&self, environment: &[i32]) -> i32 {
        match self {
            Self::Rust(..) => unreachable!("opaque Rust operations use the compiled reference"),
            Self::Var(variable) => environment[*variable],
            Self::Constant(value) => *value,
            Self::Negate(inner) => -inner.value(environment),
            Self::OpaqueCall(inner) => inner.value(environment),
            Self::Binary(operator, left, right) => {
                let left = left.value(environment);
                let right = right.value(environment);
                match operator {
                    Arithmetic::Add => left + right,
                    Arithmetic::Subtract => left - right,
                    Arithmetic::Multiply => left * right,
                    Arithmetic::Remainder => left % right,
                }
            }
        }
    }
}

impl Guard {
    fn has_simple_operands(&self) -> bool {
        match self {
            Self::Compare(_, l, r) => {
                matches!(l, Expr::Var(_) | Expr::Constant(_))
                    && matches!(r, Expr::Var(_) | Expr::Constant(_))
            }
            Self::And(l, r) | Self::Or(l, r) => l.has_simple_operands() && r.has_simple_operands(),
            Self::Not(g) => g.has_simple_operands(),
        }
    }

    fn features(&self, output: &mut BTreeSet<String>) {
        match self {
            Self::Compare(comparison, left, right) => {
                output.insert(
                    match comparison {
                        Comparison::Equal => "comparison-equal",
                        Comparison::NotEqual => "comparison-not-equal",
                        Comparison::Less => "comparison-less",
                        Comparison::LessEqual => "comparison-less-equal",
                        Comparison::Greater => "comparison-greater",
                        Comparison::GreaterEqual => "comparison-greater-equal",
                    }
                    .into(),
                );
                left.features(output);
                right.features(output);
            }
            Self::And(left, right) | Self::Or(left, right) => {
                output.insert(
                    if matches!(self, Self::And(..)) {
                        "boolean-and"
                    } else {
                        "boolean-or"
                    }
                    .into(),
                );
                if left.has_simple_operands() != right.has_simple_operands() {
                    output.insert(
                        if matches!(self, Self::And(..)) {
                            "mixed-and"
                        } else {
                            "mixed-or"
                        }
                        .into(),
                    );
                }
                left.features(output);
                right.features(output);
            }
            Self::Not(inner) => {
                output.insert("boolean-not".into());
                if !inner.has_simple_operands() {
                    output.insert("opaque-not".into());
                }
                inner.features(output);
            }
        }
    }

    fn variables(&self, output: &mut BTreeSet<usize>) {
        match self {
            Self::Compare(_, left, right) => {
                left.variables(output);
                right.variables(output);
            }
            Self::And(left, right) | Self::Or(left, right) => {
                left.variables(output);
                right.variables(output);
            }
            Self::Not(inner) => inner.variables(output),
        }
    }

    fn has_arithmetic(&self) -> bool {
        match self {
            Self::Compare(_, left, right) => left.has_arithmetic() || right.has_arithmetic(),
            Self::And(left, right) | Self::Or(left, right) => {
                left.has_arithmetic() || right.has_arithmetic()
            }
            Self::Not(inner) => inner.has_arithmetic(),
        }
    }

    fn value(&self, environment: &[i32]) -> bool {
        match self {
            Self::Compare(comparison, left, right) => {
                let left = left.value(environment);
                let right = right.value(environment);
                match comparison {
                    Comparison::Equal => left == right,
                    Comparison::NotEqual => left != right,
                    Comparison::Less => left < right,
                    Comparison::LessEqual => left <= right,
                    Comparison::Greater => left > right,
                    Comparison::GreaterEqual => left >= right,
                }
            }
            Self::And(left, right) => left.value(environment) && right.value(environment),
            Self::Or(left, right) => left.value(environment) || right.value(environment),
            Self::Not(inner) => !inner.value(environment),
        }
    }
}

impl Query {
    fn cq_steps(&self, steps: &[Step]) -> String {
        let mut clauses = Vec::new();
        for step in steps.iter().copied() {
            match step {
                Step::Block(index) => {
                    let block = &self.blocks[index];
                    let operator = if block.kind == BlockKind::Union {
                        " | "
                    } else {
                        " else "
                    };
                    clauses.push(
                        block
                            .branches
                            .iter()
                            .map(|arm| format!("{{ {} }}", self.cq_steps(arm)))
                            .collect::<Vec<_>>()
                            .join(operator),
                    );
                }
                Step::Atom(index) => {
                    let atom = &self.atoms[index];
                    let terms = atom
                        .terms
                        .iter()
                        .map(|term| match term {
                            Term::Var(variable) => format!("v{variable}"),
                            Term::Constant(value) => value.to_string(),
                            Term::Wildcard => "_".to_owned(),
                            Term::Expression(expression) => rust_expression(expression),
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    clauses.push(format!("{}({terms})", self.schema[atom.relation].name));
                }
                Step::Binding(index) => {
                    let binding = &self.bindings[index];
                    clauses.push(if let Some(generator) = binding.generator {
                        format!(
                            "for v{} in {}",
                            binding.variable,
                            generator.rust(&binding.expression)
                        )
                    } else {
                        format!(
                            "let v{}: i32 = {}",
                            binding.variable,
                            rust_expression(&binding.expression)
                        )
                    });
                }
                Step::Constraint(index) => {
                    clauses.push(format!("where {}", cq_constraint(&self.constraints[index])))
                }
                Step::Guard(index) => {
                    let guard = &self.guards[index];
                    clauses.push(format!("if {}", rust_guard(guard)));
                }
            }
        }
        clauses.join(", ")
    }
    pub(super) fn body_order(&self) -> Vec<Step> {
        if !self.order.is_empty() {
            return self.order.clone();
        }
        (0..self.atoms.len())
            .map(Step::Atom)
            .chain((0..self.bindings.len()).map(Step::Binding))
            .chain((0..self.guards.len()).map(Step::Guard))
            .chain((0..self.constraints.len()).map(Step::Constraint))
            .collect()
    }

    fn well_bound(&self) -> bool {
        let mut bound = BTreeSet::new();
        if !self.steps_bound(&self.body_order(), &mut bound) {
            return false;
        }
        let mut used = BTreeSet::new();
        for expression in &self.head {
            expression.variables(&mut used);
        }
        used.is_subset(&bound)
    }

    fn steps_bound(&self, steps: &[Step], bound: &mut BTreeSet<usize>) -> bool {
        for step in steps.iter().copied() {
            match step {
                Step::Block(index) => {
                    let block = &self.blocks[index];
                    let mut common: Option<BTreeSet<usize>> = None;
                    for branch in &block.branches {
                        let mut names = bound.clone();
                        if !self.steps_bound(branch, &mut names) {
                            return false;
                        }
                        common = Some(match common {
                            None => names,
                            Some(old) => old.intersection(&names).copied().collect::<BTreeSet<_>>(),
                        });
                    }
                    let mut expected = bound.clone();
                    expected.extend(&block.interface);
                    if common != Some(expected.clone()) {
                        return false;
                    }
                    *bound = expected;
                }
                Step::Atom(index) => {
                    let atom = &self.atoms[index];
                    bound.extend(atom_variables(atom));
                    for term in &atom.terms {
                        if let Term::Expression(expression) = term {
                            let mut used = BTreeSet::new();
                            expression.variables(&mut used);
                            if !used.is_subset(bound) {
                                return false;
                            }
                        }
                    }
                }
                Step::Binding(index) => {
                    let binding = &self.bindings[index];
                    let mut used = BTreeSet::new();
                    binding.expression.variables(&mut used);
                    if !used.is_subset(bound) || !bound.insert(binding.variable) {
                        return false;
                    }
                }
                Step::Constraint(index) => {
                    let mut used = BTreeSet::new();
                    self.constraints[index].variables(&mut used);
                    if !used.is_subset(bound) {
                        return false;
                    }
                }
                Step::Guard(index) => {
                    let mut used = BTreeSet::new();
                    self.guards[index].variables(&mut used);
                    if !used.is_subset(bound) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn disconnected(&self) -> bool {
        let mut reached = BTreeSet::from([0]);
        let mut variables = atom_variables(&self.atoms[0]);
        loop {
            let before = reached.len();
            for (index, atom) in self.atoms.iter().enumerate() {
                let atom_vars = atom_variables(atom);
                if !variables.is_disjoint(&atom_vars) {
                    reached.insert(index);
                    variables.extend(atom_vars);
                }
            }
            if reached.len() == before {
                return reached.len() != self.atoms.len();
            }
        }
    }
}

fn atom_variables(atom: &Atom) -> BTreeSet<usize> {
    atom.terms
        .iter()
        .filter_map(|term| match term {
            Term::Var(variable) => Some(*variable),
            Term::Constant(_) | Term::Wildcard | Term::Expression(_) => None,
        })
        .collect()
}

/// SplitMix64 with explicit wrapping arithmetic gives stable cases on all hosts
/// and does not add a dependency or depend on Rust's randomized hash state.
struct Random(crate::random::SplitMix64);

impl Random {
    fn pick(&mut self, alternatives: usize) -> usize {
        (self.0.next_u64() % alternatives as u64) as usize
    }

    fn small(&mut self) -> i32 {
        self.pick(7) as i32 - 3
    }

    fn expression(&mut self, variables: usize, depth: usize) -> Expr {
        if depth == 0 {
            return if self.pick(4) == 0 {
                Expr::Constant(self.small())
            } else {
                Expr::Var(self.pick(variables))
            };
        }
        match self.pick(7) {
            0 => Expr::Var(self.pick(variables)),
            1 => Expr::Negate(Box::new(self.expression(variables, depth - 1))),
            choice => {
                let operator = match choice {
                    2 => Arithmetic::Add,
                    3 => Arithmetic::Subtract,
                    4 => Arithmetic::Multiply,
                    _ => Arithmetic::Remainder,
                };
                let left = self.expression(variables, depth - 1);
                let right = if operator == Arithmetic::Remainder {
                    Expr::Constant(1 + self.pick(4) as i32)
                } else {
                    self.expression(variables, depth - 1)
                };
                Expr::Binary(operator, Box::new(left), Box::new(right))
            }
        }
    }

    fn comparison(&mut self, variables: usize) -> Guard {
        let comparison = match self.pick(6) {
            0 => Comparison::Equal,
            1 => Comparison::NotEqual,
            2 => Comparison::Less,
            3 => Comparison::LessEqual,
            4 => Comparison::Greater,
            _ => Comparison::GreaterEqual,
        };
        Guard::Compare(
            comparison,
            self.expression(variables, 1),
            self.expression(variables, 1),
        )
    }

    fn relational_term(&mut self, variables: usize) -> RelTerm {
        match self.pick(4) {
            0 => RelTerm::Literal(self.small()),
            1 => RelTerm::Parameter(self.small()),
            _ => RelTerm::Var(self.pick(variables)),
        }
    }
    fn relational_atomic(&mut self, variables: usize) -> Relational {
        let v = RelTerm::Var(self.pick(variables));
        match self.pick(8) {
            6 => Relational::Between(
                v,
                self.relational_term(variables),
                self.relational_term(variables),
            ),
            7 => Relational::In(v, (0..1 + self.pick(4)).map(|_| self.small()).collect()),
            op => Relational::Compare(
                match op {
                    0 => Comparison::Equal,
                    1 => Comparison::NotEqual,
                    2 => Comparison::Less,
                    3 => Comparison::LessEqual,
                    4 => Comparison::Greater,
                    _ => Comparison::GreaterEqual,
                },
                v,
                self.relational_term(variables),
            ),
        }
    }
    fn relational(&mut self, variables: usize) -> Relational {
        if self.pick(4) == 0 {
            Relational::Or(
                (0..2 + self.pick(2))
                    .map(|_| {
                        (0..1 + self.pick(3))
                            .map(|_| self.relational_atomic(variables))
                            .collect()
                    })
                    .collect(),
            )
        } else {
            self.relational_atomic(variables)
        }
    }

    fn guard(&mut self, variables: usize) -> Guard {
        let relational = Guard::Compare(
            match self.pick(6) {
                0 => Comparison::Equal,
                1 => Comparison::NotEqual,
                2 => Comparison::Less,
                3 => Comparison::LessEqual,
                4 => Comparison::Greater,
                _ => Comparison::GreaterEqual,
            },
            Expr::Var(self.pick(variables)),
            Expr::Constant(self.small()),
        );
        let opaque = if self.pick(2) == 0 {
            Guard::Compare(
                Comparison::Less,
                Expr::OpaqueCall(Box::new(Expr::Var(self.pick(variables)))),
                Expr::Constant(self.small()),
            )
        } else {
            Guard::Compare(
                Comparison::Greater,
                Expr::Binary(
                    Arithmetic::Add,
                    Box::new(Expr::Var(self.pick(variables))),
                    Box::new(Expr::Constant(self.small())),
                ),
                Expr::Constant(self.small()),
            )
        };
        match self.pick(7) {
            0 => self.comparison(variables),
            1 => Guard::Not(Box::new(relational)),
            2 => Guard::And(Box::new(relational), Box::new(opaque)),
            3 => Guard::And(Box::new(opaque), Box::new(relational)),
            4 => Guard::Or(Box::new(relational), Box::new(opaque)),
            5 => Guard::Or(Box::new(opaque), Box::new(relational)),
            _ => Guard::Not(Box::new(Guard::And(Box::new(relational), Box::new(opaque)))),
        }
    }
}

/// Generate a reproducible bounded query and raw input tables.
///
/// Even seeds sample total opaque Rust with a compiled oracle; odd seeds sample
/// total queries with independent SQL. Schemas have 1..=4 tables of arity 1..=3,
/// 1..=5 atoms, and heads have 1..=3 columns. Raw duplicate rows are retained.
pub fn generate(seed: u64) -> Case {
    if matches!(seed % 10, 3 | 8) {
        return generate_block(seed);
    }
    if seed & 1 == 0 {
        return generate_rust(seed);
    }
    let mut random = Random(crate::random::SplitMix64::new(seed));
    let relations = 1 + random.pick(4);
    let schema = (0..relations)
        .map(|index| Relation {
            name: format!("r{index}"),
            arity: 1 + random.pick(3),
        })
        .collect::<Vec<_>>();
    let count = 1 + random.pick(5);
    let mut variables = 0;
    let mut atoms = Vec::new();
    for index in 0..count {
        let relation = random.pick(relations);
        // Fresh variables in some later atoms deliberately sample disconnected
        // components; arbitrary reuse also creates cycles and self-joins.
        let fresh_component = index > 0 && random.pick(4) == 0;
        let mut terms = Vec::new();
        for column in 0..schema[relation].arity {
            let choice = random.pick(10);
            let term = if variables == 0 || (fresh_component && column == 0) || choice < 3 {
                let variable = variables;
                variables += 1;
                Term::Var(variable)
            } else if choice < 6 && !fresh_component {
                Term::Var(random.pick(variables))
            } else if choice < 8 {
                Term::Constant(random.small())
            } else {
                Term::Wildcard
            };
            terms.push(term);
        }
        atoms.push(Atom { relation, terms });
    }
    let mut witness = (0..variables).map(|_| random.small()).collect::<Vec<_>>();
    let mut bindings = Vec::new();
    for _ in 0..random.pick(3) {
        // One multiplication per let and at most two lets keep every possible
        // input valuation far below i32 overflow, even for two-level heads.
        let expression = Expr::Binary(
            match random.pick(3) {
                0 => Arithmetic::Add,
                1 => Arithmetic::Subtract,
                _ => Arithmetic::Multiply,
            },
            Box::new(Expr::Var(random.pick(variables))),
            Box::new(Expr::Constant(random.small())),
        );
        witness.push(expression.value(&witness));
        bindings.push(Binding {
            variable: variables,
            expression,
            generator: None,
        });
        variables += 1;
    }
    let preserve_witness = random.pick(4) != 0;
    let guards = (0..random.pick(3))
        .map(|_| {
            let guard = random.guard(variables);
            if preserve_witness && !guard.value(&witness) {
                Guard::Not(Box::new(guard))
            } else {
                guard
            }
        })
        .collect();
    let constraints = (0..1 + random.pick(3))
        .map(|_| random.relational(variables))
        .collect();
    let head = (0..1 + random.pick(3))
        .map(|_| {
            if random.pick(3) == 0 {
                random.expression(variables, 2)
            } else {
                Expr::Var(random.pick(variables))
            }
        })
        .collect();
    let mut data = schema
        .iter()
        .map(|relation| {
            (0..random.pick(5))
                .map(|_| {
                    (0..relation.arity)
                        .map(|_| random.small())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    if preserve_witness {
        for atom in &atoms {
            data[atom.relation].push(
                atom.terms
                    .iter()
                    .map(|term| match term {
                        Term::Var(variable) => witness[*variable],
                        Term::Constant(value) => *value,
                        Term::Wildcard => random.small(),
                        Term::Expression(_) => unreachable!("total lane atoms"),
                    })
                    .collect(),
            );
        }
    }
    for rows in &mut data {
        if !rows.is_empty() && random.pick(2) == 0 {
            rows.push(rows[random.pick(rows.len())].clone());
        }
    }
    if random.pick(5) == 0 {
        // Pick a used input, so this also exercises empty query answers.
        data[atoms[random.pick(atoms.len())].relation].clear();
    }
    Case {
        seed,
        query: Query {
            schema,
            atoms,
            bindings,
            guards,
            constraints,
            blocks: Vec::new(),
            head,
            order: Vec::new(),
        },
        data,
    }
}

fn generate_block(seed: u64) -> Case {
    let mut random = Random(crate::random::SplitMix64::new(seed));
    let rust_oracle = seed & 1 == 0;
    let test = (seed / 10) % 3 == 0;
    let kind = if (seed / 10) % 2 == 0 {
        BlockKind::Union
    } else {
        BlockKind::Fallback
    };
    let mut query = Query {
        schema: vec![
            Relation {
                name: "r0".into(),
                arity: 1,
            },
            Relation {
                name: "r1".into(),
                arity: 3,
            },
            Relation {
                name: "r2".into(),
                arity: 3,
            },
        ],
        atoms: vec![
            Atom {
                relation: 0,
                terms: vec![Term::Var(0)],
            },
            Atom {
                relation: 1,
                terms: vec![Term::Var(0), Term::Var(1), Term::Var(3)],
            },
            Atom {
                relation: 2,
                terms: vec![
                    Term::Var(0),
                    Term::Var(if test { 2 } else { 1 }),
                    Term::Var(4),
                ],
            },
        ],
        bindings: Vec::new(),
        guards: Vec::new(),
        constraints: Vec::new(),
        blocks: Vec::new(),
        head: if test {
            vec![Expr::Var(0)]
        } else {
            vec![Expr::Var(0), Expr::Var(1)]
        },
        order: vec![Step::Atom(0), Step::Block(0)],
    };
    let mut second = vec![Step::Atom(2)];
    if rust_oracle {
        query.guards.push(Guard::Compare(
            Comparison::GreaterEqual,
            Expr::Rust(RustOp::Divide, Box::new(Expr::Var(0))),
            Expr::Constant(0),
        ));
        second.push(Step::Guard(0));
    } else if random.pick(2) == 0 {
        query.guards.push(Guard::Compare(
            Comparison::GreaterEqual,
            Expr::OpaqueCall(Box::new(Expr::Var(0))),
            Expr::Constant(random.small()),
        ));
        second.push(Step::Guard(0));
    }
    query.blocks.push(Block {
        kind,
        branches: vec![vec![Step::Atom(1)], second],
        interface: if test { vec![] } else { vec![1] },
    });
    if (seed / 10) % 3 == 1 {
        query.atoms.push(query.atoms[1].clone());
        query.blocks.push(Block {
            kind: BlockKind::Union,
            branches: vec![vec![Step::Atom(1)], vec![Step::Atom(3)]],
            interface: vec![1, 3],
        });
        query.blocks[0].branches[0] = vec![Step::Block(1)];
    }
    if (seed / 10) % 5 == 4 {
        query.atoms.truncate(1);
        query.guards.clear();
        query.blocks.clear();
        query.bindings.push(Binding {
            variable: 1,
            generator: None,
            expression: if rust_oracle {
                Expr::Rust(RustOp::Divide, Box::new(Expr::Var(0)))
            } else {
                Expr::OpaqueCall(Box::new(Expr::Var(0)))
            },
        });
        query.constraints = vec![
            Relational::Compare(Comparison::Equal, RelTerm::Var(1), RelTerm::Literal(1)),
            Relational::Compare(Comparison::Equal, RelTerm::Var(1), RelTerm::Literal(2)),
        ];
        query.blocks.push(Block {
            kind,
            branches: vec![vec![Step::Constraint(0)], vec![Step::Constraint(1)]],
            interface: vec![],
        });
        query.head = vec![Expr::Var(0)];
        query.order = vec![Step::Atom(0), Step::Binding(0), Step::Block(0)];
    }
    let data = query
        .schema
        .iter()
        .map(|schema| {
            let mut rows = (0..random.pick(8))
                .map(|_| {
                    (0..schema.arity)
                        .map(|_| random.small())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            if !rows.is_empty() {
                rows.push(rows[0].clone());
            }
            rows
        })
        .collect();
    assert!(query.well_bound());
    Case { seed, query, data }
}

fn generate_rust(seed: u64) -> Case {
    let mut random = Random(crate::random::SplitMix64::new(seed ^ 0x0050_4152_5449_414c));
    // Consecutive even seeds cover every operator/position combination before
    // random data and body schedules vary its domain of evaluation.
    let ordinal = seed / 2;
    let operator = [
        RustOp::Divide,
        RustOp::Remainder,
        RustOp::Add,
        RustOp::Subtract,
        RustOp::Multiply,
        RustOp::Unwrap,
    ][(ordinal % 6) as usize];
    let position = ((ordinal / 6) % 7) as usize;
    let operation = |variable| Expr::Rust(operator, Box::new(Expr::Var(variable)));
    let selection = match operator {
        RustOp::Divide | RustOp::Remainder | RustOp::Unwrap => {
            Guard::Compare(Comparison::NotEqual, Expr::Var(0), Expr::Constant(0))
        }
        RustOp::Add | RustOp::Subtract => {
            Guard::Compare(Comparison::LessEqual, Expr::Var(0), Expr::Constant(0))
        }
        RustOp::Multiply => Guard::And(
            Box::new(Guard::Compare(
                Comparison::GreaterEqual,
                Expr::Var(0),
                Expr::Constant(0),
            )),
            Box::new(Guard::Compare(
                Comparison::LessEqual,
                Expr::Var(0),
                Expr::Constant(1),
            )),
        ),
    };
    let mut query = Query {
        schema: vec![
            Relation {
                name: "r0".into(),
                arity: 1,
            },
            Relation {
                name: "r1".into(),
                arity: 2,
            },
        ],
        atoms: vec![
            Atom {
                relation: 0,
                terms: vec![Term::Var(0)],
            },
            Atom {
                relation: 1,
                terms: vec![Term::Var(1), Term::Var(2)],
            },
        ],
        bindings: Vec::new(),
        guards: vec![selection],
        constraints: Vec::new(),
        blocks: Vec::new(),
        head: vec![Expr::Var(0), Expr::Var(2)],
        order: vec![Step::Atom(0)],
    };
    let guard_first = random.pick(3) != 0;
    let relation_before_rust = random.pick(2) == 0;
    if relation_before_rust && !(2..5).contains(&position) {
        query.order.push(Step::Atom(1));
    }
    if guard_first {
        query.order.push(Step::Guard(0));
    }
    match position {
        0 => {
            query.guards.push(Guard::Compare(
                Comparison::GreaterEqual,
                operation(0),
                Expr::Constant(0),
            ));
            query.order.push(Step::Guard(1));
        }
        1 => {
            query.bindings.push(Binding {
                variable: 3,
                expression: operation(0),
                generator: None,
            });
            query.order.push(Step::Binding(0));
            query.head[0] = Expr::Var(3);
        }
        2 | 3 => {
            // Both dependencies from earlier atoms and same-atom variables
            // exercise the two argument-expression desugaring rules.
            query.atoms[1].terms[0] =
                Term::Expression(operation(if position == 2 { 0 } else { 2 }));
            query.order.push(Step::Atom(1));
            if position == 3 {
                // Occurrence literals restrict rows independently of same-atom
                // total Rust expressions, regardless of argument order.
                query.schema[1].arity = 3;
                query.atoms[1].terms.push(Term::Constant(5));
            }
        }
        5 | 6 => {
            let generator = [
                Generator::Range,
                Generator::InclusiveRange,
                Generator::Duplicates,
                Generator::Empty,
                Generator::Mapped,
            ][((ordinal / 42) % 5) as usize];
            query.bindings.push(Binding {
                variable: 3,
                expression: if position == 5 {
                    operation(0)
                } else {
                    Expr::Var(0)
                },
                generator: Some(generator),
            });
            query.order.push(Step::Binding(0));
            query.head[0] = Expr::Var(3);
            if (ordinal / 210) % 2 == 1 {
                query.bindings.push(Binding {
                    variable: 4,
                    expression: Expr::Var(3),
                    generator: Some(Generator::Duplicates),
                });
                query.order.push(Step::Binding(1));
                query.head.push(Expr::Var(4));
            }
            if !relation_before_rust {
                // A later relational occurrence joins against the generator.
                query.atoms[1].terms[0] = Term::Var(3);
            }
        }
        _ => {
            query.head[0] = operation(0);
            query.order.push(Step::Atom(1));
        }
    }
    if !(2..5).contains(&position) && !relation_before_rust {
        query.order.push(Step::Atom(1));
    }
    if !guard_first {
        query.order.push(Step::Guard(0));
    }
    if position == 0 {
        let mixed = match random.pick(6) {
            0 => Guard::And(
                Box::new(query.guards[0].clone()),
                Box::new(query.guards[1].clone()),
            ),
            1 => Guard::And(
                Box::new(query.guards[1].clone()),
                Box::new(query.guards[0].clone()),
            ),
            2 => Guard::Or(
                Box::new(query.guards[0].clone()),
                Box::new(query.guards[1].clone()),
            ),
            3 => Guard::Or(
                Box::new(query.guards[1].clone()),
                Box::new(query.guards[0].clone()),
            ),
            4 => Guard::Not(Box::new(Guard::And(
                Box::new(query.guards[0].clone()),
                Box::new(query.guards[1].clone()),
            ))),
            _ => Guard::Not(Box::new(Guard::Or(
                Box::new(query.guards[1].clone()),
                Box::new(query.guards[0].clone()),
            ))),
        };
        query.guards[1] = mixed;
    }
    // Relational constraints sample either side of opaque Rust in source.
    // Placement depends only on available variables, not a written-order chain.
    for i in 0..1 + random.pick(2) {
        query.constraints.push(random.relational(1));
        if random.pick(2) == 0 {
            query.order.insert(1, Step::Constraint(i));
        } else {
            query.order.push(Step::Constraint(i));
        }
    }
    let values = [-2, -1, 0, 1, 2, i32::MIN, i32::MAX];
    let mut data = vec![
        (0..1 + random.pick(5))
            .map(|_| vec![values[random.pick(values.len())]])
            .collect::<Vec<_>>(),
        (0..1 + random.pick(5))
            .map(|_| {
                vec![
                    values[random.pick(values.len())],
                    values[random.pick(values.len())],
                ]
            })
            .collect::<Vec<_>>(),
    ];
    if query.schema[1].arity == 3 {
        for row in &mut data[1] {
            row.push(5 + random.pick(2) as i32);
        }
        // The occurrence literal excludes this row independently of where the
        // total local expression is evaluated.
        data[1].push(vec![0, 0, 6]);
    }
    // Empty inputs must give empty answers under eager and split configurations.
    if random.pick(4) == 0 {
        data[1].clear();
    }
    if random.pick(8) == 0 {
        data[0].clear();
    }
    if random.pick(3) == 0 && !data[0].is_empty() {
        let duplicate = data[0][0].clone();
        data[0].push(duplicate);
    }
    assert!(query.well_bound());
    Case { seed, query, data }
}

/// Counts cases with each property, rather than counting syntax occurrences.
/// Histograms and feature counts overlap intentionally.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    counts: BTreeMap<String, usize>,
}

impl Coverage {
    /// All observed feature counts in stable name order.
    pub fn counts(&self) -> &BTreeMap<String, usize> {
        &self.counts
    }

    /// The number of observed cases with a feature (zero if never observed).
    pub fn count(&self, feature: &str) -> usize {
        self.counts.get(feature).copied().unwrap_or(0)
    }

    /// Count every property of this case once, retaining overlapping tags.
    pub fn observe(&mut self, case: &Case) {
        let query = &case.query;
        let mut features = BTreeSet::from([
            "cases".to_owned(),
            format!("atoms={}", query.atoms.len()),
            format!("relations={}", query.schema.len()),
            format!("head-arity={}", query.head.len()),
        ]);
        for block in &query.blocks {
            features.insert(
                if block.kind == BlockKind::Union {
                    "union-block"
                } else {
                    "fallback-block"
                }
                .into(),
            );
            features.insert(
                if block.interface.is_empty() {
                    "test-block"
                } else {
                    "binding-block"
                }
                .into(),
            );
            if query.atoms.len() > 1 {
                features.insert("block-locals".into());
            }
            if block
                .branches
                .iter()
                .flatten()
                .any(|step| matches!(step, Step::Block(_)))
            {
                features.insert("nested-block".into());
            }
        }
        if !query.blocks.is_empty()
            && query
                .order
                .iter()
                .take_while(|step| !matches!(step, Step::Block(_)))
                .any(|step| matches!(step, Step::Binding(_)))
        {
            features.insert("prefix-let-block".into());
        }
        if !query.blocks.is_empty() && !query.guards.is_empty() {
            features.insert("rust-in-block".into());
        }
        for relation in &query.schema {
            features.insert(format!("relation-arity={}", relation.arity));
        }
        if query
            .atoms
            .iter()
            .map(|atom| atom.relation)
            .collect::<BTreeSet<_>>()
            .len()
            < query.atoms.len()
        {
            features.insert("self-join".into());
        }
        for atom in &query.atoms {
            let mut seen = BTreeSet::new();
            for term in &atom.terms {
                match term {
                    Term::Var(variable) => {
                        if !seen.insert(*variable) {
                            features.insert("repeated-variable".into());
                        }
                    }
                    Term::Constant(value) => {
                        features.insert("constant".into());
                        if *value < 0 {
                            features.insert("negative-constant".into());
                        }
                    }
                    Term::Wildcard => {
                        features.insert("wildcard".into());
                    }
                    Term::Expression(expression) => {
                        features.insert("rust-argument".into());
                        expression.features(&mut features);
                    }
                }
            }
        }
        for c in &query.constraints {
            c.features(&mut features);
        }
        if !query.guards.is_empty() {
            features.insert("guard".into());
        }
        for guard in &query.guards {
            guard.features(&mut features);
            let mut own = BTreeSet::new();
            guard.features(&mut own);
            if own.contains("rust-term") {
                features.insert("rust-guard".into());
            }
        }
        for binding in &query.bindings {
            let mut own = BTreeSet::new();
            binding.expression.features(&mut own);
            if let Some(generator) = binding.generator {
                features.insert("for".into());
                features.insert(generator.feature().into());
                features.insert("rust-term".into());
                let mut variables = BTreeSet::new();
                binding.expression.variables(&mut variables);
                if variables.iter().any(|variable| {
                    query.bindings.iter().any(|previous| {
                        previous.generator.is_some() && previous.variable == *variable
                    })
                }) {
                    features.insert("nested-for".into());
                }
                if own.contains("rust-term") {
                    features.insert("rust-for".into());
                }
            } else if own.contains("rust-term") {
                features.insert("rust-let".into());
            }
        }
        for expression in &query.head {
            let mut own = BTreeSet::new();
            expression.features(&mut own);
            if own.contains("rust-term") {
                features.insert("rust-head".into());
            }
        }
        for expression in query
            .head
            .iter()
            .chain(query.bindings.iter().map(|binding| &binding.expression))
        {
            expression.features(&mut features);
        }
        if query.guards.iter().any(Guard::has_arithmetic) {
            features.insert("arithmetic-guard".into());
        }
        if query
            .guards
            .iter()
            .any(|guard| !matches!(guard, Guard::Compare(..)))
        {
            features.insert("boolean-guard".into());
        }
        if query
            .bindings
            .iter()
            .any(|binding| binding.generator.is_none())
        {
            features.insert("arithmetic-let".into());
        }
        if query.head.iter().any(|expr| !matches!(expr, Expr::Var(_))) {
            features.insert("computed-head".into());
        }
        let mut head_variables = BTreeSet::new();
        if query.head.iter().any(|expression| match expression {
            Expr::Var(variable) => !head_variables.insert(*variable),
            _ => false,
        }) {
            features.insert("repeated-head-variable".into());
        }
        if query.disconnected() {
            features.insert("disconnected".into());
        }
        if case.data.iter().any(Vec::is_empty) {
            features.insert("empty-relation".into());
        }
        if query
            .atoms
            .iter()
            .any(|atom| case.data[atom.relation].is_empty())
        {
            features.insert("empty-used-relation".into());
        }
        if case
            .data
            .iter()
            .any(|rows| rows.iter().collect::<BTreeSet<_>>().len() != rows.len())
        {
            features.insert("duplicate-rows".into());
        }
        for feature in features {
            *self.counts.entry(feature).or_default() += 1;
        }
    }
}

impl fmt::Display for Coverage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, (feature, count)) in self.counts.iter().enumerate() {
            if index > 0 {
                write!(formatter, ", ")?;
            }
            write!(formatter, "{feature}:{count}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_reference_uses_the_same_opaque_rust_and_relational_syntax() {
        for seed in 0..64 {
            let case = generate(seed);
            assert_eq!(case.written_cq(), case.cq());
            let written: cq_lang_cq::Program = syn::parse_str(&case.written_cq()).unwrap();
            cq_lang_cq::contract::check(&written).unwrap();
            for constraint in &case.query.constraints {
                assert!(
                    case.cq()
                        .contains(&format!("where {}", cq_constraint(constraint)))
                );
            }
        }
    }

    #[test]
    fn seeded_cases_cover_every_required_feature_and_stay_bounded() {
        let mut coverage = Coverage::default();
        for seed in 0..2048 {
            let case = generate(seed);
            assert_eq!(case, generate(seed));
            let params = case.parameters();
            assert!(params.keys().all(|name| case.cq().contains(name)));
            assert!((1..=4).contains(&case.query.schema.len()));
            assert!((1..=5).contains(&case.query.atoms.len()));
            assert!(case.query.well_bound());
            for (relation, rows) in case.query.schema.iter().zip(&case.data) {
                assert!((1..=3).contains(&relation.arity));
                assert!(rows.iter().all(|row| row.len() == relation.arity));
                if !case.requires_rust_oracle() {
                    assert!(rows.iter().flatten().all(|value| (-3..=3).contains(value)));
                }
            }
            coverage.observe(&case);
        }
        for &feature in super::super::FEATURE_TAGS {
            assert!(coverage.counts[feature] > 30, "{feature}: {coverage}");
        }
    }

    #[test]
    fn independent_printers_preserve_negative_constants_repeats_and_lets() {
        let case = Case {
            seed: 0,
            query: Query {
                schema: vec![Relation {
                    name: "r0".into(),
                    arity: 3,
                }],
                atoms: vec![
                    Atom {
                        relation: 0,
                        terms: vec![Term::Var(0), Term::Var(0), Term::Constant(-2)],
                    },
                    Atom {
                        relation: 0,
                        terms: vec![Term::Var(0), Term::Wildcard, Term::Var(1)],
                    },
                ],
                bindings: vec![Binding {
                    variable: 2,
                    generator: None,
                    expression: Expr::Binary(
                        Arithmetic::Add,
                        Box::new(Expr::Var(0)),
                        Box::new(Expr::Var(1)),
                    ),
                }],
                constraints: Vec::new(),
                blocks: Vec::new(),
                guards: vec![Guard::Not(Box::new(Guard::Compare(
                    Comparison::Equal,
                    Expr::Var(2),
                    Expr::Constant(0),
                )))],
                head: vec![Expr::Var(2)],
                order: Vec::new(),
            },
            data: vec![vec![]],
        };
        assert!(case.cq().contains("r0(v0, v0, -2), r0(v0, _, v1)"));
        assert!(
            case.cq()
                .contains("let v2: i32 = (v0.to_owned() + v1.to_owned())")
        );
        assert!(case.sql().contains("a0.c1 = a0.c0 AND a0.c2 = -2"));
        assert!(case.sql().contains("(NOT ((a0.c0 + a1.c2) = 0))"));
        assert!(case.without_atom(1).is_none());
        assert!(case.without_atom(0).is_some());
    }

    #[cfg(feature = "duckdb")]
    #[test]
    fn sql_boolean_precedence_signed_remainders_and_duplicates_match_known_answers() {
        let mut case = Case {
            seed: 0,
            query: Query {
                schema: vec![Relation {
                    name: "r0".into(),
                    arity: 2,
                }],
                atoms: vec![
                    Atom {
                        relation: 0,
                        terms: vec![Term::Var(0), Term::Constant(-2)],
                    },
                    Atom {
                        relation: 0,
                        terms: vec![Term::Var(0), Term::Var(1)],
                    },
                ],
                bindings: vec![Binding {
                    variable: 2,
                    generator: None,
                    expression: Expr::Binary(
                        Arithmetic::Add,
                        Box::new(Expr::Var(0)),
                        Box::new(Expr::Var(1)),
                    ),
                }],
                constraints: Vec::new(),
                blocks: Vec::new(),
                guards: vec![
                    Guard::Or(
                        Box::new(Guard::Compare(
                            Comparison::Equal,
                            Expr::Var(0),
                            Expr::Constant(-3),
                        )),
                        Box::new(Guard::Compare(
                            Comparison::Equal,
                            Expr::Var(1),
                            Expr::Constant(-3),
                        )),
                    ),
                    Guard::Not(Box::new(Guard::Compare(
                        Comparison::Equal,
                        Expr::Var(2),
                        Expr::Constant(0),
                    ))),
                ],
                head: vec![
                    Expr::Binary(
                        Arithmetic::Remainder,
                        Box::new(Expr::Var(2)),
                        Box::new(Expr::Constant(3)),
                    ),
                    Expr::Var(0),
                ],
                order: Vec::new(),
            },
            data: vec![vec![
                vec![-3, -2],
                vec![-3, 2],
                vec![-3, 2],
                vec![2, -2],
                vec![2, -3],
                vec![0, -2],
            ]],
        };
        assert_eq!(
            crate::duckdb::evaluate_fuzz(&case).unwrap(),
            vec![vec![-2, -3], vec![-1, -3], vec![-1, 2]]
        );
        case.data[0].clear();
        assert!(crate::duckdb::evaluate_fuzz(&case).unwrap().is_empty());
    }
}
