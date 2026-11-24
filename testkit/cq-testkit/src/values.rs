//! Finite integer relations, Set semantics, and total comparison helpers.
//!
//! Headings are named sets; output tuples preserve the requested head order.
//! Equality is Leibniz equality on `i32`, with consistent total ordering.
//! Helpers assume bound variables and supported integer syntax; they panic
//! outside that domain. They do not use language analysis or pass helpers.

use cq_syntax::symbol_name;
use std::collections::{BTreeMap, BTreeSet};
/// A finite set of ordered integer tuples.
pub type Rows = BTreeSet<Vec<i32>>;
/// Declared relation names mapped to sets of positional tuples.
pub type Database = BTreeMap<String, Rows>;
/// A complete assignment to the named variables of a heading.
pub type Binding = BTreeMap<String, i32>;

/// Column types of generated relations. `Str` is an interned dictionary code
/// in compiled programs; the reference compares the decoded strings, whose
/// dictionary is ordered, so order comparisons agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ColumnType {
    I32,
    I64,
    U32,
    Usize,
    Str,
}

impl ColumnType {
    /// The Rust column type spelling for numeric columns. `Str` has none:
    /// queries see interned codes, never `&str` columns.
    pub fn rust(self) -> &'static str {
        match self {
            Self::I32 => "i32",
            Self::I64 => "i64",
            Self::U32 => "u32",
            Self::Usize => "usize",
            Self::Str => panic!("string columns are interned codes, declared by the consumer"),
        }
    }

    /// The DuckDB column type with the same value domain.
    pub fn sql(self) -> &'static str {
        match self {
            Self::I32 => "INTEGER",
            Self::I64 => "BIGINT",
            Self::U32 => "UINTEGER",
            Self::Usize => "UBIGINT",
            Self::Str => "VARCHAR",
        }
    }

    /// Parse the runtime text encoding produced by `Value`'s `Display`.
    pub fn parse(self, text: &str) -> Value {
        fn number<T: std::str::FromStr>(text: &str, ty: ColumnType) -> T {
            text.parse()
                .unwrap_or_else(|_| panic!("{text:?} is not a {ty:?} value"))
        }
        match self {
            Self::I32 => Value::I32(number(text, self)),
            Self::I64 => Value::I64(number(text, self)),
            Self::U32 => Value::U32(number(text, self)),
            Self::Usize => Value::Usize(number(text, self)),
            Self::Str => Value::Str(text.to_owned()),
        }
    }
}

/// One typed scalar. Ordering is by type, then by value; generated relations
/// never compare values of different types.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Value {
    I32(i32),
    I64(i64),
    U32(u32),
    Usize(u64),
    Str(String),
}

impl Value {
    pub fn column_type(&self) -> ColumnType {
        match self {
            Self::I32(_) => ColumnType::I32,
            Self::I64(_) => ColumnType::I64,
            Self::U32(_) => ColumnType::U32,
            Self::Usize(_) => ColumnType::Usize,
            Self::Str(_) => ColumnType::Str,
        }
    }

    /// A suffixed Rust literal, e.g. `-5i64`; strings are quoted.
    pub fn rust_literal(&self) -> String {
        match self {
            Self::I32(v) => format!("{v}i32"),
            Self::I64(v) => format!("{v}i64"),
            Self::U32(v) => format!("{v}u32"),
            Self::Usize(v) => format!("{v}usize"),
            Self::Str(s) => format!("{s:?}"),
        }
    }

    /// A literal DuckDB accepts for the same domain.
    pub fn sql_literal(&self) -> String {
        match self {
            Self::I32(v) => format!("CAST({v} AS INTEGER)"),
            Self::I64(v) => format!("CAST({v} AS BIGINT)"),
            Self::U32(v) => format!("CAST({v} AS UINTEGER)"),
            Self::Usize(v) => format!("CAST({v} AS UBIGINT)"),
            Self::Str(s) => format!("'{}'", s.replace('\'', "''")),
        }
    }

    /// Integer value widened to i128; `None` for strings.
    pub fn as_i128(&self) -> Option<i128> {
        match self {
            Self::I32(v) => Some((*v).into()),
            Self::I64(v) => Some((*v).into()),
            Self::U32(v) => Some((*v).into()),
            Self::Usize(v) => Some((*v).into()),
            Self::Str(_) => None,
        }
    }

    pub fn as_i32(&self) -> i32 {
        match self {
            Self::I32(v) => *v,
            other => panic!("{other:?} is not an i32 value"),
        }
    }
}

impl std::fmt::Display for Value {
    /// Runtime text encoding: plain decimal, or the raw ASCII string.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::I32(v) => write!(f, "{v}"),
            Self::I64(v) => write!(f, "{v}"),
            Self::U32(v) => write!(f, "{v}"),
            Self::Usize(v) => write!(f, "{v}"),
            Self::Str(s) => f.write_str(s),
        }
    }
}

impl From<i32> for Value {
    fn from(value: i32) -> Self {
        Self::I32(value)
    }
}

/// A finite set of typed tuples (the G3 reference's answer type).
pub type TypedRows = BTreeSet<Vec<Value>>;

/// Lift i32 answers of the IR evaluators into typed rows for comparison.
pub fn typed(rows: &Rows) -> TypedRows {
    rows.iter()
        .map(|row| row.iter().copied().map(Value::I32).collect())
        .collect()
}

/// A finite named-heading relation under Set semantics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relation {
    pub heading: BTreeSet<String>,
    pub rows: BTreeSet<Binding>,
}

impl Relation {
    /// Retain a subset of the heading and collapse duplicate witnesses.
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

    /// Natural join, requiring one witness matching every shared variable.
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

    /// Project into the supplied positional order, including repeated columns.
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

pub fn evaluate_predicate(expr: &syn::Expr, row: &Binding) -> bool {
    // Conjunctions of the total comparison fragment; nothing else is modeled.
    if let syn::Expr::Binary(syn::ExprBinary {
        op: syn::BinOp::And(_),
        left,
        right,
        ..
    }) = cq_syntax::rust::unparen(expr)
    {
        return evaluate_predicate(left, row) && evaluate_predicate(right, row);
    }
    let syn::Expr::Binary(binary) = cq_syntax::rust::unparen(expr) else {
        panic!("integer comparison oracle domain")
    };
    let l = evaluate_integer(&binary.left, row);
    let r = evaluate_integer(&binary.right, row);
    match binary.op {
        syn::BinOp::Eq(_) => l == r,
        syn::BinOp::Ne(_) => l != r,
        syn::BinOp::Lt(_) => l < r,
        syn::BinOp::Le(_) => l <= r,
        syn::BinOp::Gt(_) => l > r,
        syn::BinOp::Ge(_) => l >= r,
        _ => panic!("integer comparison oracle domain"),
    }
}

pub fn evaluate_integer(expr: &syn::Expr, row: &Binding) -> i32 {
    match cq_syntax::rust::unparen(expr) {
        syn::Expr::Path(p) => row[&symbol_name(p.path.get_ident().expect("variable"))],
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Int(n),
            ..
        }) => n.base10_parse().unwrap(),
        syn::Expr::Unary(u) => match u.op {
            syn::UnOp::Deref(_) => evaluate_integer(&u.expr, row),
            syn::UnOp::Neg(_) => -evaluate_integer(&u.expr, row),
            _ => panic!("integer term oracle domain"),
        },
        _ => panic!("integer term oracle domain"),
    }
}

pub fn evaluate_equality(e: &cq_syntax::Equality, row: &Binding) -> bool {
    let right = match &e.right {
        cq_syntax::EqualityValue::Variable(v) => row[&symbol_name(v)],
        cq_syntax::EqualityValue::Literal(l) => evaluate_integer(l, row),
    };
    row[&symbol_name(&e.left)] == right
}

/// Independent integer interpretation of the relational syntax. Closed host
/// captures, when used by an IR fixture, are supplied in the external binding map.
pub fn evaluate_constraint(c: &cq_syntax::Constraint, row: &Binding) -> bool {
    use cq_syntax::constraint::{Comparison, Constraint, Term};
    let value = |t: &Term| match t {
        Term::Variable(v) => row[&symbol_name(v)],
        Term::Closed(expression) => evaluate_integer(expression, row),
        Term::Literal(l) => evaluate_integer(l, row),
    };
    match c {
        Constraint::Compare {
            left,
            comparison,
            right,
        } => {
            let (l, r) = (value(left), value(right));
            match comparison {
                Comparison::Eq(_) => l == r,
                Comparison::Ne(_) => l != r,
                Comparison::Lt(_) => l < r,
                Comparison::Le(_) => l <= r,
                Comparison::Gt(_) => l > r,
                Comparison::Ge(_) => l >= r,
            }
        }
        Constraint::Between { term, low, high } => {
            value(low) <= value(term) && value(term) <= value(high)
        }
        Constraint::In { term, literals } => literals
            .iter()
            .any(|l| value(term) == evaluate_integer(l, row)),
        Constraint::Or(branches) => branches
            .iter()
            .any(|b| b.iter().all(|c| evaluate_constraint(c, row))),
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
}
