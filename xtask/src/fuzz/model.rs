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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub variable: usize,
    pub expression: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub schema: Vec<Relation>,
    pub atoms: Vec<Atom>,
    pub bindings: Vec<Binding>,
    pub guards: Vec<Guard>,
    pub head: Vec<Expr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    pub seed: u64,
    pub query: Query,
    /// Schema order; raw duplicates are deliberately retained.
    pub data: Vec<Vec<Vec<i32>>>,
}

impl Case {
    pub fn output_arity(&self) -> usize {
        self.query.head.len()
    }

    /// Emit a full macro input, with explicitly typed computed output columns.
    pub fn cq(&self) -> String {
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
        let mut clauses = Vec::new();
        for atom in &self.query.atoms {
            let terms = atom
                .terms
                .iter()
                .map(|term| match term {
                    Term::Var(variable) => format!("v{variable}"),
                    Term::Constant(value) => value.to_string(),
                    Term::Wildcard => "_".to_owned(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            clauses.push(format!(
                "{}({terms})",
                self.query.schema[atom.relation].name
            ));
        }
        for binding in &self.query.bindings {
            clauses.push(format!(
                "let v{}: i32 = {}",
                binding.variable,
                rust_expression(&binding.expression)
            ));
        }
        for guard in &self.query.guards {
            clauses.push(format!("if {}", rust_guard(guard)));
        }
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
            clauses.join(", ")
        )
        .unwrap();
        source
    }

    /// Emit SQL directly from abstract atoms and expressions. Raw duplicates
    /// need no input deduplication: positive CQs with a DISTINCT output have
    /// set semantics, including self-joins and existential wildcard columns.
    pub fn sql(&self) -> String {
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
        if self.query.atoms.len() <= 1 || index >= self.query.atoms.len() {
            return None;
        }
        let mut candidate = self.clone();
        candidate.query.atoms.remove(index);
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

fn sql_expression(expression: &Expr, environment: &BTreeMap<usize, String>) -> String {
    match expression {
        Expr::Var(variable) => environment[variable].clone(),
        Expr::Constant(value) => value.to_string(),
        Expr::Negate(inner) => format!("(-({}))", sql_expression(inner, environment)),
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

fn sql_guard(guard: &Guard, environment: &BTreeMap<usize, String>) -> String {
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
            Self::Negate(inner) => inner.variables(output),
        }
    }

    fn has_arithmetic(&self) -> bool {
        matches!(self, Self::Binary(..) | Self::Negate(_))
    }

    fn value(&self, environment: &[i32]) -> i32 {
        match self {
            Self::Var(variable) => environment[*variable],
            Self::Constant(value) => *value,
            Self::Negate(inner) => -inner.value(environment),
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
                left.features(output);
                right.features(output);
            }
            Self::Not(inner) => {
                output.insert("boolean-not".into());
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
    fn well_bound(&self) -> bool {
        let mut bound = self
            .atoms
            .iter()
            .flat_map(atom_variables)
            .collect::<BTreeSet<_>>();
        for binding in &self.bindings {
            let mut used = BTreeSet::new();
            binding.expression.variables(&mut used);
            if !used.is_subset(&bound) || !bound.insert(binding.variable) {
                return false;
            }
        }
        let mut used = BTreeSet::new();
        for guard in &self.guards {
            guard.variables(&mut used);
        }
        for expression in &self.head {
            expression.variables(&mut used);
        }
        used.is_subset(&bound)
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
            Term::Constant(_) | Term::Wildcard => None,
        })
        .collect()
}

/// SplitMix64 with explicit wrapping arithmetic gives stable cases on all hosts
/// and does not add a dependency or depend on Rust's randomized hash state.
struct Random(u64);

impl Random {
    fn pick(&mut self, alternatives: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((value ^ (value >> 31)) % alternatives as u64) as usize
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

    fn guard(&mut self, variables: usize) -> Guard {
        let left = self.comparison(variables);
        match self.pick(4) {
            0 => left,
            1 => Guard::Not(Box::new(left)),
            2 => Guard::And(Box::new(left), Box::new(self.comparison(variables))),
            _ => Guard::Or(Box::new(left), Box::new(self.comparison(variables))),
        }
    }
}

pub fn generate(seed: u64) -> Case {
    let mut random = Random(seed);
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
            head,
        },
        data,
    }
}

/// Counts cases with each property, rather than counting syntax occurrences.
/// Histograms and feature counts overlap intentionally.
#[derive(Default)]
pub struct Coverage {
    counts: BTreeMap<String, usize>,
}

impl Coverage {
    pub fn observe(&mut self, case: &Case) {
        let query = &case.query;
        let mut features = BTreeSet::from([
            "cases".to_owned(),
            format!("atoms={}", query.atoms.len()),
            format!("relations={}", query.schema.len()),
            format!("head-arity={}", query.head.len()),
        ]);
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
                }
            }
        }
        if !query.guards.is_empty() {
            features.insert("guard".into());
        }
        for guard in &query.guards {
            guard.features(&mut features);
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
        if !query.bindings.is_empty() {
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
    fn seeded_cases_cover_every_required_feature_and_stay_bounded() {
        let mut coverage = Coverage::default();
        for seed in 0..2048 {
            let case = generate(seed);
            assert_eq!(case, generate(seed));
            assert!((1..=4).contains(&case.query.schema.len()));
            assert!((1..=5).contains(&case.query.atoms.len()));
            assert!(case.query.well_bound());
            for (relation, rows) in case.query.schema.iter().zip(&case.data) {
                assert!((1..=3).contains(&relation.arity));
                assert!(rows.iter().all(|row| row.len() == relation.arity));
                assert!(rows.iter().flatten().all(|value| (-3..=3).contains(value)));
            }
            coverage.observe(&case);
        }
        for feature in [
            "self-join",
            "constant",
            "negative-constant",
            "repeated-variable",
            "wildcard",
            "guard",
            "arithmetic-guard",
            "boolean-guard",
            "arithmetic-let",
            "computed-head",
            "repeated-head-variable",
            "disconnected",
            "empty-relation",
            "empty-used-relation",
            "duplicate-rows",
            "comparison-equal",
            "comparison-not-equal",
            "comparison-less",
            "comparison-less-equal",
            "comparison-greater",
            "comparison-greater-equal",
            "boolean-and",
            "boolean-or",
            "boolean-not",
            "expression-add",
            "expression-subtract",
            "expression-multiply",
            "expression-remainder",
            "expression-negate",
        ] {
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
                    expression: Expr::Binary(
                        Arithmetic::Add,
                        Box::new(Expr::Var(0)),
                        Box::new(Expr::Var(1)),
                    ),
                }],
                guards: vec![Guard::Not(Box::new(Guard::Compare(
                    Comparison::Equal,
                    Expr::Var(2),
                    Expr::Constant(0),
                )))],
                head: vec![Expr::Var(2)],
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

    #[cfg(feature = "oracle")]
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
                    expression: Expr::Binary(
                        Arithmetic::Add,
                        Box::new(Expr::Var(0)),
                        Box::new(Expr::Var(1)),
                    ),
                }],
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
            super::super::reference(&case).unwrap(),
            vec![vec![-2, -3], vec![-1, -3], vec![-1, 2]]
        );
        case.data[0].clear();
        assert!(super::super::reference(&case).unwrap().is_empty());
    }
}
