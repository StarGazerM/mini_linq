use cq_syntax::symbol_name;
use std::collections::{BTreeMap, BTreeSet};
pub type Rows = BTreeSet<Vec<i32>>;
pub type Database = BTreeMap<String, Rows>;
pub type Binding = BTreeMap<String, i32>;

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
