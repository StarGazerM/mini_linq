//! A deliberately total selection fragment: comparisons of i32 variables/literals.
//! This is an optimizer recognizer, not the CQ expression grammar. CQ and RA
//! retain syn::Expr; only this proven total subset is eligible for movement.
use syn::{Ident, Token};

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub enum Term {
    Variable(Ident),
    Integer(i32),
}
impl Term {
    pub fn from_expr(expr: &syn::Expr) -> Option<Self> {
        if let syn::Expr::Paren(paren) = expr {
            if !paren.attrs.is_empty() {
                return None;
            }
            return Self::from_expr(&paren.expr);
        }
        if let syn::Expr::Path(path) = expr {
            if !path.attrs.is_empty() {
                return None;
            }
            return super::rust::variable(expr).cloned().map(Self::Variable);
        }
        let (sign, literal) = match expr {
            syn::Expr::Unary(e) if e.attrs.is_empty() && matches!(e.op, syn::UnOp::Neg(_)) => {
                (-1, e.expr.as_ref())
            }
            _ => (1, expr),
        };
        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Int(literal),
            attrs,
        }) = literal
        else {
            return None;
        };
        if !attrs.is_empty() || !matches!(literal.suffix(), "" | "i32") {
            return None;
        }
        let value = sign * literal.base10_parse::<i64>().ok()?;
        Some(Self::Integer(i32::try_from(value).ok()?))
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub enum Comparison {
    Eq(Token![==]),
    Ne(Token![!=]),
    Lt(Token![<]),
    Le(Token![<=]),
    Gt(Token![>]),
    Ge(Token![>=]),
}
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub struct Predicate {
    pub left: Term,
    pub comparison: Comparison,
    pub right: Term,
}
impl Predicate {
    /// Recognize the movable comparison fragment directly in Syn's Rust AST.
    pub fn from_expr(expr: &syn::Expr) -> Option<Self> {
        if let syn::Expr::Paren(paren) = expr {
            if !paren.attrs.is_empty() {
                return None;
            }
            return Self::from_expr(&paren.expr);
        }
        let syn::Expr::Binary(expr) = expr else {
            return None;
        };
        if !expr.attrs.is_empty() {
            return None;
        }
        let comparison = match &expr.op {
            syn::BinOp::Eq(t) => Comparison::Eq(*t),
            syn::BinOp::Ne(t) => Comparison::Ne(*t),
            syn::BinOp::Lt(t) => Comparison::Lt(*t),
            syn::BinOp::Le(t) => Comparison::Le(*t),
            syn::BinOp::Gt(t) => Comparison::Gt(*t),
            syn::BinOp::Ge(t) => Comparison::Ge(*t),
            _ => return None,
        };
        Some(Self {
            left: Term::from_expr(&expr.left)?,
            comparison,
            right: Term::from_expr(&expr.right)?,
        })
    }
}
