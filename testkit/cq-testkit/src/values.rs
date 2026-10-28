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

/// Independent integer interpretation of the relational syntax. Parameters,
/// when used by an IR fixture, are supplied in the external binding map.
pub fn evaluate_constraint(c: &cq_syntax::Constraint, row: &Binding) -> bool {
    use cq_syntax::constraint::{Comparison, Constraint, Term};
    let value = |t: &Term| match t {
        Term::Variable(v) => row[&symbol_name(v)],
        Term::Parameter(p) => row[&p.name()],
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
