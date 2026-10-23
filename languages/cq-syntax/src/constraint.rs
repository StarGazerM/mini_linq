//! Normalized total predicates shared by RA/IX/Physical, not CQ source syntax.
//! The internal wire format keeps the existing comparison, paired inclusive
//! bounds, literal equality disjunction and boolean branch representations.
//! CQ accepts only Rust guards and constructs these forms through `guard`.
//! Nested branches represent arbitrary boolean structure without DNF expansion.
//! Parameters use `@name` internally; CQ Rust guards use bare host names.

use crate::{CommaList, Equality, EqualityValue, symbol_name};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::collections::BTreeSet;
use syn::{Ident, Token, parse::ParseStream};

mod kw {
    syn::custom_keyword!(between);
    syn::custom_keyword!(and);
    syn::custom_keyword!(or);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    Variable(Ident),
    Parameter(Ident),
    Literal(Box<syn::Expr>),
}

impl syn::parse::Parse for Term {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(Token![@]) {
            input.parse::<Token![@]>()?;
            return Ok(Self::Parameter(input.parse()?));
        }
        if input.peek(Ident) {
            return Ok(Self::Variable(input.parse()?));
        }
        let minus = if input.peek(Token![-]) {
            Some(input.parse::<Token![-]>()?)
        } else {
            None
        };
        if !input.peek(syn::Lit) {
            return Err(unexpected(
                input,
                "a relational term (identifier or literal)",
            ));
        }
        let literal: syn::Lit = input.parse()?;
        if minus.is_some() && !matches!(literal, syn::Lit::Int(_) | syn::Lit::Float(_)) {
            return Err(syn::Error::new_spanned(
                literal,
                "unexpected token after `-`: expected a numeric literal",
            ));
        }
        Ok(Self::Literal(Box::new(syn::parse2(
            quote!(#minus #literal),
        )?)))
    }
}

impl ToTokens for Term {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            Self::Variable(v) => v.to_tokens(tokens),
            Self::Parameter(v) => tokens.extend(quote!(@#v)),
            Self::Literal(v) => v.to_tokens(tokens),
        }
    }
}

impl Term {
    pub fn resolve(&mut self, logical: &BTreeSet<String>) {
        if let Self::Variable(v) = self {
            if !logical.contains(&symbol_name(v)) {
                *self = Self::Parameter(v.clone());
            }
        }
    }

    pub fn constant(&self) -> bool {
        !matches!(self, Self::Variable(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub enum Comparison {
    Eq(Token![=]),
    Ne(Token![!=]),
    Lt(Token![<]),
    Le(Token![<=]),
    Gt(Token![>]),
    Ge(Token![>=]),
}

impl Comparison {
    pub fn ordered(self) -> bool {
        !matches!(self, Self::Eq(_) | Self::Ne(_))
    }
}

impl syn::parse::Parse for Comparison {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        // Multi-token operators must precede their prefixes.
        if input.peek(Token![==]) {
            return Err(unexpected(
                input,
                "relational `=` (Rust `==` belongs in an `if` guard)",
            ));
        }
        Ok(if input.peek(Token![!=]) {
            Self::Ne(input.parse()?)
        } else if input.peek(Token![<=]) {
            Self::Le(input.parse()?)
        } else if input.peek(Token![>=]) {
            Self::Ge(input.parse()?)
        } else if input.peek(Token![=]) {
            Self::Eq(input.parse()?)
        } else if input.peek(Token![<]) {
            Self::Lt(input.parse()?)
        } else if input.peek(Token![>]) {
            Self::Gt(input.parse()?)
        } else {
            return Err(unexpected(
                input,
                "a relational comparison operator, `between`, or `in`",
            ));
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Constraint {
    Compare {
        left: Term,
        comparison: Comparison,
        right: Term,
    },
    Between {
        term: Term,
        low: Term,
        high: Term,
    },
    In {
        term: Term,
        literals: CommaList<syn::Expr>,
    },
    Or(Vec<Vec<Constraint>>),
}

impl syn::parse::Parse for Constraint {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        crate::reject_guard_attributes(input)?;
        if !input.peek(syn::token::Paren) {
            return atomic(input);
        }
        // IR branches serialize conjunctions, including nested boolean predicates.
        let mut branches = vec![conjunction(input)?];
        if !input.peek(kw::or) {
            return Err(unexpected(input, "`or` after a parenthesized conjunction"));
        }
        while input.peek(kw::or) {
            input.parse::<kw::or>()?;
            if !input.peek(syn::token::Paren) {
                return Err(unexpected(input, "a parenthesized conjunction after `or`"));
            }
            branches.push(conjunction(input)?);
        }
        Ok(Self::Or(branches))
    }
}

fn atomic(input: ParseStream<'_>) -> syn::Result<Constraint> {
    let term = input.parse()?;
    // Internal paired inclusive bounds.
    if input.peek(kw::between) {
        input.parse::<kw::between>()?;
        let low = input.parse()?;
        if !input.peek(kw::and) {
            return Err(unexpected(
                input,
                "`and` in an inclusive `between` constraint",
            ));
        }
        input.parse::<kw::and>()?;
        return Ok(Constraint::Between {
            term,
            low,
            high: input.parse()?,
        });
    }
    // Internal literal equality disjunction.
    if input.peek(Token![in]) {
        input.parse::<Token![in]>()?;
        if !input.peek(syn::token::Brace) {
            return Err(unexpected(input, "a literal set after `in`"));
        }
        let body;
        syn::braced!(body in input);
        let mut literals = CommaList::new();
        if body.is_empty() {
            return Err(unexpected(&body, "a nonempty literal set"));
        }
        while !body.is_empty() {
            let value: Term = body.parse()?;
            let Term::Literal(literal) = value else {
                return Err(syn::Error::new_spanned(
                    &value,
                    format!(
                        "unexpected token `{}` in membership set: expected a literal",
                        value.to_token_stream()
                    ),
                ));
            };
            literals.push(*literal);
            if body.is_empty() {
                break;
            }
            if !body.peek(Token![,]) {
                return Err(unexpected(&body, "`,` between membership literals"));
            }
            literals.push_punct(body.parse()?);
        }
        return Ok(Constraint::In { term, literals });
    }
    Ok(Constraint::Compare {
        left: term,
        comparison: input.parse()?,
        right: input.parse()?,
    })
}

fn conjunction(input: ParseStream<'_>) -> syn::Result<Vec<Constraint>> {
    let body;
    syn::parenthesized!(body in input);
    let mut constraints = vec![body.parse()?];
    while !body.is_empty() {
        if !body.peek(kw::and) {
            return Err(unexpected(&body, "`and` between atomic constraints"));
        }
        body.parse::<kw::and>()?;
        constraints.push(body.parse()?);
    }
    Ok(constraints)
}

pub fn unexpected(input: ParseStream<'_>, expected: &str) -> syn::Error {
    let token = input
        .fork()
        .parse::<proc_macro2::TokenTree>()
        .map(|t| t.to_string())
        .unwrap_or_else(|_| "end of input".into());
    input.error(format!("unexpected token `{token}`: expected {expected}"))
}

impl ToTokens for Constraint {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            Self::Compare {
                left,
                comparison,
                right,
            } => tokens.extend(quote!(#left #comparison #right)),
            Self::Between { term, low, high } => {
                tokens.extend(quote!(#term between #low and #high))
            }
            Self::In { term, literals } => tokens.extend(quote!(#term in { #literals })),
            Self::Or(branches) => {
                for (i, branch) in branches.iter().enumerate() {
                    if i != 0 {
                        tokens.extend(quote!(or));
                    }
                    let mut inner = TokenStream::new();
                    for (j, constraint) in branch.iter().enumerate() {
                        if j != 0 {
                            inner.extend(quote!(and));
                        }
                        constraint.to_tokens(&mut inner);
                    }
                    tokens.extend(quote!((#inner)));
                }
            }
        }
    }
}

impl Constraint {
    /// Formal rule E-Domain: Project every disjunct's literal restrictions onto a common variable, retaining the exact predicate.
    pub fn implied_domains(&self) -> Vec<Self> {
        let Self::Or(branches) = self else {
            return Vec::new();
        };
        let mut names = BTreeSet::new();
        for c in branches.iter().flatten() {
            names.extend(c.variables().map(symbol_name));
        }
        names
            .into_iter()
            .filter_map(|name| {
                let mut projected = Vec::new();
                for branch in branches {
                    let conjuncts: Vec<_> = branch
                        .iter()
                        .filter(|c| {
                            let literal_domain = matches!(
                                c,
                                Self::Compare {
                                    left: Term::Variable(_),
                                    right: Term::Literal(_),
                                    ..
                                } | Self::Compare {
                                    left: Term::Literal(_),
                                    right: Term::Variable(_),
                                    ..
                                } | Self::Between {
                                    term: Term::Variable(_),
                                    low: Term::Literal(_),
                                    high: Term::Literal(_),
                                } | Self::In {
                                    term: Term::Variable(_),
                                    ..
                                }
                            );
                            literal_domain && c.variables().all(|v| symbol_name(v) == name)
                        })
                        .cloned()
                        .collect();
                    if conjuncts.is_empty() {
                        return None;
                    }
                    projected.push(conjuncts);
                }
                let domain = Self::Or(projected);
                (domain != *self).then_some(domain)
            })
            .collect()
    }
    pub fn terms(&self) -> Vec<&Term> {
        match self {
            Self::Compare { left, right, .. } => vec![left, right],
            Self::Between { term, low, high } => vec![term, low, high],
            Self::In { term, .. } => vec![term],
            Self::Or(branches) => branches.iter().flatten().flat_map(Self::terms).collect(),
        }
    }

    pub fn variables(&self) -> impl Iterator<Item = &Ident> {
        self.terms().into_iter().filter_map(|t| match t {
            Term::Variable(v) => Some(v),
            _ => None,
        })
    }

    pub fn parameters(&self) -> impl Iterator<Item = &Ident> {
        self.terms().into_iter().filter_map(|t| match t {
            Term::Parameter(v) => Some(v),
            _ => None,
        })
    }

    pub fn ordered_variables(&self) -> Vec<&Ident> {
        match self {
            Self::Compare { comparison, .. } if !comparison.ordered() => Vec::new(),
            Self::In { .. } => Vec::new(),
            Self::Or(branches) => branches
                .iter()
                .flatten()
                .flat_map(Self::ordered_variables)
                .collect(),
            _ => self.variables().collect(),
        }
    }

    pub fn map_terms(&mut self, f: &mut impl FnMut(&mut Term)) {
        match self {
            Self::Compare { left, right, .. } => {
                f(left);
                f(right);
            }
            Self::Between { term, low, high } => {
                f(term);
                f(low);
                f(high);
            }
            Self::In { term, .. } => f(term),
            Self::Or(branches) => branches.iter_mut().flatten().for_each(|c| c.map_terms(f)),
        }
    }

    pub fn resolve(&mut self, logical: &BTreeSet<String>) {
        self.map_terms(&mut |t| t.resolve(logical));
    }

    pub fn equality(&self) -> Option<Equality> {
        let Self::Compare {
            left: Term::Variable(left),
            comparison: Comparison::Eq(eq_token),
            right,
        } = self
        else {
            return None;
        };
        let right = match right {
            Term::Variable(v) => EqualityValue::Variable(v.clone()),
            Term::Literal(v) => EqualityValue::Literal((**v).clone()),
            Term::Parameter(_) => return None,
        };
        Some(Equality {
            left: left.clone(),
            eq_token: *eq_token,
            right,
        })
    }

    pub fn from_equality(e: &Equality) -> Self {
        Self::Compare {
            left: Term::Variable(e.left.clone()),
            comparison: Comparison::Eq(e.eq_token),
            right: match &e.right {
                EqualityValue::Variable(v) => Term::Variable(v.clone()),
                EqualityValue::Literal(v) => Term::Literal(Box::new(v.clone())),
            },
        }
    }

    /// Rust guard spelling for normalized constraints; captures are bare names.
    pub fn cq_tokens(&self) -> TokenStream {
        let term = |t: &Term| match t {
            Term::Variable(v) | Term::Parameter(v) => quote!(#v),
            Term::Literal(l) => quote!(#l),
        };
        match self {
            Self::Compare {
                left,
                comparison,
                right,
            } => {
                let left = term(left);
                let right = term(right);
                let op = match comparison {
                    Comparison::Eq(_) => quote!(==),
                    _ => comparison.to_token_stream(),
                };
                quote!(#left #op #right)
            }
            Self::Between { term: t, low, high } => {
                let (t, low, high) = (term(t), term(low), term(high));
                quote!((#t >= #low && #t <= #high))
            }
            Self::In { term: t, literals } => {
                let t = term(t);
                let tests: Vec<_> = literals.iter().map(|l| quote!(#t == #l)).collect();
                quote!((#(#tests)||*))
            }
            Self::Or(branches) => {
                let branches = branches.iter().map(|branch| {
                    let tests = branch.iter().map(Self::cq_tokens);
                    quote!((#((#tests))&&*))
                });
                quote!((#(#branches)||*))
            }
        }
    }

    pub fn check(&self) -> syn::Result<()> {
        for term in self.terms() {
            if let Term::Literal(literal) = term {
                if !crate::equality::literal(literal) {
                    return Err(syn::Error::new_spanned(
                        literal,
                        "relational term requires a literal",
                    ));
                }
            }
        }
        match self {
            Self::In { literals, .. }
                if literals.is_empty() || literals.iter().any(|l| !crate::equality::literal(l)) =>
            {
                return Err(syn::Error::new_spanned(
                    self,
                    "membership requires a nonempty set of literals",
                ));
            }
            Self::Or(branches) => {
                if branches.len() < 2 || branches.iter().any(Vec::is_empty) {
                    return Err(syn::Error::new_spanned(
                        self,
                        "disjunction requires at least two nonempty parenthesized conjunctions",
                    ));
                }
                for c in branches.iter().flatten() {
                    c.check()?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}
