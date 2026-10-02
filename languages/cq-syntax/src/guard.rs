//! Recognize Rust's comparison trait operations under the Eq/Ord premises.
//! All other expressions remain opaque. Only a top-level conjunction may be
//! split: its relational conjuncts restrict the domain of its residual guard.
use crate::{
    Constraint,
    constraint::{Comparison, Term},
    symbol_name,
};
use std::collections::BTreeSet;
use syn::{BinOp, Expr, UnOp};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Split {
    pub constraints: Vec<Constraint>,
    pub residual: Option<Expr>,
}

/// Formal rule CQ-GuardSplit: Extract total conjuncts; preserve opaque order in one guard.
/// For `a && f`, f already runs only where a holds. For `f && a`, selecting
/// a first only shrinks f's domain. Neither operation introduces a panic on
/// an input where the written guard returns. Mixed disjunctions/negations
/// cannot be split, since their short-circuit domains depend on opaque values.
pub fn split(expression: &Expr, query_names: &BTreeSet<String>) -> Split {
    fn collect(e: &Expr, names: &BTreeSet<String>, locals: &mut BTreeSet<String>, out: &mut Split) {
        let plain = unparen(e);
        if let Expr::Binary(b) = plain {
            if b.attrs.is_empty() && matches!(b.op, BinOp::And(_)) {
                collect(&b.left, names, locals, out);
                collect(&b.right, names, locals, out);
                return;
            }
        }
        if let Some(cs) = recognize(e, names, locals, false) {
            out.constraints.extend(cs);
        } else {
            out.residual = Some(match out.residual.take() {
                None => e.clone(),
                Some(left) => syn::parse_quote!(#left && #e),
            });
            if let Expr::Let(binding) = plain {
                locals.extend(crate::rust::bindings(&binding.pat).iter().map(symbol_name));
            }
        }
    }
    let mut result = Split {
        constraints: Vec::new(),
        residual: None,
    };
    collect(expression, query_names, &mut BTreeSet::new(), &mut result);
    if result.constraints.is_empty() {
        result.residual = Some(expression.clone());
    } else {
        result.constraints = compact(result.constraints);
    }
    result
}

fn unparen(mut e: &Expr) -> &Expr {
    loop {
        match e {
            Expr::Paren(p) if p.attrs.is_empty() => e = &p.expr,
            Expr::Group(g) if g.attrs.is_empty() => e = &g.expr,
            _ => return e,
        }
    }
}

fn term(e: &Expr, names: &BTreeSet<String>, locals: &BTreeSet<String>) -> Option<Term> {
    let e = unparen(e);
    if let Expr::Path(p) = e {
        if p.attrs.is_empty() {
            let id = crate::rust::variable(e)?;
            if locals.contains(&symbol_name(id)) {
                return None;
            }
            return Some(if names.contains(&symbol_name(id)) {
                Term::Variable(id.clone())
            } else {
                Term::Parameter(id.clone())
            });
        }
    }
    let wrapped = match e {
        Expr::Unary(u) if u.attrs.is_empty() && matches!(u.op, UnOp::Deref(_)) => &u.expr,
        Expr::Reference(r) if r.attrs.is_empty() && r.mutability.is_none() => &r.expr,
        _ => {
            let literal = match e {
                Expr::Lit(l) => l.attrs.is_empty(),
                Expr::Unary(u) if u.attrs.is_empty() && matches!(u.op, UnOp::Neg(_)) => {
                    matches!(u.expr.as_ref(), Expr::Lit(l) if l.attrs.is_empty()
                        && matches!(l.lit, syn::Lit::Int(_) | syn::Lit::Float(_)))
                }
                _ => false,
            };
            return literal.then(|| Term::Literal(Box::new(e.clone())));
        }
    };
    let wrapped = unparen(wrapped);
    let Expr::Path(p) = wrapped else { return None };
    if !p.attrs.is_empty() {
        return None;
    }
    let id = crate::rust::variable(wrapped)?;
    (names.contains(&symbol_name(id)) && !locals.contains(&symbol_name(id)))
        .then(|| Term::Variable(id.clone()))
}

/// Formal rule CQ-GuardRecognize: Rust comparison trait calls and pure boolean structure are central.
fn recognize(
    e: &Expr,
    names: &BTreeSet<String>,
    locals: &BTreeSet<String>,
    negative: bool,
) -> Option<Vec<Constraint>> {
    let e = unparen(e);
    if let Expr::Unary(u) = e {
        if u.attrs.is_empty() && matches!(u.op, UnOp::Not(_)) {
            return recognize(&u.expr, names, locals, !negative);
        }
    }
    let Expr::Binary(b) = e else { return None };
    if !b.attrs.is_empty() {
        return None;
    }
    if matches!(b.op, BinOp::And(_) | BinOp::Or(_)) {
        // Both sides must be recognizable. Never extract through mixed || or !.
        let left = recognize(&b.left, names, locals, negative)?;
        let right = recognize(&b.right, names, locals, negative)?;
        return Some(if matches!(b.op, BinOp::And(_)) != negative {
            compact(left.into_iter().chain(right).collect())
        } else {
            vec![disjoin(left, right)]
        });
    }
    let comparison = match (&b.op, negative) {
        (BinOp::Eq(_), false) | (BinOp::Ne(_), true) => Comparison::Eq(Default::default()),
        (BinOp::Ne(_), false) | (BinOp::Eq(_), true) => Comparison::Ne(Default::default()),
        (BinOp::Lt(_), false) | (BinOp::Ge(_), true) => Comparison::Lt(Default::default()),
        (BinOp::Le(_), false) | (BinOp::Gt(_), true) => Comparison::Le(Default::default()),
        (BinOp::Gt(_), false) | (BinOp::Le(_), true) => Comparison::Gt(Default::default()),
        (BinOp::Ge(_), false) | (BinOp::Lt(_), true) => Comparison::Ge(Default::default()),
        _ => return None,
    };
    Some(vec![Constraint::Compare {
        left: term(&b.left, names, locals)?,
        comparison,
        right: term(&b.right, names, locals)?,
    }])
}

// Preserve the existing compact IR for paired inclusive bounds and literal
// equality disjunctions. These are normalizations, never CQ surface syntax.
fn compact(cs: Vec<Constraint>) -> Vec<Constraint> {
    let mut result = Vec::new();
    let mut items = cs.into_iter().peekable();
    while let Some(c) = items.next() {
        if let Constraint::Compare {
            left,
            comparison: Comparison::Ge(_),
            right: low,
        } = &c
        {
            if let Some(Constraint::Compare {
                left: other,
                comparison: Comparison::Le(_),
                right: high,
            }) = items.peek()
            {
                if left == other {
                    result.push(Constraint::Between {
                        term: left.clone(),
                        low: low.clone(),
                        high: high.clone(),
                    });
                    items.next();
                    continue;
                }
            }
        }
        result.push(c);
    }
    result
}

fn disjoin(left: Vec<Constraint>, right: Vec<Constraint>) -> Constraint {
    let mut branches = Vec::new();
    for mut branch in [left, right] {
        if branch.len() == 1 && matches!(branch[0], Constraint::Or(_)) {
            let Constraint::Or(inner) = branch.pop().unwrap() else {
                unreachable!()
            };
            branches.extend(inner);
        } else {
            branches.push(branch);
        }
    }
    let mut membership = None;
    let mut literals = crate::CommaList::new();
    for branch in &branches {
        let (t, values) = match branch.as_slice() {
            [
                Constraint::Compare {
                    left,
                    comparison: Comparison::Eq(_),
                    right: Term::Literal(l),
                },
            ] => (left, vec![(**l).clone()]),
            [Constraint::In { term, literals }] => (term, literals.iter().cloned().collect()),
            _ => return Constraint::Or(branches),
        };
        if membership.as_ref().is_some_and(|old| old != t) {
            return Constraint::Or(branches);
        }
        membership = Some(t.clone());
        literals.extend(values);
    }
    Constraint::In {
        term: membership.unwrap(),
        literals,
    }
}
