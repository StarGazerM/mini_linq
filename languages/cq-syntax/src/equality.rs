//! Relational equality is Leibniz identity in classical Set. An admissible
//! column type's `==` is the generated decision procedure for that identity.
//! CQ recognizes the corresponding Rust comparison trait operations in guards.
use syn::{Ident, Token};

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub enum EqualityValue {
    Variable(Ident),
    Literal(syn::Expr),
}

impl syn::parse::Parse for EqualityValue {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let expression: syn::Expr = input.parse()?;
        if let Some(variable) = super::rust::variable(&expression) {
            Ok(Self::Variable(variable.clone()))
        } else if literal(&expression) {
            Ok(Self::Literal(expression))
        } else {
            Err(syn::Error::new_spanned(
                expression,
                "relational equality requires a variable or literal",
            ))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Equality {
    pub left: Ident,
    pub eq_token: Token![=],
    pub right: EqualityValue,
}

impl Equality {
    pub fn variables(&self) -> impl Iterator<Item = &Ident> {
        std::iter::once(&self.left).chain(match &self.right {
            EqualityValue::Variable(variable) => Some(variable),
            EqualityValue::Literal(_) => None,
        })
    }

    pub fn check(&self) -> syn::Result<()> {
        if let EqualityValue::Literal(value) = &self.right {
            if !literal(value) {
                return Err(syn::Error::new_spanned(
                    value,
                    "relational equality requires a literal",
                ));
            }
        }
        Ok(())
    }
}

/// Exactly a Syn literal, or unary minus applied to a numeric literal.
pub fn literal(expression: &syn::Expr) -> bool {
    match expression {
        syn::Expr::Lit(_) => true,
        syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)) => {
            matches!(
                &*unary.expr,
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Int(_) | syn::Lit::Float(_),
                    ..
                })
            )
        }
        _ => false,
    }
}
