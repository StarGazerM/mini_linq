//! Embedded Rust: comparison windows and the fragment deref rewrite.

use super::*;

/// The comparison window of a select on one attribute: the tightest integer
/// bounds its `&&`-conjuncts state against literals, and the conjuncts that
/// state something else.
pub struct Window {
    pub low: Option<i32>,
    pub high: Option<i32>,
    pub residual: Vec<Expr>,
}

fn conjuncts(expr: &Expr) -> Vec<&Expr> {
    match structures::syntax::rust::unparen(expr) {
        Expr::Binary(syn::ExprBinary {
            op: syn::BinOp::And(_),
            left,
            right,
            ..
        }) => {
            let mut all = conjuncts(left);
            all.extend(conjuncts(right));
            all
        }
        other => vec![other],
    }
}

pub fn window(condition: &Expr, column: &str) -> Window {
    use structures::syntax::predicate::{Comparison, Predicate, Term};
    let mut window = Window {
        low: None,
        high: None,
        residual: Vec::new(),
    };
    for part in conjuncts(condition) {
        // `column cmp literal`, with a literal on the left mirrored.
        let bounds = Predicate::from_expr(part).and_then(|predicate| {
            let (mirrored, literal) = match (&predicate.left, &predicate.right) {
                (Term::Variable(v), Term::Integer(k)) if symbol_name(v) == column => (false, *k),
                (Term::Integer(k), Term::Variable(v)) if symbol_name(v) == column => (true, *k),
                _ => return None,
            };
            let (below, above) = match (predicate.comparison, mirrored) {
                (Comparison::Eq(_), _) => (Some(literal), Some(literal)),
                (Comparison::Ge(_), false) | (Comparison::Le(_), true) => (Some(literal), None),
                (Comparison::Gt(_), false) | (Comparison::Lt(_), true) => {
                    (Some(literal.checked_add(1)?), None)
                }
                (Comparison::Le(_), false) | (Comparison::Ge(_), true) => (None, Some(literal)),
                (Comparison::Lt(_), false) | (Comparison::Gt(_), true) => {
                    (None, Some(literal.checked_sub(1)?))
                }
                (Comparison::Ne(_), _) => return None,
            };
            Some((below, above))
        });
        match bounds {
            Some((below, above)) => {
                if let Some(v) = below {
                    window.low = Some(window.low.map_or(v, |l| l.max(v)));
                }
                if let Some(v) = above {
                    window.high = Some(window.high.map_or(v, |h| h.min(v)));
                }
            }
            None => window.residual.push(part.clone()),
        }
    }
    window
}

/// Bound columns are references; the total comparison fragment is written
/// over values, so its variables are dereferenced here. Other Rust is the
/// author's and is emitted verbatim.
pub fn deref_predicates(expr: &Expr) -> Expr {
    use structures::syntax::predicate::{Predicate, Term};
    if let Expr::Binary(syn::ExprBinary {
        op: syn::BinOp::And(and),
        left,
        right,
        ..
    }) = structures::syntax::rust::unparen(expr)
    {
        let (left, right) = (deref_predicates(left), deref_predicates(right));
        return syn::parse_quote!((#left) #and (#right));
    }
    let Some(predicate) = Predicate::from_expr(expr) else {
        return expr.clone();
    };
    let term = |term: &Term| -> Expr {
        match term {
            Term::Variable(v) => syn::parse_quote!(*#v),
            Term::Integer(i) => syn::parse_quote!(#i),
        }
    };
    let (left, right, comparison) = (
        term(&predicate.left),
        term(&predicate.right),
        predicate.comparison,
    );
    syn::parse_quote!(#left #comparison #right)
}
