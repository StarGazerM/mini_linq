use cq_lang_structures::syntax::{
    self, Constraint, EqualityValue,
    constraint::{Comparison, Term},
    symbol_name,
};
use proc_macro2::TokenStream;
use quote::{ToTokens, format_ident, quote};
use std::collections::BTreeMap;
use syn::{Expr, Ident};

/// Values evaluated exactly once on the host, then passed as encoded scalars.
#[derive(Default)]
pub(crate) struct Constants {
    pub values: Vec<(Ident, TokenStream)>,
}
impl Constants {
    pub fn add(&mut self, value: TokenStream) -> TokenStream {
        let spelling = value.to_string();
        if let Some((id, _)) = self.values.iter().find(|(_, v)| v.to_string() == spelling) {
            return quote!(#id);
        }
        let id = format_ident!("__cq_gpu_constant_{}", self.values.len());
        self.values.push((id.clone(), value));
        quote!(#id)
    }
    pub fn term(
        &mut self,
        term: &Term,
        vars: &BTreeMap<String, TokenStream>,
    ) -> syn::Result<TokenStream> {
        match term {
            Term::Variable(id) => vars.get(&symbol_name(id)).cloned().ok_or_else(|| {
                syn::Error::new_spanned(id, "CQ_GPU_COLUMN: missing carried predicate column")
            }),
            Term::Parameter(id) => Ok(self.add(quote!(#id))),
            Term::Literal(expr) => self.literal(expr),
        }
    }
    pub fn literal(&mut self, expression: &Expr) -> syn::Result<TokenStream> {
        match expression {
            Expr::Lit(lit)
                if matches!(
                    lit.lit,
                    syn::Lit::Int(_) | syn::Lit::Str(_) | syn::Lit::Bool(_)
                ) =>
            {
                Ok(self.add(quote!(#expression)))
            }
            Expr::Unary(unary)
                if matches!(unary.op, syn::UnOp::Neg(_))
                    && matches!(&*unary.expr, Expr::Lit(lit) if matches!(lit.lit, syn::Lit::Int(_))) =>
            {
                Ok(self.add(quote!(#expression)))
            }
            _ => Err(syn::Error::new_spanned(
                expression,
                "CQ_GPU_LITERAL: only integral, boolean and string literals are supported",
            )),
        }
    }
    pub fn theta(
        &mut self,
        theta: &syntax::algebra::Theta,
        vars: &BTreeMap<String, TokenStream>,
        types: &BTreeMap<String, syn::Type>,
    ) -> syn::Result<TokenStream> {
        let mut conditions = Vec::new();
        for equality in &theta.equalities {
            let left = self.term(&Term::Variable(equality.left.clone()), vars)?;
            let right = match &equality.right {
                EqualityValue::Variable(id) => self.term(&Term::Variable(id.clone()), vars)?,
                EqualityValue::Literal(expr) => self.literal(expr)?,
            };
            conditions.push(quote!((#left) == (#right)));
        }
        for constraint in &theta.constraints {
            conditions.push(self.constraint(constraint, vars, types)?);
        }
        Ok(quote!(true #(&& (#conditions))*))
    }
    fn constraint(
        &mut self,
        constraint: &Constraint,
        vars: &BTreeMap<String, TokenStream>,
        types: &BTreeMap<String, syn::Type>,
    ) -> syn::Result<TokenStream> {
        Ok(match constraint {
            Constraint::Compare {
                left,
                comparison,
                right,
            } => {
                if comparison.ordered() && (string_term(left, types) || string_term(right, types)) {
                    return Err(syn::Error::new_spanned(
                        constraint,
                        "CQ_GPU_STRING_ORDER: dictionary IDs support equality, not string ordering",
                    ));
                }
                let left = self.term(left, vars)?;
                let right = self.term(right, vars)?;
                let op = match comparison {
                    Comparison::Eq(_) => quote!(==),
                    _ => comparison.to_token_stream(),
                };
                quote!((#left) #op (#right))
            }
            Constraint::Between { term, low, high } => {
                if [term, low, high]
                    .iter()
                    .any(|term| string_term(term, types))
                {
                    return Err(syn::Error::new_spanned(
                        constraint,
                        "CQ_GPU_STRING_ORDER: dictionary IDs support equality, not string ordering",
                    ));
                }
                let term = self.term(term, vars)?;
                let low = self.term(low, vars)?;
                let high = self.term(high, vars)?;
                quote!((#low) <= (#term) && (#term) <= (#high))
            }
            Constraint::In { term, literals } => {
                let term = self.term(term, vars)?;
                let values = literals
                    .iter()
                    .map(|literal| self.literal(literal))
                    .collect::<syn::Result<Vec<_>>>()?;
                quote!(false #(|| (#term) == (#values))*)
            }
            Constraint::Or(branches) => {
                let branches = branches
                    .iter()
                    .map(|branch| {
                        let parts = branch
                            .iter()
                            .map(|c| self.constraint(c, vars, types))
                            .collect::<syn::Result<Vec<_>>>()?;
                        Ok(quote!(true #(&& (#parts))*))
                    })
                    .collect::<syn::Result<Vec<_>>>()?;
                quote!(false #(|| (#branches))*)
            }
            Constraint::Total {
                expression,
                bindings,
            } => {
                let strings = bindings
                    .iter()
                    .filter(|(_, term)| string_term(term, types))
                    .map(|(id, _)| symbol_name(id))
                    .collect();
                check_total_order(expression, &strings)?;
                let aliases = bindings.iter().map(|(id, term)| {
                    let value=self.term(term,vars)?;
                    let boolean=match term {
                        Term::Variable(column)=>types.get(&symbol_name(column)).is_some_and(|ty|matches!(ty,syn::Type::Path(path) if path.path.is_ident("bool"))),
                        Term::Literal(expr)=>matches!(&**expr,Expr::Lit(lit) if matches!(lit.lit,syn::Lit::Bool(_))),
                        Term::Parameter(_)=>false,
                    };
                    Ok((symbol_name(id),if boolean {quote!((#value)!=0)}else{value}))
                }).collect::<syn::Result<BTreeMap<_, _>>>()?;
                self.total(expression, &aliases)?
            }
        })
    }
    fn total(
        &mut self,
        expr: &Expr,
        vars: &BTreeMap<String, TokenStream>,
    ) -> syn::Result<TokenStream> {
        match expr {
            Expr::Path(path) if path.path.get_ident().is_some() => {
                let id = path.path.get_ident().unwrap();
                vars.get(&symbol_name(id)).cloned().ok_or_else(|| syn::Error::new_spanned(expr, "CQ_GPU_TOTAL_CAPTURE: total predicates require explicit relational bindings"))
            }
            Expr::Paren(p) => self.total(&p.expr, vars).map(|e| quote!((#e))),
            Expr::Group(p) => self.total(&p.expr, vars),
            Expr::Reference(r) => self.total(&r.expr, vars),
            Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => self.total(&u.expr, vars),
            Expr::Unary(u) if matches!(u.op, syn::UnOp::Not(_)) => {
                let e = self.total(&u.expr, vars)?;
                Ok(quote!(!(#e)))
            }
            Expr::Lit(lit) if matches!(lit.lit, syn::Lit::Bool(_)) => Ok(quote!(#expr)),
            Expr::Lit(_) | Expr::Unary(_) => self.literal(expr),
            Expr::Binary(b)
                if matches!(
                    b.op,
                    syn::BinOp::Eq(_)
                        | syn::BinOp::Ne(_)
                        | syn::BinOp::Lt(_)
                        | syn::BinOp::Le(_)
                        | syn::BinOp::Gt(_)
                        | syn::BinOp::Ge(_)
                        | syn::BinOp::And(_)
                        | syn::BinOp::Or(_)
                ) =>
            {
                let left = self.total(&b.left, vars)?;
                let right = self.total(&b.right, vars)?;
                let op = &b.op;
                Ok(quote!((#left) #op (#right)))
            }
            _ => Err(syn::Error::new_spanned(
                expr,
                "CQ_GPU_TOTAL_EXPR: only bound comparisons and boolean connectives are supported on device",
            )),
        }
    }
}

fn string_term(term: &Term, types: &BTreeMap<String, syn::Type>) -> bool {
    match term {
        Term::Variable(id) => types.get(&symbol_name(id)).is_some_and(|ty| match ty {
            syn::Type::Path(path) => path
                .path
                .segments
                .last()
                .is_some_and(|s| s.ident == "String" || s.ident == "str"),
            syn::Type::Reference(r) => {
                matches!(&*r.elem,syn::Type::Path(path) if path.path.is_ident("str"))
            }
            _ => false,
        }),
        Term::Literal(expr) => {
            matches!(&**expr,Expr::Lit(lit) if matches!(lit.lit,syn::Lit::Str(_)))
        }
        Term::Parameter(_) => false,
    }
}

fn check_total_order(
    expr: &Expr,
    strings: &std::collections::BTreeSet<String>,
) -> syn::Result<bool> {
    Ok(match expr {
        Expr::Path(path) => path
            .path
            .get_ident()
            .is_some_and(|id| strings.contains(&symbol_name(id))),
        Expr::Lit(lit) => matches!(lit.lit, syn::Lit::Str(_)),
        Expr::Paren(p) => check_total_order(&p.expr, strings)?,
        Expr::Group(p) => check_total_order(&p.expr, strings)?,
        Expr::Reference(r) => check_total_order(&r.expr, strings)?,
        Expr::Unary(u) => check_total_order(&u.expr, strings)?,
        Expr::Binary(b) => {
            let left = check_total_order(&b.left, strings)?;
            let right = check_total_order(&b.right, strings)?;
            if (left || right)
                && matches!(
                    b.op,
                    syn::BinOp::Lt(_) | syn::BinOp::Le(_) | syn::BinOp::Gt(_) | syn::BinOp::Ge(_)
                )
            {
                return Err(syn::Error::new_spanned(
                    expr,
                    "CQ_GPU_STRING_ORDER: dictionary IDs support equality, not string ordering",
                ));
            }
            false
        }
        _ => false,
    })
}
