//! Random query specifications, independent of MiniLinq's syntax and IR.
//!
//! The Rust and SQL printers each consume this model directly. Neither printer
//! calls the other, parses its output, or uses compiler normalization/planning.
//! `Case::reference` (in `reference.rs`) interprets the model directly.

use crate::values::{ColumnType, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write};

/// A relation of typed columns. `key` lists declared key columns in declared
/// order (possibly unsorted); empty means no declared key. Generated data is
/// unique on a declared key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub name: String,
    pub types: Vec<ColumnType>,
    pub key: Vec<usize>,
}

impl Relation {
    /// An undeclared-key relation of `arity` i32 columns.
    pub fn i32(name: impl Into<String>, arity: usize) -> Self {
        Self {
            name: name.into(),
            types: vec![ColumnType::I32; arity],
            key: Vec::new(),
        }
    }

    pub fn arity(&self) -> usize {
        self.types.len()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    Var(usize),
    Constant(Value),
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

/// i32 Rust expressions. `Parameter` is a closed host capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Var(usize),
    Constant(i32),
    Parameter(i32),
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
    Negation(usize),
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

impl Comparison {
    pub(super) fn holds<T: Ord>(self, left: &T, right: &T) -> bool {
        match self {
            Self::Equal => left == right,
            Self::NotEqual => left != right,
            Self::Less => left < right,
            Self::LessEqual => left <= right,
            Self::Greater => left > right,
            Self::GreaterEqual => left >= right,
        }
    }

    fn negated(self) -> Self {
        match self {
            Self::Equal => Self::NotEqual,
            Self::NotEqual => Self::Equal,
            Self::Less => Self::GreaterEqual,
            Self::LessEqual => Self::Greater,
            Self::Greater => Self::LessEqual,
            Self::GreaterEqual => Self::Less,
        }
    }

    fn rust(self) -> &'static str {
        match self {
            Self::Equal => "==",
            Self::NotEqual => "!=",
            Self::Less => "<",
            Self::LessEqual => "<=",
            Self::Greater => ">",
            Self::GreaterEqual => ">=",
        }
    }

    fn sql(self) -> &'static str {
        match self {
            Self::Equal => "=",
            Self::NotEqual => "<>",
            Self::Less => "<",
            Self::LessEqual => "<=",
            Self::Greater => ">",
            Self::GreaterEqual => ">=",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Guard {
    Compare(Comparison, Expr, Expr),
    And(Box<Guard>, Box<Guard>),
    Or(Box<Guard>, Box<Guard>),
    Not(Box<Guard>),
}

/// Relational operands are query columns, literals, or closed host captures.
/// Capture names are never query binders. Operands of one constraint share a
/// column type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelTerm {
    Var(usize),
    Literal(Value),
    Parameter(Value),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Relational {
    Compare(Comparison, RelTerm, RelTerm),
    Between(RelTerm, RelTerm, RelTerm),
    In(RelTerm, Vec<Value>),
    Or(Vec<Vec<Relational>>),
}

/// The host capture name of a parameter value; distinct per type and value.
pub fn parameter_name(value: &Value) -> String {
    let (sign, magnitude) = match value.as_i128() {
        Some(v) => (
            if v < 0 { "negative_" } else { "positive_" },
            v.unsigned_abs(),
        ),
        None => panic!("string parameters are not generated"),
    };
    match value.column_type() {
        ColumnType::I32 => format!("parameter_{sign}{magnitude}"),
        other => format!("parameter_{sign}{magnitude}_{}", other.rust()),
    }
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
            Self::Between(_, low, high) => {
                output.insert("relational-between".into());
                if let (
                    RelTerm::Literal(l) | RelTerm::Parameter(l),
                    RelTerm::Literal(h) | RelTerm::Parameter(h),
                ) = (low, high)
                {
                    if l == h {
                        output.insert("one-point-range".into());
                    } else if l > h {
                        output.insert("empty-range".into());
                    }
                }
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

    fn rust(self, query: &Query, expression: &Expr) -> String {
        let value = query.rust_expression(expression);
        match self {
            Self::Range => format!("(0i32..(({value}) % 4))"),
            Self::InclusiveRange => format!("(0i32..=(({value}) % 4))"),
            Self::Duplicates => format!("{{ let value: i32 = {value}; vec![value, value, 0i32] }}"),
            Self::Empty => format!("{{ let _: i32 = {value}; std::iter::empty::<i32>() }}"),
            Self::Mapped => format!("{{ let value: i32 = {value}; (0..2).map(move |_| value) }}"),
        }
    }

    /// The values the printed iterable yields, in order.
    pub(super) fn values(self, value: i32) -> Vec<i32> {
        match self {
            Self::Range => (0..value % 4).collect(),
            Self::InclusiveRange => (0..=value % 4).collect(),
            Self::Duplicates => vec![value, value, 0],
            Self::Empty => Vec::new(),
            Self::Mapped => vec![value, value],
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FoldKind {
    Count,
    Sum,
    Min,
    Max,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FoldArg {
    Var(usize),
    /// A closed host capture, e.g. `sum(parameter_positive_2)`.
    Parameter(Value),
}

/// A core aggregate head column. `Count` has no argument; the others have one.
/// Folds consume the DISTINCT set of complete body valuations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fold {
    pub kind: FoldKind,
    pub argument: Option<FoldArg>,
}

/// One declared output column type; `optional` columns are `Option<T>` and
/// rows hold the (always present) inner value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputType {
    pub column: ColumnType,
    pub optional: bool,
}

impl OutputType {
    pub fn rust(self) -> String {
        if self.optional {
            format!("Option<{}>", self.column.rust())
        } else {
            self.column.rust().to_owned()
        }
    }
}

/// Rust keywords used as raw identifiers (`r#type`) when `raw_names` is set.
pub const RAW_KEYWORDS: &[&str] = &["type", "match", "loop", "fn", "move", "where", "in", "let"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub schema: Vec<Relation>,
    pub atoms: Vec<Atom>,
    /// Negated atoms (`!R(..)`); terms are bound variables, constants or wildcards.
    pub negations: Vec<Atom>,
    pub bindings: Vec<Binding>,
    pub guards: Vec<Guard>,
    pub constraints: Vec<Relational>,
    pub blocks: Vec<Block>,
    /// Plain head columns; with folds these are the group keys (variables).
    pub head: Vec<Expr>,
    /// Aggregate head columns, printed after `head`.
    pub folds: Vec<Fold>,
    /// Empty uses atoms, negations, bindings, guards, constraints order.
    pub order: Vec<Step>,
    /// Some variables are printed as raw keyword identifiers.
    pub raw_names: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Case {
    pub seed: u64,
    pub query: Query,
    /// Schema order; raw duplicates are deliberately retained.
    pub data: Vec<Vec<Vec<Value>>>,
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

    /// Closed host captures: name to typed value.
    pub fn parameters(&self) -> BTreeMap<String, Value> {
        let mut output = BTreeMap::new();
        for term in self.query.constraints.iter().flat_map(Relational::terms) {
            if let RelTerm::Parameter(v) = term {
                output.insert(parameter_name(v), v.clone());
            }
        }
        let mut expressions = Vec::new();
        for atom in &self.query.atoms {
            for term in &atom.terms {
                if let Term::Expression(expression) = term {
                    expressions.push(expression);
                }
            }
        }
        expressions.extend(self.query.bindings.iter().map(|b| &b.expression));
        expressions.extend(&self.query.head);
        let mut values = BTreeSet::new();
        for expression in expressions {
            expression.parameters(&mut values);
        }
        for guard in &self.query.guards {
            guard.parameters(&mut values);
        }
        for value in values {
            let value = Value::I32(value);
            output.insert(parameter_name(&value), value);
        }
        for fold in &self.query.folds {
            if let Some(FoldArg::Parameter(v)) = &fold.argument {
                output.insert(parameter_name(v), v.clone());
            }
        }
        output
    }

    pub fn output_arity(&self) -> usize {
        self.query.head.len() + self.query.folds.len()
    }

    /// Declared answer column types, in output order.
    pub fn output_types(&self) -> Vec<OutputType> {
        let plain = |column| OutputType {
            column,
            optional: false,
        };
        let mut types = self
            .query
            .head
            .iter()
            .map(|expression| match expression {
                Expr::Var(v) => plain(self.query.variable_type(*v)),
                _ => plain(ColumnType::I32),
            })
            .collect::<Vec<_>>();
        for fold in &self.query.folds {
            let argument = match &fold.argument {
                Some(FoldArg::Var(v)) => self.query.variable_type(*v),
                Some(FoldArg::Parameter(v)) => v.column_type(),
                None => ColumnType::Usize,
            };
            types.push(match fold.kind {
                FoldKind::Count => plain(ColumnType::Usize),
                FoldKind::Sum => plain(argument),
                FoldKind::Min | FoldKind::Max => OutputType {
                    column: argument,
                    optional: true,
                },
            });
        }
        types
    }

    /// The data as i32 rows; panics unless every column is i32.
    pub fn i32_data(&self) -> Vec<Vec<Vec<i32>>> {
        self.data
            .iter()
            .map(|rows| {
                rows.iter()
                    .map(|row| row.iter().map(Value::as_i32).collect())
                    .collect()
            })
            .collect()
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
                relation
                    .types
                    .iter()
                    .enumerate()
                    .map(|(column, ty)| format!("c{column}: {}", ty.rust()))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .unwrap();
        }
        writeln!(
            source,
            "relation answer({});",
            self.output_types()
                .iter()
                .enumerate()
                .map(|(column, ty)| format!("c{column}: {}", ty.rust()))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .unwrap();
        let clauses = self.query.cq_steps(&self.query.body_order());
        let head = self
            .query
            .head
            .iter()
            .map(|expression| match expression {
                Expr::Var(variable) => self.query.name(*variable),
                _ => self.query.rust_expression(expression),
            })
            .chain(self.query.folds.iter().map(|fold| {
                let argument = match &fold.argument {
                    None => String::new(),
                    Some(FoldArg::Var(v)) => self.query.name(*v),
                    Some(FoldArg::Parameter(v)) => parameter_name(v),
                };
                let name = match fold.kind {
                    FoldKind::Count => "count",
                    FoldKind::Sum => "sum",
                    FoldKind::Min => "min",
                    FoldKind::Max => "max",
                };
                format!("{name}({argument})")
            }))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(source, "answer({head}) :- {clauses};").unwrap();
        source
    }

    /// Emit SQL directly from abstract atoms and expressions. Raw duplicates
    /// need no input deduplication: CQs with a DISTINCT output have set
    /// semantics, including self-joins and existential wildcard columns.
    /// Aggregates fold a DISTINCT valuation subquery.
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
                    Term::Constant(value) => {
                        conditions.push(format!("{field} = {}", sql_value(value)))
                    }
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
        for negation in &self.query.negations {
            conditions.push(sql_negation(negation, &environment, None));
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
        let from = from.join(" CROSS JOIN ");
        let filter = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}\n", conditions.join(" AND "))
        };
        let order = (1..=self.output_arity())
            .map(|position| position.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        if !self.query.folds.is_empty() {
            // The DISTINCT valuation: every named binder.
            let valuation = environment
                .iter()
                .map(|(v, expression)| format!("{expression} AS v{v}"))
                .collect::<Vec<_>>()
                .join(", ");
            let mut columns = self
                .query
                .head
                .iter()
                .map(|expression| match expression {
                    Expr::Var(v) => format!("v{v}"),
                    _ => unreachable!("group keys are variables"),
                })
                .collect::<Vec<_>>();
            let keys = columns.join(", ");
            for fold in &self.query.folds {
                let ty = self.output_types()[columns.len()].column.sql();
                columns.push(match (&fold.kind, &fold.argument) {
                    (FoldKind::Count, _) => format!("CAST(COUNT(*) AS {ty})"),
                    (FoldKind::Sum, Some(FoldArg::Var(v))) => format!("CAST(SUM(v{v}) AS {ty})"),
                    (FoldKind::Sum, Some(FoldArg::Parameter(p))) => {
                        format!("CAST(COUNT(*) * {} AS {ty})", sql_value(p))
                    }
                    (FoldKind::Min, Some(FoldArg::Var(v))) => format!("MIN(v{v})"),
                    (FoldKind::Max, Some(FoldArg::Var(v))) => format!("MAX(v{v})"),
                    (FoldKind::Min | FoldKind::Max, Some(FoldArg::Parameter(p))) => sql_value(p),
                    (_, None) => unreachable!("only count has no argument"),
                });
            }
            let group = if keys.is_empty() {
                String::new()
            } else {
                format!(" GROUP BY {keys}")
            };
            return format!(
                "SELECT {}\nFROM (SELECT DISTINCT {valuation}\nFROM {from}\n{filter}) AS valuation{group}\nORDER BY {order};\n",
                columns.join(", ")
            );
        }
        let projection = self
            .query
            .head
            .iter()
            .enumerate()
            .map(|(column, expression)| match expression {
                Expr::Var(_) => format!(
                    "{} AS out{column}",
                    sql_expression(expression, &environment)
                ),
                _ => format!(
                    "CAST({} AS INTEGER) AS out{column}",
                    sql_expression(expression, &environment)
                ),
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("SELECT DISTINCT {projection}\nFROM {from}\n{filter}ORDER BY {order};\n")
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

    /// Every property of this case, each counted once (see `Coverage`).
    pub fn features(&self) -> BTreeSet<String> {
        let query = &self.query;
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
            features.insert(format!("relation-arity={}", relation.arity()));
            for ty in &relation.types {
                if *ty != ColumnType::I32 {
                    features.insert(format!("type-{}", ty.sql().to_lowercase()));
                }
            }
            if !relation.key.is_empty() {
                features.insert("declared-key".into());
                if relation.key.len() > 1 {
                    features.insert("multi-column-key".into());
                }
                if relation.key.windows(2).any(|pair| pair[0] > pair[1]) {
                    features.insert("unsorted-key".into());
                }
            }
            if relation.name.starts_with("r#") {
                features.insert("raw-identifier".into());
            }
        }
        if query.raw_names {
            features.insert("raw-identifier".into());
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
                        if value.as_i128().is_some_and(|v| v < 0) {
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
        for negation in &query.negations {
            features.insert("negation".into());
            if query.atoms.iter().any(|a| a.relation == negation.relation) {
                features.insert("negation-self".into());
            }
            for term in &negation.terms {
                features.insert(
                    match term {
                        Term::Var(_) => "negation-variable",
                        Term::Constant(_) => "negation-constant",
                        Term::Wildcard => "negation-wildcard",
                        Term::Expression(_) => unreachable!("negated atoms have no expressions"),
                    }
                    .into(),
                );
            }
        }
        for c in &query.constraints {
            c.features(&mut features);
        }
        if query.contradictory() {
            features.insert("contradictory-constraints".into());
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
            if guard.is_closed() {
                features.insert("closed-guard".into());
            }
        }
        for binding in &query.bindings {
            let mut own = BTreeSet::new();
            binding.expression.features(&mut own);
            if binding.generator.is_none() && binding.expression.is_closed() {
                features.insert("closed-let".into());
            }
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
            if expression.is_closed() {
                features.insert("head-constant".into());
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
        for fold in &query.folds {
            features.insert("aggregate".into());
            features.insert(
                match (fold.kind, &fold.argument) {
                    (FoldKind::Count, _) => "fold-count",
                    (FoldKind::Sum, Some(FoldArg::Parameter(_))) => "fold-sum-capture",
                    (FoldKind::Sum, _) => "fold-sum",
                    (FoldKind::Min, _) => "fold-min",
                    (FoldKind::Max, _) => "fold-max",
                }
                .into(),
            );
        }
        if !query.atoms.is_empty() && query.disconnected() {
            features.insert("disconnected".into());
        }
        if self.data.iter().any(Vec::is_empty) {
            features.insert("empty-relation".into());
        }
        if query
            .atoms
            .iter()
            .any(|atom| self.data[atom.relation].is_empty())
        {
            features.insert("empty-used-relation".into());
        }
        if self
            .data
            .iter()
            .any(|rows| rows.iter().collect::<BTreeSet<_>>().len() != rows.len())
        {
            features.insert("duplicate-rows".into());
        }
        for rows in &self.data {
            features.extend(super::data::shape_features(rows));
        }
        features.extend(super::data::domain_features(&self.data));
        features
    }
}

/// Query-text literal: i32 stays unsuffixed (legacy spelling); other integer
/// types carry their suffix so captures and literals are typed.
fn rust_value(value: &Value) -> String {
    match value {
        Value::I32(v) => v.to_string(),
        other => other.rust_literal(),
    }
}

fn sql_value(value: &Value) -> String {
    match value {
        Value::I32(v) => v.to_string(),
        other => other.sql_literal(),
    }
}

impl Query {
    /// The printed binder name of a variable.
    pub fn name(&self, variable: usize) -> String {
        if self.raw_names && variable % 2 == 1 {
            if let Some(keyword) = RAW_KEYWORDS.get(variable / 2) {
                return format!("r#{keyword}");
            }
        }
        format!("v{variable}")
    }

    /// The column type of a variable: its first atom column, else i32
    /// (computed lets and generators are i32).
    pub fn variable_type(&self, variable: usize) -> ColumnType {
        for atom in &self.atoms {
            for (column, term) in atom.terms.iter().enumerate() {
                if *term == Term::Var(variable) {
                    return self.schema[atom.relation].types[column];
                }
            }
        }
        ColumnType::I32
    }

    fn rust_expression(&self, expression: &Expr) -> String {
        match expression {
            // Atom values are borrowed and let values are owned. This ordinary
            // Rust operation yields i32 in either case, including after a planner
            // has materialized an intermediate relation.
            Expr::Var(variable) => format!("{}.to_owned()", self.name(*variable)),
            Expr::Constant(value) => value.to_string(),
            Expr::Parameter(value) => parameter_name(&Value::I32(*value)),
            Expr::Negate(inner) => format!("(-({}))", self.rust_expression(inner)),
            Expr::OpaqueCall(inner) => {
                format!("std::convert::identity({})", self.rust_expression(inner))
            }
            Expr::Rust(operator, inner) => {
                let value = self.rust_expression(inner);
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
                    self.rust_expression(left),
                    self.rust_expression(right)
                )
            }
        }
    }

    fn rust_guard(&self, guard: &Guard) -> String {
        match guard {
            Guard::Compare(comparison, left, right) => format!(
                "({} {} {})",
                self.rust_expression(left),
                comparison.rust(),
                self.rust_expression(right)
            ),
            Guard::And(left, right) => {
                format!("({} && {})", self.rust_guard(left), self.rust_guard(right))
            }
            Guard::Or(left, right) => {
                format!("({} || {})", self.rust_guard(left), self.rust_guard(right))
            }
            Guard::Not(inner) => format!("!({})", self.rust_guard(inner)),
        }
    }

    fn cq_constraint(&self, c: &Relational) -> String {
        let term = |t: &RelTerm| match t {
            RelTerm::Var(v) => self.name(*v),
            RelTerm::Literal(v) => rust_value(v),
            RelTerm::Parameter(v) => parameter_name(v),
        };
        match c {
            Relational::Compare(op, l, r) => format!("{} {} {}", term(l), op.rust(), term(r)),
            Relational::Between(t, l, h) => {
                format!("{} in {}..={}", term(t), term(l), term(h))
            }
            Relational::In(t, ls) => format!(
                "{} in [{}]",
                term(t),
                ls.iter().map(rust_value).collect::<Vec<_>>().join(", ")
            ),
            Relational::Or(bs) => bs
                .iter()
                .map(|b| {
                    format!(
                        "({})",
                        b.iter()
                            .map(|c| self.cq_constraint(c))
                            .collect::<Vec<_>>()
                            .join(" && ")
                    )
                })
                .collect::<Vec<_>>()
                .join(" || "),
        }
    }

    fn cq_atom(&self, atom: &Atom) -> String {
        let terms = atom
            .terms
            .iter()
            .map(|term| match term {
                Term::Var(variable) => self.name(*variable),
                Term::Constant(value) => rust_value(value),
                Term::Wildcard => "_".to_owned(),
                Term::Expression(expression) => self.rust_expression(expression),
            })
            .collect::<Vec<_>>()
            .join(", ");
        format!("{}({terms})", self.schema[atom.relation].name)
    }

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
                Step::Atom(index) => clauses.push(self.cq_atom(&self.atoms[index])),
                Step::Negation(index) => {
                    clauses.push(format!("!{}", self.cq_atom(&self.negations[index])))
                }
                Step::Binding(index) => {
                    let binding = &self.bindings[index];
                    clauses.push(if let Some(generator) = binding.generator {
                        format!(
                            "for {} in {}",
                            self.name(binding.variable),
                            generator.rust(self, &binding.expression)
                        )
                    } else {
                        format!(
                            "let {}: i32 = {}",
                            self.name(binding.variable),
                            self.rust_expression(&binding.expression)
                        )
                    });
                }
                Step::Constraint(index) => clauses.push(format!(
                    "where {}",
                    self.cq_constraint(&self.constraints[index])
                )),
                Step::Guard(index) => {
                    clauses.push(format!("if {}", self.rust_guard(&self.guards[index])));
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
            .chain((0..self.negations.len()).map(Step::Negation))
            .chain((0..self.bindings.len()).map(Step::Binding))
            .chain((0..self.guards.len()).map(Step::Guard))
            .chain((0..self.constraints.len()).map(Step::Constraint))
            .collect()
    }

    pub(super) fn well_bound(&self) -> bool {
        let mut bound = BTreeSet::new();
        if !self.steps_bound(&self.body_order(), &mut bound) {
            return false;
        }
        let mut used = BTreeSet::new();
        for expression in &self.head {
            expression.variables(&mut used);
        }
        for fold in &self.folds {
            if let Some(FoldArg::Var(v)) = fold.argument {
                used.insert(v);
            }
        }
        used.is_subset(&bound)
            && (self.folds.is_empty() || self.head.iter().all(|e| matches!(e, Expr::Var(_))))
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
                Step::Negation(index) => {
                    let atom = &self.negations[index];
                    if !atom_variables(atom).is_subset(bound)
                        || atom.terms.iter().any(|t| matches!(t, Term::Expression(_)))
                    {
                        return false;
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

    /// Two top-level literal comparisons on one variable that no value meets,
    /// e.g. `x < 2 && x == 3`.
    fn contradictory(&self) -> bool {
        let literal = |c: &Relational| match c {
            Relational::Compare(op, RelTerm::Var(v), RelTerm::Literal(l)) => {
                Some((*op, *v, l.clone()))
            }
            _ => None,
        };
        let comparisons = self
            .constraints
            .iter()
            .filter_map(literal)
            .collect::<Vec<_>>();
        comparisons.iter().any(|(op, v, point)| {
            *op == Comparison::Equal
                && comparisons
                    .iter()
                    .any(|(other, w, bound)| v == w && !other.holds(point, bound))
        })
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

/// The SQL printer independently interprets abstract relational nodes.
pub(super) fn sql_constraint(c: &Relational, environment: &BTreeMap<usize, String>) -> String {
    let term = |t: &RelTerm| match t {
        RelTerm::Var(v) => environment[v].clone(),
        RelTerm::Literal(v) | RelTerm::Parameter(v) => sql_value(v),
    };
    match c {
        Relational::Compare(op, l, r) => format!("({} {} {})", term(l), op.sql(), term(r)),
        Relational::Between(t, l, h) => {
            format!("({} BETWEEN {} AND {})", term(t), term(l), term(h))
        }
        Relational::In(t, ls) => format!(
            "({} IN ({}))",
            term(t),
            ls.iter().map(sql_value).collect::<Vec<_>>().join(", ")
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

/// `NOT EXISTS` over the raw negated table; wildcards are unconstrained.
pub(super) fn sql_negation(
    atom: &Atom,
    environment: &BTreeMap<usize, String>,
    _prefix: Option<&str>,
) -> String {
    let conditions = atom
        .terms
        .iter()
        .enumerate()
        .filter_map(|(column, term)| match term {
            Term::Var(v) => Some(format!("n.c{column} = {}", environment[v])),
            Term::Constant(value) => Some(format!("n.c{column} = {}", sql_value(value))),
            Term::Wildcard => None,
            Term::Expression(_) => unreachable!("negated atoms have no expressions"),
        })
        .collect::<Vec<_>>();
    let filter = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };
    format!(
        "(NOT EXISTS (SELECT 1 FROM \"mini-linq/raw/{}\" n{filter}))",
        atom.relation
    )
}

pub(super) fn sql_expression(expression: &Expr, environment: &BTreeMap<usize, String>) -> String {
    match expression {
        Expr::Rust(..) => unreachable!("opaque Rust operations use the compiled reference"),
        Expr::Var(variable) => environment[variable].clone(),
        Expr::Constant(value) | Expr::Parameter(value) => value.to_string(),
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
        Guard::Compare(comparison, left, right) => format!(
            "({} {} {})",
            sql_expression(left, environment),
            comparison.sql(),
            sql_expression(right, environment)
        ),
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
            Self::Parameter(_) => {
                output.insert("closed-capture".into());
            }
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

    pub(super) fn variables(&self, output: &mut BTreeSet<usize>) {
        match self {
            Self::Var(variable) => {
                output.insert(*variable);
            }
            Self::Constant(_) | Self::Parameter(_) => (),
            Self::Binary(_, left, right) => {
                left.variables(output);
                right.variables(output);
            }
            Self::Negate(inner) | Self::Rust(_, inner) | Self::OpaqueCall(inner) => {
                inner.variables(output)
            }
        }
    }

    fn parameters(&self, output: &mut BTreeSet<i32>) {
        match self {
            Self::Parameter(value) => {
                output.insert(*value);
            }
            Self::Var(_) | Self::Constant(_) => (),
            Self::Binary(_, left, right) => {
                left.parameters(output);
                right.parameters(output);
            }
            Self::Negate(inner) | Self::Rust(_, inner) | Self::OpaqueCall(inner) => {
                inner.parameters(output)
            }
        }
    }

    /// Closed: no query variable occurs (a query-closed capture site).
    pub fn is_closed(&self) -> bool {
        let mut variables = BTreeSet::new();
        self.variables(&mut variables);
        variables.is_empty()
    }

    fn has_arithmetic(&self) -> bool {
        matches!(self, Self::Binary(..) | Self::Negate(_) | Self::Rust(..))
    }

    /// Generation-time i32 value over a dense witness; the reference uses its
    /// own evaluator.
    fn value(&self, environment: &[i32]) -> i32 {
        match self {
            Self::Rust(..) => unreachable!("opaque Rust operations use the compiled reference"),
            Self::Var(variable) => environment[*variable],
            Self::Constant(value) | Self::Parameter(value) => *value,
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

    pub(super) fn variables(&self, output: &mut BTreeSet<usize>) {
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

    fn parameters(&self, output: &mut BTreeSet<i32>) {
        match self {
            Self::Compare(_, left, right) => {
                left.parameters(output);
                right.parameters(output);
            }
            Self::And(left, right) | Self::Or(left, right) => {
                left.parameters(output);
                right.parameters(output);
            }
            Self::Not(inner) => inner.parameters(output),
        }
    }

    /// Closed: no query variable occurs.
    pub fn is_closed(&self) -> bool {
        let mut variables = BTreeSet::new();
        self.variables(&mut variables);
        variables.is_empty()
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
                comparison.holds(&left.value(environment), &right.value(environment))
            }
            Self::And(left, right) => left.value(environment) && right.value(environment),
            Self::Or(left, right) => left.value(environment) || right.value(environment),
            Self::Not(inner) => !inner.value(environment),
        }
    }
}

/// SplitMix64 with explicit wrapping arithmetic gives stable cases on all hosts
/// and does not add a dependency or depend on Rust's randomized hash state.
struct Random(crate::random::SplitMix64);

impl Random {
    fn pick(&mut self, alternatives: usize) -> usize {
        (self.0.next_u64() % alternatives as u64) as usize
    }

    fn choose<T: Clone>(&mut self, values: &[T]) -> T {
        values[self.pick(values.len())].clone()
    }

    fn small(&mut self) -> i32 {
        self.pick(7) as i32 - 3
    }

    fn comparison(&mut self) -> Comparison {
        match self.pick(6) {
            0 => Comparison::Equal,
            1 => Comparison::NotEqual,
            2 => Comparison::Less,
            3 => Comparison::LessEqual,
            4 => Comparison::Greater,
            _ => Comparison::GreaterEqual,
        }
    }

    /// An i32 expression over the arithmetic-safe `variables` (nonempty).
    fn expression(&mut self, variables: &[usize], depth: usize) -> Expr {
        if depth == 0 {
            return if self.pick(4) == 0 {
                Expr::Constant(self.small())
            } else {
                Expr::Var(self.choose(variables))
            };
        }
        match self.pick(7) {
            0 => Expr::Var(self.choose(variables)),
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

    /// Relational terms over one typed variable; literals and captures come
    /// from that variable's domain.
    fn relational_term(&mut self, variable: &TypedVar, others: &[TypedVar]) -> RelTerm {
        let peers = others
            .iter()
            .filter(|other| other.ty == variable.ty)
            .collect::<Vec<_>>();
        match self.pick(4) {
            0 => RelTerm::Literal(self.literal(variable)),
            1 => RelTerm::Parameter(self.literal(variable)),
            _ if !peers.is_empty() => RelTerm::Var(peers[self.pick(peers.len())].index),
            _ => RelTerm::Literal(self.literal(variable)),
        }
    }

    fn literal(&mut self, variable: &TypedVar) -> Value {
        let values = variable
            .domain
            .values(variable.ty)
            .into_iter()
            .filter(super::data::printable)
            .collect::<Vec<_>>();
        self.choose(&values)
    }

    fn relational_atomic(&mut self, variables: &[TypedVar]) -> Relational {
        let variable = self.choose(variables);
        let v = RelTerm::Var(variable.index);
        match self.pick(8) {
            6 => Relational::Between(
                v,
                self.relational_term(&variable, variables),
                self.relational_term(&variable, variables),
            ),
            7 => Relational::In(
                v,
                (0..1 + self.pick(4))
                    .map(|_| self.literal(&variable))
                    .collect(),
            ),
            _ => Relational::Compare(
                self.comparison(),
                v,
                self.relational_term(&variable, variables),
            ),
        }
    }

    fn relational(&mut self, variables: &[TypedVar]) -> Relational {
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

    fn guard(&mut self, variables: &[usize]) -> Guard {
        let relational = Guard::Compare(
            self.comparison(),
            Expr::Var(self.choose(variables)),
            Expr::Constant(self.small()),
        );
        let opaque = if self.pick(2) == 0 {
            Guard::Compare(
                Comparison::Less,
                Expr::OpaqueCall(Box::new(Expr::Var(self.choose(variables)))),
                Expr::Constant(self.small()),
            )
        } else {
            Guard::Compare(
                Comparison::Greater,
                Expr::Binary(
                    Arithmetic::Add,
                    Box::new(Expr::Var(self.choose(variables))),
                    Box::new(Expr::Constant(self.small())),
                ),
                Expr::Constant(self.small()),
            )
        };
        match self.pick(7) {
            0 => Guard::Compare(
                self.comparison(),
                self.expression(variables, 1),
                self.expression(variables, 1),
            ),
            1 => Guard::Not(Box::new(relational)),
            2 => Guard::And(Box::new(relational), Box::new(opaque)),
            3 => Guard::And(Box::new(opaque), Box::new(relational)),
            4 => Guard::Or(Box::new(relational), Box::new(opaque)),
            5 => Guard::Or(Box::new(opaque), Box::new(relational)),
            _ => Guard::Not(Box::new(Guard::And(Box::new(relational), Box::new(opaque)))),
        }
    }
}

/// A generation-time variable: index, type and the domain its values come from.
#[derive(Clone, Copy, Debug)]
struct TypedVar {
    index: usize,
    ty: ColumnType,
    domain: super::data::Domain,
}

/// Generate a reproducible case under the full CPU profile.
pub fn generate(seed: u64) -> Case {
    generate_with(seed, &super::Profile::full())
}

/// Generate a reproducible case using only the features `profile` enables.
///
/// With opaque Rust and generators enabled, even seeds sample total opaque Rust
/// (compiled oracle lane); seeds ending in 3 or 8 sample blocks. Other seeds
/// sample typed relational queries: 1..=4 tables of arity 1..=3, 1..=5 atoms,
/// typed columns, shaped data, and a planted witness valuation.
///
/// Most answers should be nonempty (G2): one case in eight keeps its first
/// draw whatever the answer; the others redraw the seed's random stream (the
/// seed-determined lane, operator and block shape stay fixed) up to six times
/// until the reference answer is nonempty, repairing each draw's data first.
pub fn generate_with(seed: u64, profile: &super::Profile) -> Case {
    let accept_empty = Random(crate::random::SplitMix64::new(seed ^ 0x6163_6365_7074)).pick(8) == 0;
    let mut last = None;
    for attempt in 0..6u64 {
        let stream = attempt.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let mut case = if profile.blocks && matches!(seed % 10, 3 | 8) {
            generate_block(seed, stream, profile.opaque_rust)
        } else if profile.opaque_rust && profile.generators && seed & 1 == 0 {
            generate_rust(seed, stream)
        } else {
            generate_relational(seed, stream, profile)
        };
        debug_assert!(case.query.well_bound());
        if accept_empty {
            return case;
        }
        repair_witness(&mut case, stream);
        if !case.reference().is_empty() {
            return case;
        }
        last = Some(case);
    }
    last.expect("at least one attempt")
}

/// When a draw's data gives no answer, try up to 32 random additions of one
/// row per used relation, drawn from each column's present values (plus, for
/// total opaque Rust, small integers and the query's own expression and
/// generator values); keep the first that yields an answer.
fn repair_witness(case: &mut Case, stream: u64) {
    let mut random = Random(crate::random::SplitMix64::new(
        case.seed ^ stream ^ 0x7769_746e_6573_73,
    ));
    if !case.reference().is_empty() {
        return;
    }
    let mut base = (-3..=3).collect::<BTreeSet<i32>>();
    for value in case.data.iter().flatten().flatten() {
        if let Value::I32(v) = value {
            base.insert(*v);
        }
    }
    let mut expressions = Vec::new();
    for atom in &case.query.atoms {
        for term in &atom.terms {
            if let Term::Expression(e) = term {
                expressions.push(e);
            }
        }
    }
    expressions.extend(case.query.bindings.iter().map(|b| &b.expression));
    let width = case.query.variable_count();
    let mut derived = base.clone();
    for v in &base {
        let env = vec![Some(Value::I32(*v)); width];
        for e in &expressions {
            if let Some(value) = super::reference::checked_expression(e, &env) {
                derived.insert(value);
                for binding in &case.query.bindings {
                    if let Some(generator) = binding.generator {
                        derived.extend(generator.values(value));
                    }
                }
            }
        }
    }
    let derived = derived.into_iter().map(Value::I32).collect::<Vec<_>>();
    let used = case
        .query
        .atoms
        .iter()
        .map(|atom| atom.relation)
        .collect::<BTreeSet<_>>();
    let rust_oracle = case.requires_rust_oracle();
    let columns = case
        .query
        .schema
        .iter()
        .zip(&case.data)
        .map(|(relation, rows)| {
            (0..relation.arity())
                .map(|column| {
                    let ty = relation.types[column];
                    let mut values = rows
                        .iter()
                        .map(|row| row[column].clone())
                        .collect::<BTreeSet<_>>();
                    // Total opaque Rust tolerates any i32; the SQL lanes keep
                    // each column's own (arithmetic-safe) values.
                    if ty == ColumnType::I32 && rust_oracle {
                        values.extend(derived.iter().cloned());
                    }
                    if values.is_empty() {
                        values.extend(super::data::Domain::Small.values(ty));
                    }
                    values.into_iter().collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for _ in 0..32 {
        let mut candidate = case.clone();
        for relation in &used {
            let schema = &case.query.schema[*relation];
            let row = columns[*relation]
                .iter()
                .map(|values| random.choose(values))
                .collect::<Vec<_>>();
            let key = |row: &Vec<Value>| {
                schema
                    .key
                    .iter()
                    .map(|c| row[*c].clone())
                    .collect::<Vec<_>>()
            };
            if !schema.key.is_empty()
                && candidate.data[*relation]
                    .iter()
                    .any(|r| key(r) == key(&row))
            {
                continue;
            }
            candidate.data[*relation].push(row);
        }
        if !candidate.reference().is_empty() {
            *case = candidate;
            return;
        }
    }
}

fn generate_relational(seed: u64, stream: u64, profile: &super::Profile) -> Case {
    use super::data::{Domain, Shape};
    let mut random = Random(crate::random::SplitMix64::new(
        seed ^ stream ^ 0x7479_7065_6400,
    ));
    let raw = profile.raw_identifiers && random.pick(5) == 0;
    let relations = 1 + random.pick(4);
    let mut schema = Vec::new();
    let mut domains = Vec::new();
    let mut shapes = Vec::new();
    for index in 0..relations {
        let arity = 1 + random.pick(3);
        let types = (0..arity)
            .map(|_| {
                let choice = random.pick(10);
                if profile.wide_ints && choice < 2 {
                    ColumnType::I64
                } else if profile.wide_ints && choice == 2 {
                    ColumnType::U32
                } else if profile.strings && choice == 3 {
                    ColumnType::Str
                } else {
                    ColumnType::I32
                }
            })
            .collect::<Vec<_>>();
        let key = if profile.keys && arity >= 2 && random.pick(3) == 0 {
            let mut columns = (0..arity).collect::<Vec<_>>();
            for i in (1..columns.len()).rev() {
                columns.swap(i, random.pick(i + 1));
            }
            columns.truncate(1 + random.pick(arity - 1));
            columns
        } else {
            Vec::new()
        };
        let shape = if profile.max_rows > 1024 && random.pick(12) == 0 {
            Shape::Large
        } else {
            [
                Shape::Empty,
                Shape::Single,
                Shape::AllDuplicates,
                Shape::OneKey,
                Shape::Distinct,
                Shape::Random,
                Shape::Random,
                Shape::Random,
                Shape::Random,
            ][random.pick(9)]
        };
        let rows = if shape == Shape::Large {
            1025 + random.pick(profile.max_rows - 1024)
        } else {
            0
        };
        domains.push(
            types
                .iter()
                .enumerate()
                .map(|(column, ty)| match ty {
                    ColumnType::I32 | ColumnType::I64 | ColumnType::U32
                        if shape == Shape::Large && column == 0 =>
                    {
                        Domain::Sequential(rows)
                    }
                    ColumnType::I32 | ColumnType::U32 => {
                        if random.pick(5) == 0 {
                            Domain::Extreme
                        } else {
                            Domain::Small
                        }
                    }
                    ColumnType::I64 => {
                        [Domain::Wide, Domain::Wide, Domain::Extreme, Domain::Small][random.pick(4)]
                    }
                    ColumnType::Str | ColumnType::Usize => Domain::Small,
                })
                .collect::<Vec<_>>(),
        );
        shapes.push((shape, rows));
        schema.push(Relation {
            name: if raw && index < 2 {
                ["r#match", "r#in"][index].to_owned()
            } else {
                format!("r{index}")
            },
            types,
            key,
        });
    }

    let count = 1 + random.pick(5);
    let mut variables: Vec<TypedVar> = Vec::new();
    let mut atoms = Vec::new();
    for index in 0..count {
        let relation = random.pick(relations);
        // Fresh variables in some later atoms deliberately sample disconnected
        // components; arbitrary reuse also creates cycles and self-joins.
        let fresh_component = index > 0 && random.pick(4) == 0;
        let mut terms = Vec::new();
        for column in 0..schema[relation].arity() {
            let ty = schema[relation].types[column];
            let domain = domains[relation][column];
            let peers = variables
                .iter()
                .filter(|v| v.ty == ty)
                .map(|v| v.index)
                .collect::<Vec<_>>();
            let choice = random.pick(10);
            let term = if peers.is_empty() || (fresh_component && column == 0) || choice < 3 {
                variables.push(TypedVar {
                    index: variables.len(),
                    ty,
                    domain,
                });
                Term::Var(variables.len() - 1)
            } else if choice < 6 && !fresh_component {
                Term::Var(random.choose(&peers))
            } else if choice < 8 {
                let values = domain
                    .values(ty)
                    .into_iter()
                    .filter(super::data::printable)
                    .collect::<Vec<_>>();
                Term::Constant(random.choose(&values))
            } else {
                Term::Wildcard
            };
            terms.push(term);
        }
        atoms.push(Atom { relation, terms });
    }
    let atom_variables = variables.clone();
    let pick_value = |random: &mut Random, domain: Domain, ty: ColumnType| match domain {
        // Row ordinals: draw directly rather than materializing the domain.
        Domain::Sequential(n) => {
            let ordinal = random.pick(n);
            match ty {
                ColumnType::I64 => Value::I64(ordinal as i64),
                ColumnType::U32 => Value::U32(ordinal as u32),
                _ => Value::I32(ordinal as i32),
            }
        }
        _ => random.choose(&domain.values(ty)),
    };
    let mut witness = variables
        .iter()
        .map(|v| pick_value(&mut random, v.domain, v.ty))
        .collect::<Vec<_>>();
    let arithmetic = |variables: &[TypedVar]| {
        variables
            .iter()
            .filter(|v| v.ty == ColumnType::I32 && v.domain.arithmetic())
            .map(|v| v.index)
            .collect::<Vec<_>>()
    };
    // Dense i32 view for generation-time expression values (0 for others).
    let dense = |witness: &[Value]| {
        witness
            .iter()
            .map(|v| match v {
                Value::I32(v) => *v,
                _ => 0,
            })
            .collect::<Vec<_>>()
    };
    let mut bindings = Vec::new();
    for _ in 0..random.pick(3) {
        let safe = arithmetic(&variables);
        // One multiplication per let and at most two lets keep every possible
        // input valuation far below i32 overflow, even for two-level heads.
        let left = if safe.is_empty() || (profile.closed_captures && random.pick(5) == 0) {
            Expr::Parameter(random.small())
        } else {
            Expr::Var(random.choose(&safe))
        };
        let expression = Expr::Binary(
            match random.pick(3) {
                0 => Arithmetic::Add,
                1 => Arithmetic::Subtract,
                _ => Arithmetic::Multiply,
            },
            Box::new(left),
            Box::new(Expr::Constant(random.small())),
        );
        let expression = if !profile.closed_captures {
            match expression {
                Expr::Binary(op, left, right) if matches!(*left, Expr::Parameter(_)) => {
                    if safe.is_empty() {
                        continue;
                    }
                    Expr::Binary(op, Box::new(Expr::Var(safe[0])), right)
                }
                other => other,
            }
        } else {
            expression
        };
        witness.push(Value::I32(expression.value(&dense(&witness))));
        bindings.push(Binding {
            variable: variables.len(),
            expression,
            generator: None,
        });
        variables.push(TypedVar {
            index: variables.len(),
            ty: ColumnType::I32,
            domain: Domain::Small,
        });
    }
    let mut preserve_witness = random.pick(8) != 0;
    let safe = arithmetic(&variables);
    let mut guards = Vec::new();
    for _ in 0..random.pick(3) {
        let guard = if safe.is_empty() || (profile.closed_captures && random.pick(8) == 0) {
            if !profile.closed_captures {
                continue;
            }
            Guard::Compare(
                random.comparison(),
                Expr::Parameter(random.small()),
                Expr::Constant(random.small()),
            )
        } else {
            random.guard(&safe)
        };
        guards.push(if preserve_witness && !guard.value(&dense(&witness)) {
            Guard::Not(Box::new(guard))
        } else {
            guard
        });
    }
    let holds = |c: &Relational, witness: &[Value]| {
        let env = witness.iter().cloned().map(Some).collect::<Vec<_>>();
        super::reference::relational_holds(c, &env)
    };
    let mut constraints = Vec::new();
    for _ in 0..1 + random.pick(3) {
        let mut constraint = random.relational(&variables);
        for _ in 0..4 {
            if !preserve_witness || holds(&constraint, &witness) {
                break;
            }
            constraint = random.relational(&variables);
        }
        if preserve_witness && !holds(&constraint, &witness) {
            constraint = match constraint {
                Relational::Compare(op, l, r) => Relational::Compare(op.negated(), l, r),
                other => {
                    let term = other.terms()[0].clone();
                    let RelTerm::Var(v) = term else {
                        unreachable!()
                    };
                    Relational::In(term, vec![witness[v].clone()])
                }
            };
        }
        constraints.push(constraint);
    }
    // Interval edge cases on one ordered variable.
    let ordered = variables
        .iter()
        .filter(|v| v.ty != ColumnType::Str && super::data::printable(&witness[v.index]))
        .copied()
        .collect::<Vec<_>>();
    if !ordered.is_empty() {
        let variable = random.choose(&ordered);
        let mut values = variable
            .domain
            .values(variable.ty)
            .into_iter()
            .filter(super::data::printable)
            .collect::<Vec<_>>();
        values.sort();
        let v = RelTerm::Var(variable.index);
        match random.pick(12) {
            0 if values.len() > 1 => {
                // `x < a && x == b` with b >= a: contradictory (c82e9abf).
                let a = random.pick(values.len() - 1);
                let b = a + random.pick(values.len() - a);
                constraints.push(Relational::Compare(
                    Comparison::Less,
                    v.clone(),
                    RelTerm::Literal(values[a].clone()),
                ));
                constraints.push(Relational::Compare(
                    Comparison::Equal,
                    v,
                    RelTerm::Literal(values[b].clone()),
                ));
                preserve_witness = false;
            }
            1 => {
                let point = witness[variable.index].clone();
                constraints.push(Relational::Between(
                    v,
                    RelTerm::Literal(point.clone()),
                    RelTerm::Parameter(point),
                ));
            }
            2 if values.len() > 1 => {
                let low = random.pick(values.len() - 1);
                constraints.push(Relational::Between(
                    v,
                    RelTerm::Literal(values[low + 1].clone()),
                    RelTerm::Literal(values[low].clone()),
                ));
                preserve_witness = false;
            }
            _ => (),
        }
    }
    let mut negations = Vec::new();
    if profile.negation && random.pick(3) == 0 {
        for _ in 0..1 + random.pick(2) {
            let relation = random.pick(relations);
            let terms = (0..schema[relation].arity())
                .map(|column| {
                    let ty = schema[relation].types[column];
                    let peers = atom_variables
                        .iter()
                        .filter(|v| v.ty == ty)
                        .map(|v| v.index)
                        .collect::<Vec<_>>();
                    match random.pick(6) {
                        0..=2 if !peers.is_empty() => Term::Var(random.choose(&peers)),
                        0..=3 => {
                            let values = domains[relation][column]
                                .values(ty)
                                .into_iter()
                                .filter(super::data::printable)
                                .collect::<Vec<_>>();
                            Term::Constant(random.choose(&values))
                        }
                        _ => Term::Wildcard,
                    }
                })
                .collect();
            negations.push(Atom { relation, terms });
        }
    }
    let mut folds = Vec::new();
    let head = if profile.aggregates && random.pick(4) == 0 {
        let keys = (0..1 + random.pick(2))
            .map(|_| Expr::Var(random.choose(&atom_variables).index))
            .collect();
        let safe = arithmetic(&atom_variables);
        for _ in 0..1 + random.pick(3) {
            folds.push(match random.pick(5) {
                0 => Fold {
                    kind: FoldKind::Count,
                    argument: None,
                },
                1 if !safe.is_empty() => Fold {
                    kind: FoldKind::Sum,
                    argument: Some(FoldArg::Var(random.choose(&safe))),
                },
                1 | 2 if profile.closed_captures => Fold {
                    kind: FoldKind::Sum,
                    argument: Some(FoldArg::Parameter(Value::I32(random.small()))),
                },
                choice => Fold {
                    kind: if choice % 2 == 0 {
                        FoldKind::Min
                    } else {
                        FoldKind::Max
                    },
                    argument: Some(FoldArg::Var(random.choose(&atom_variables).index)),
                },
            });
        }
        keys
    } else {
        let safe = arithmetic(&variables);
        (0..1 + random.pick(3))
            .map(|_| match random.pick(12) {
                0 if profile.closed_captures => Expr::Parameter(random.small()),
                1..=4 if !safe.is_empty() => random.expression(&safe, 2),
                _ => Expr::Var(random.choose(&variables).index),
            })
            .collect()
    };

    // Larger-than-one-tile tables: at most one per case, used by at most one
    // atom, so no reference cross product grows beyond |large| x |small|^k.
    let mut large_seen = false;
    for (relation, (shape, _)) in shapes.iter_mut().enumerate() {
        if *shape == Shape::Large {
            let uses = atoms.iter().filter(|a| a.relation == relation).count();
            if large_seen || uses > 1 {
                *shape = Shape::Random;
            }
            large_seen = true;
        }
    }

    // Data: shaped rows, then planted witness rows first so declared keys
    // keep them.
    let mut data = Vec::new();
    for (relation, (shape, large)) in schema.iter().zip(&shapes) {
        let index = data.len();
        let domain = &domains[index];
        let row = |random: &mut Random| {
            relation
                .types
                .iter()
                .zip(domain)
                .map(|(ty, d)| pick_value(random, *d, *ty))
                .collect::<Vec<_>>()
        };
        let rows: Vec<Vec<Value>> = match shape {
            Shape::Empty => Vec::new(),
            Shape::Single => vec![row(&mut random)],
            Shape::AllDuplicates => vec![row(&mut random); 2 + random.pick(3)],
            Shape::OneKey => {
                let first = row(&mut random);
                let key = if relation.key.is_empty() {
                    vec![0]
                } else {
                    relation.key.clone()
                };
                (0..2 + random.pick(5))
                    .map(|_| {
                        let mut next = row(&mut random);
                        for column in &key {
                            next[*column] = first[*column].clone();
                        }
                        next
                    })
                    .collect()
            }
            Shape::Distinct => {
                let mut rows = Vec::new();
                for _ in 0..2 + random.pick(5) {
                    let next = row(&mut random);
                    if !rows.contains(&next) {
                        rows.push(next);
                    }
                }
                rows
            }
            Shape::Large => (0..*large)
                .map(|ordinal| {
                    let mut next = row(&mut random);
                    next[0] = match relation.types[0] {
                        ColumnType::I64 => Value::I64(ordinal as i64),
                        ColumnType::U32 => Value::U32(ordinal as u32),
                        _ => Value::I32(ordinal as i32),
                    };
                    next
                })
                .collect(),
            Shape::Random => {
                let mut rows = (0..random.pick(5))
                    .map(|_| row(&mut random))
                    .collect::<Vec<_>>();
                if !rows.is_empty() && random.pick(2) == 0 {
                    let duplicate = rows[random.pick(rows.len())].clone();
                    rows.push(duplicate);
                }
                rows
            }
        };
        data.push(rows);
    }
    let mut planted = vec![Vec::new(); relations];
    if preserve_witness {
        for atom in &atoms {
            let row = atom
                .terms
                .iter()
                .enumerate()
                .map(|(column, term)| match term {
                    Term::Var(variable) => witness[*variable].clone(),
                    Term::Constant(value) => value.clone(),
                    Term::Wildcard => pick_value(
                        &mut random,
                        domains[atom.relation][column],
                        schema[atom.relation].types[column],
                    ),
                    Term::Expression(_) => unreachable!("typed lane atoms"),
                })
                .collect::<Vec<_>>();
            planted[atom.relation].push(row);
        }
        for (rows, witness_rows) in data.iter_mut().zip(&planted) {
            rows.splice(0..0, witness_rows.iter().cloned());
        }
        // Keep the witness out of every negated pattern where possible.
        for negation in &negations {
            data[negation.relation].retain(|row| {
                planted[negation.relation].contains(row)
                    || !negation
                        .terms
                        .iter()
                        .zip(row)
                        .all(|(term, value)| match term {
                            Term::Var(v) => witness[*v] == *value,
                            Term::Constant(c) => c == value,
                            _ => true,
                        })
            });
        }
    }
    for (relation, rows) in schema.iter().zip(&mut data) {
        if !relation.key.is_empty() {
            let mut keys = BTreeSet::new();
            rows.retain(|row| {
                keys.insert(
                    relation
                        .key
                        .iter()
                        .map(|c| row[*c].clone())
                        .collect::<Vec<_>>(),
                )
            });
        }
    }
    if random.pick(10) == 0 {
        // Pick a used input, so this also exercises empty query answers.
        data[atoms[random.pick(atoms.len())].relation].clear();
    }
    Case {
        seed,
        query: Query {
            schema,
            atoms,
            negations,
            bindings,
            guards,
            constraints,
            blocks: Vec::new(),
            head,
            folds,
            order: Vec::new(),
            raw_names: raw,
        },
        data,
    }
}

fn generate_block(seed: u64, stream: u64, opaque_rust: bool) -> Case {
    let mut random = Random(crate::random::SplitMix64::new(seed ^ stream));
    let rust_oracle = opaque_rust && seed & 1 == 0;
    let test = (seed / 10) % 3 == 0;
    let kind = if (seed / 10) % 2 == 0 {
        BlockKind::Union
    } else {
        BlockKind::Fallback
    };
    let mut query = Query {
        schema: vec![
            Relation::i32("r0", 1),
            Relation::i32("r1", 3),
            Relation::i32("r2", 3),
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
        negations: Vec::new(),
        bindings: Vec::new(),
        guards: Vec::new(),
        constraints: Vec::new(),
        blocks: Vec::new(),
        head: if test {
            vec![Expr::Var(0)]
        } else {
            vec![Expr::Var(0), Expr::Var(1)]
        },
        folds: Vec::new(),
        order: vec![Step::Atom(0), Step::Block(0)],
        raw_names: false,
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
            Relational::Compare(
                Comparison::Equal,
                RelTerm::Var(1),
                RelTerm::Literal(Value::I32(1)),
            ),
            Relational::Compare(
                Comparison::Equal,
                RelTerm::Var(1),
                RelTerm::Literal(Value::I32(2)),
            ),
        ];
        query.blocks.push(Block {
            kind,
            branches: vec![vec![Step::Constraint(0)], vec![Step::Constraint(1)]],
            interface: vec![],
        });
        query.head = vec![Expr::Var(0)];
        query.order = vec![Step::Atom(0), Step::Binding(0), Step::Block(0)];
    }
    let mut data = query
        .schema
        .iter()
        .map(|schema| {
            let mut rows = (0..random.pick(8))
                .map(|_| {
                    (0..schema.arity())
                        .map(|_| random.small())
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            if !rows.is_empty() {
                rows.push(rows[0].clone());
            }
            rows
        })
        .collect::<Vec<_>>();
    // Witness rows joining r0's first value through both branches.
    if !data[0].is_empty() && random.pick(4) != 0 {
        let (x, y) = (data[0][0][0], random.small());
        data[1].push(vec![x, y, random.small()]);
        data[2].push(vec![x, y, random.small()]);
    }
    assert!(query.well_bound());
    Case {
        seed,
        query,
        data: typed_data(data),
    }
}

fn typed_data(data: Vec<Vec<Vec<i32>>>) -> Vec<Vec<Vec<Value>>> {
    data.into_iter()
        .map(|rows| {
            rows.into_iter()
                .map(|row| row.into_iter().map(Value::I32).collect())
                .collect()
        })
        .collect()
}

fn generate_rust(seed: u64, stream: u64) -> Case {
    let mut random = Random(crate::random::SplitMix64::new(
        seed ^ stream ^ 0x0050_4152_5449_414c,
    ));
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
        schema: vec![Relation::i32("r0", 1), Relation::i32("r1", 2)],
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
        negations: Vec::new(),
        bindings: Vec::new(),
        guards: vec![selection],
        constraints: Vec::new(),
        blocks: Vec::new(),
        head: vec![Expr::Var(0), Expr::Var(2)],
        folds: Vec::new(),
        order: vec![Step::Atom(0)],
        raw_names: false,
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
                query.schema[1].types.push(ColumnType::I32);
                query.atoms[1].terms.push(Term::Constant(Value::I32(5)));
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
        query.constraints.push(random.relational(&[TypedVar {
            index: 0,
            ty: ColumnType::I32,
            domain: super::data::Domain::Small,
        }]));
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
    if query.schema[1].arity() == 3 {
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
    Case {
        seed,
        query,
        data: typed_data(data),
    }
}

impl Case {
    /// The same case with `relation`'s columns stored in `order` (new column
    /// `i` is old column `order[i]`): declaration, data, every positive and
    /// negated occurrence, and the declared key are permuted together, so the
    /// answer is unchanged. Native hosts use it to put a declared key first.
    pub fn reorder_columns(&self, relation: usize, order: &[usize]) -> Self {
        let arity = self.query.schema[relation].arity();
        assert_eq!(
            order.iter().copied().collect::<BTreeSet<_>>(),
            (0..arity).collect::<BTreeSet<_>>(),
            "column order must be a permutation"
        );
        let permute = |items: &[Term]| order.iter().map(|c| items[*c].clone()).collect();
        let mut case = self.clone();
        let schema = &mut case.query.schema[relation];
        schema.types = order
            .iter()
            .map(|c| self.query.schema[relation].types[*c])
            .collect();
        schema.key = schema
            .key
            .iter()
            .map(|old| order.iter().position(|c| c == old).unwrap())
            .collect();
        for atom in case
            .query
            .atoms
            .iter_mut()
            .chain(&mut case.query.negations)
            .filter(|atom| atom.relation == relation)
        {
            atom.terms = permute(&atom.terms);
        }
        for row in &mut case.data[relation] {
            *row = order.iter().map(|c| row[*c].clone()).collect();
        }
        case
    }
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
        for feature in case.features() {
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
    use crate::fuzz::Profile;

    #[test]
    #[ignore = "generated sweep: cargo test -p cq-testkit -- --ignored"]
    fn printed_cq_passes_the_cq_contract_for_every_profile() {
        for profile in [Profile::quick(), Profile::full(), Profile::gpu()] {
            for seed in 0..256 {
                let case = generate_with(seed, &profile);
                assert_eq!(case.written_cq(), case.cq());
                let written: cq_lang_cq::Program = syn::parse_str(&case.cq())
                    .unwrap_or_else(|e| panic!("seed {seed}: {e}\n{}", case.cq()));
                cq_lang_cq::contract::check(&written)
                    .unwrap_or_else(|e| panic!("seed {seed}: {e}\n{}", case.cq()));
                cq_lang_cq::normal::normalize(&written)
                    .unwrap_or_else(|e| panic!("seed {seed}: {e}\n{}", case.cq()));
            }
        }
    }

    #[test]
    #[ignore = "generated sweep: cargo test -p cq-testkit -- --ignored"]
    fn seeded_cases_are_well_formed_typed_and_keyed() {
        for seed in 0..2048 {
            let case = generate(seed);
            assert_eq!(case, generate(seed));
            assert!((1..=4).contains(&case.query.schema.len()));
            assert!((1..=5).contains(&case.query.atoms.len()));
            assert!(case.query.well_bound());
            assert!(
                case.parameters()
                    .keys()
                    .all(|name| case.cq().contains(name))
            );
            for (relation, rows) in case.query.schema.iter().zip(&case.data) {
                assert!((1..=3).contains(&relation.arity()));
                for row in rows {
                    let types = row.iter().map(Value::column_type).collect::<Vec<_>>();
                    assert_eq!(types, relation.types, "seed {seed}");
                }
                let keys = rows
                    .iter()
                    .map(|row| relation.key.iter().map(|c| &row[*c]).collect::<Vec<_>>())
                    .collect::<BTreeSet<_>>();
                if !relation.key.is_empty() {
                    assert_eq!(keys.len(), rows.len(), "seed {seed}: key not unique");
                }
            }
        }
    }

    #[test]
    fn independent_printers_preserve_negative_constants_repeats_and_lets() {
        let case = Case {
            seed: 0,
            query: Query {
                schema: vec![Relation::i32("r0", 3)],
                atoms: vec![
                    Atom {
                        relation: 0,
                        terms: vec![Term::Var(0), Term::Var(0), Term::Constant(Value::I32(-2))],
                    },
                    Atom {
                        relation: 0,
                        terms: vec![Term::Var(0), Term::Wildcard, Term::Var(1)],
                    },
                ],
                negations: Vec::new(),
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
                folds: Vec::new(),
                order: Vec::new(),
                raw_names: false,
            },
            data: vec![vec![
                vec![Value::I32(1), Value::I32(1), Value::I32(-2)],
                vec![Value::I32(1), Value::I32(7), Value::I32(3)],
            ]],
        };
        assert_eq!(
            case.reference(),
            BTreeSet::from([vec![Value::I32(4)], vec![Value::I32(-1)]])
        );
        assert!(case.without_atom(1).is_none());
        assert!(case.without_atom(0).is_some());
    }

    #[cfg(feature = "duckdb")]
    #[test]
    fn sql_boolean_precedence_signed_remainders_and_duplicates_match_known_answers() {
        let i = Value::I32;
        let mut case = Case {
            seed: 0,
            query: Query {
                schema: vec![Relation::i32("r0", 2)],
                atoms: vec![
                    Atom {
                        relation: 0,
                        terms: vec![Term::Var(0), Term::Constant(i(-2))],
                    },
                    Atom {
                        relation: 0,
                        terms: vec![Term::Var(0), Term::Var(1)],
                    },
                ],
                negations: Vec::new(),
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
                folds: Vec::new(),
                order: Vec::new(),
                raw_names: false,
            },
            data: vec![
                [[-3, -2], [-3, 2], [-3, 2], [2, -2], [2, -3], [0, -2]]
                    .map(|row| row.map(i).to_vec())
                    .to_vec(),
            ],
        };
        let expected = BTreeSet::from([vec![i(-2), i(-3)], vec![i(-1), i(-3)], vec![i(-1), i(2)]]);
        assert_eq!(crate::duckdb::evaluate_fuzz(&case).unwrap(), expected);
        assert_eq!(case.reference(), expected);
        case.data[0].clear();
        assert!(crate::duckdb::evaluate_fuzz(&case).unwrap().is_empty());
        assert!(case.reference().is_empty());
    }
}
