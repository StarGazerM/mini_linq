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
    pub datalog_keys: BTreeMap<String, Vec<syn::Type>>,
    pub used_datalog_keys: std::collections::BTreeSet<String>,
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
        ty: Option<&syn::Type>,
    ) -> syn::Result<TokenStream> {
        match term {
            Term::Variable(id) => vars.get(&symbol_name(id)).cloned().ok_or_else(|| {
                syn::Error::new_spanned(id, "CQ_GPU_COLUMN: missing carried predicate column")
            }),
            Term::Parameter(id) => Ok(self.add(quote!(#id))),
            Term::Literal(expr) => self.literal(expr, ty),
        }
    }
    pub fn literal(
        &mut self,
        expression: &Expr,
        ty: Option<&syn::Type>,
    ) -> syn::Result<TokenStream> {
        let integral = matches!(expression, Expr::Lit(lit) if matches!(lit.lit, syn::Lit::Int(_)))
            || matches!(expression, Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Neg(_)));
        let value = if integral {
            let default = syn::parse_quote!(i64);
            let ty = ty.unwrap_or(&default);
            quote!({ let __cq_gpu_literal: #ty = #expression; __cq_gpu_literal })
        } else {
            quote!(#expression)
        };
        match expression {
            Expr::Lit(lit)
                if matches!(
                    lit.lit,
                    syn::Lit::Int(_) | syn::Lit::Str(_) | syn::Lit::Bool(_)
                ) =>
            {
                Ok(self.add(value))
            }
            Expr::Unary(unary)
                if matches!(unary.op, syn::UnOp::Neg(_))
                    && matches!(&*unary.expr, Expr::Lit(lit) if matches!(lit.lit, syn::Lit::Int(_))) =>
            {
                Ok(self.add(value))
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
            let ty = types.get(&symbol_name(&equality.left));
            let left = self.term(&Term::Variable(equality.left.clone()), vars, ty)?;
            let right = match &equality.right {
                EqualityValue::Variable(id) => self.term(&Term::Variable(id.clone()), vars, ty)?,
                EqualityValue::Literal(expr) => self.literal(expr, ty)?,
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
                let ty = term_type(left, types).or_else(|| term_type(right, types));
                let left = self.term(left, vars, ty)?;
                let right = self.term(right, vars, ty)?;
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
                let ty = term_type(term, types);
                let term = self.term(term, vars, ty)?;
                let low = self.term(low, vars, ty)?;
                let high = self.term(high, vars, ty)?;
                quote!((#low) <= (#term) && (#term) <= (#high))
            }
            Constraint::In { term, literals } => {
                let ty = term_type(term, types);
                let term = self.term(term, vars, ty)?;
                let values = literals
                    .iter()
                    .map(|literal| self.literal(literal, ty))
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
                    let value=self.term(term,vars,term_type(term,types))?;
                    let boolean=match term {
                        Term::Variable(column)=>types.get(&symbol_name(column)).is_some_and(|ty|matches!(ty,syn::Type::Path(path) if path.path.is_ident("bool"))),
                        Term::Literal(expr)=>matches!(&**expr,Expr::Lit(lit) if matches!(lit.lit,syn::Lit::Bool(_))),
                        Term::Parameter(_)=>false,
                    };
                    Ok((symbol_name(id),if boolean {quote!((#value)!=0)}else{value}))
                }).collect::<syn::Result<BTreeMap<_, _>>>()?;
                let alias_types = bindings
                    .iter()
                    .filter_map(|(id, term)| {
                        term_type(term, types).map(|ty| (symbol_name(id), ty.clone()))
                    })
                    .collect();
                self.total(expression, &aliases, &alias_types, None)?
            }
        })
    }
    fn total(
        &mut self,
        expr: &Expr,
        vars: &BTreeMap<String, TokenStream>,
        types: &BTreeMap<String, syn::Type>,
        expected: Option<&syn::Type>,
    ) -> syn::Result<TokenStream> {
        match expr {
            Expr::Call(call)
                if matches!(&*call.func, Expr::Path(path)
                if path.qself.is_none() && path.path.leading_colon.is_some()
                    && path.path.segments.iter().map(|s| s.ident.to_string()).collect::<Vec<_>>()
                        == ["cq_gpu_runtime", "datalog", "contains"]
                    && path.path.segments.iter().all(|s| matches!(s.arguments, syn::PathArguments::None))) =>
            {
                let invalid = || {
                    syn::Error::new_spanned(
                        expr,
                        "CQ_GPU_DATALOG_MEMBERSHIP: expected certified contains(&host, &(bound_columns,))",
                    )
                };
                if call.args.len() != 2 {
                    return Err(invalid());
                }
                let Expr::Reference(host) = &call.args[0] else {
                    return Err(invalid());
                };
                let Expr::Path(host) = &*host.expr else {
                    return Err(invalid());
                };
                let Some(host) = host.path.get_ident() else {
                    return Err(invalid());
                };
                let name = symbol_name(host);
                let schema = self.datalog_keys.get(&name).cloned().ok_or_else(|| {
                    syn::Error::new_spanned(
                        host,
                        "CQ_GPU_DATALOG_MEMBERSHIP: key host lacks bridge-certified schema",
                    )
                })?;
                let Expr::Reference(keys) = &call.args[1] else {
                    return Err(invalid());
                };
                let Expr::Tuple(keys) = &*keys.expr else {
                    return Err(invalid());
                };
                if keys.elems.len() != schema.len() {
                    return Err(invalid());
                }
                let mut values = Vec::new();
                for (key, ty) in keys.elems.iter().zip(&schema) {
                    let Some(actual) = expression_type(key, types) else {
                        return Err(invalid());
                    };
                    if actual != ty {
                        return Err(syn::Error::new_spanned(
                            key,
                            "CQ_GPU_DATALOG_KEY_TYPE: membership key must match its declared frozen schema",
                        ));
                    }
                    let column = match key {
                        Expr::Path(path) => path.path.get_ident(),
                        _ => None,
                    }
                    .ok_or_else(invalid)?;
                    let value = vars
                        .get(&symbol_name(column))
                        .cloned()
                        .ok_or_else(invalid)?;
                    values.push(quote!((#value) as u64));
                }
                self.used_datalog_keys.insert(name);
                let view = format_ident!(
                    "__cq_gpu_datalog_view_{}",
                    host,
                    span = proc_macro2::Span::mixed_site()
                );
                Ok(quote!(::cq_gpu_runtime::exact_contains(#view, &[#(#values),*])))
            }
            Expr::Path(path) if path.path.get_ident().is_some() => {
                let id = path.path.get_ident().unwrap();
                vars.get(&symbol_name(id)).cloned().ok_or_else(|| syn::Error::new_spanned(expr, "CQ_GPU_TOTAL_CAPTURE: total predicates require explicit relational bindings"))
            }
            Expr::Paren(p) => self
                .total(&p.expr, vars, types, expected)
                .map(|e| quote!((#e))),
            Expr::Group(p) => self.total(&p.expr, vars, types, expected),
            Expr::Reference(r) => self.total(&r.expr, vars, types, expected),
            Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => {
                self.total(&u.expr, vars, types, expected)
            }
            Expr::Unary(u) if matches!(u.op, syn::UnOp::Not(_)) => {
                let e = self.total(&u.expr, vars, types, expected)?;
                Ok(quote!(!(#e)))
            }
            Expr::Lit(lit) if matches!(lit.lit, syn::Lit::Bool(_)) => Ok(quote!(#expr)),
            Expr::Lit(_) | Expr::Unary(_) => self.literal(expr, expected),
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
                let ty =
                    expression_type(&b.left, types).or_else(|| expression_type(&b.right, types));
                let left = self.total(&b.left, vars, types, ty)?;
                let right = self.total(&b.right, vars, types, ty)?;
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

fn term_type<'a>(term: &Term, types: &'a BTreeMap<String, syn::Type>) -> Option<&'a syn::Type> {
    match term {
        Term::Variable(id) => types.get(&symbol_name(id)),
        _ => None,
    }
}

fn expression_type<'a>(
    expr: &Expr,
    types: &'a BTreeMap<String, syn::Type>,
) -> Option<&'a syn::Type> {
    match expr {
        Expr::Path(path) => path
            .path
            .get_ident()
            .and_then(|id| types.get(&symbol_name(id))),
        Expr::Paren(p) => expression_type(&p.expr, types),
        Expr::Group(p) => expression_type(&p.expr, types),
        Expr::Reference(r) => expression_type(&r.expr, types),
        Expr::Unary(u) if matches!(u.op, syn::UnOp::Deref(_)) => expression_type(&u.expr, types),
        _ => None,
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
