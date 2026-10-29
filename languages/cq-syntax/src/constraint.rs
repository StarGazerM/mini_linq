//! Relational predicates shared by CQ and its lower representations.
//! Only explicit `where` clauses construct these predicates from source.
//! Closed Rust operands are evaluated once on the host. Their internal wire
//! spelling is `@(expression)`; CQ spells them as ordinary Rust expressions.
//!
//! DESIGN contract7-surface
//! Owns explicit `where` syntax and query-closed Rust operands (contract 7, CQ-Where/CQ-Closed).
//! Source: doc/PLANNING-REDESIGN.md Contract 7 and Architecture; doc/formal/CONFORMANCE.md.
//! Rust `if/let/for` remains opaque and total by premise; only `where` supplies relational facts.
//! DO NOT recognize comparisons inside `if`, split Rust, inspect types, or infer Eq/Ord from spelling.
//! Closed operands are host-once values; their query closedness is checked after scope resolution.

use crate::{CommaList, Equality, EqualityValue, symbol_name};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::{Ident, Token, parse::ParseStream};

mod kw {
    syn::custom_keyword!(between);
    syn::custom_keyword!(and);
    syn::custom_keyword!(or);
}

/// Parse the relational language after CQ's `where` keyword. Rust operands are
/// syntax only: query closedness is checked once the complete scope is known.
pub fn parse_where(input: ParseStream<'_>) -> syn::Result<Constraint> {
    let mut tokens = TokenStream::new();
    while !input.is_empty()
        && !input.peek(Token![;])
        && !(input.peek(crate::algebra::kw::group) && input.peek2(syn::token::Bracket))
    {
        if input.peek(Token![,]) {
            if let Ok(constraint) = predicate(tokens.clone()) {
                return Ok(constraint);
            }
        }
        tokens.extend([input.parse::<proc_macro2::TokenTree>()?]);
    }
    predicate(tokens)
}

fn predicate(tokens: TokenStream) -> syn::Result<Constraint> {
    use proc_macro2::{Delimiter, TokenTree};
    if let Ok(expression) = syn::parse2::<syn::Expr>(tokens.clone()) {
        return predicate_expression(expression);
    }
    let trees: Vec<_> = tokens.clone().into_iter().collect();
    // Boolean syntax belongs to the relational language, not to Rust guards.
    for operator in ['|', '&'] {
        if let Some(index) = trees.windows(2).position(|pair| {
            matches!(&pair[0], TokenTree::Punct(p) if p.as_char() == operator
                && p.spacing() == proc_macro2::Spacing::Joint)
                && matches!(&pair[1], TokenTree::Punct(p) if p.as_char() == operator)
        }) {
            let left = predicate(trees[..index].iter().cloned().collect())?;
            let right = predicate(trees[index + 2..].iter().cloned().collect())?;
            return Ok(boolean(left, right, operator == '|'));
        }
    }
    if let [TokenTree::Group(group)] = trees.as_slice() {
        if group.delimiter() == Delimiter::Parenthesis {
            return predicate(group.stream());
        }
    }
    if let Some(index) = trees
        .iter()
        .position(|tree| matches!(tree, TokenTree::Ident(id) if id == "in"))
    {
        let term = source_term(syn::parse2(trees[..index].iter().cloned().collect())?);
        require_variable(&term)?;
        let values: syn::Expr = syn::parse2(trees[index + 1..].iter().cloned().collect())?;
        return match values {
            syn::Expr::Array(array) => {
                let mut values: Vec<_> = array.elems.into_iter().map(source_term).collect();
                if values.is_empty() {
                    return Ok(Constraint::Compare {
                        left: term.clone(),
                        comparison: Comparison::Ne(Default::default()),
                        right: term,
                    });
                }
                if values.iter().all(|value| matches!(value, Term::Literal(_))) {
                    return Ok(Constraint::In {
                        term,
                        literals: values
                            .into_iter()
                            .map(|value| {
                                let Term::Literal(value) = value else {
                                    unreachable!()
                                };
                                *value
                            })
                            .collect(),
                    });
                }
                if values.len() == 1 {
                    return Ok(Constraint::Compare {
                        left: term,
                        comparison: Comparison::Eq(Default::default()),
                        right: values.remove(0),
                    });
                }
                Ok(Constraint::Or(
                    values
                        .into_iter()
                        .map(|right| {
                            vec![Constraint::Compare {
                                left: term.clone(),
                                comparison: Comparison::Eq(Default::default()),
                                right,
                            }]
                        })
                        .collect(),
                ))
            }
            syn::Expr::Range(range) => {
                let (Some(low), Some(high)) = (range.start, range.end) else {
                    return Err(syn::Error::new_spanned(
                        tokens,
                        "membership requires both range bounds",
                    ));
                };
                let (low, high) = (source_term(*low), source_term(*high));
                if matches!(range.limits, syn::RangeLimits::Closed(_)) {
                    Ok(Constraint::Between { term, low, high })
                } else {
                    Ok(Constraint::Or(vec![vec![
                        Constraint::Compare {
                            left: term.clone(),
                            comparison: Comparison::Ge(Default::default()),
                            right: low,
                        },
                        Constraint::Compare {
                            left: term,
                            comparison: Comparison::Lt(Default::default()),
                            right: high,
                        },
                    ]]))
                }
            }
            _ => Err(syn::Error::new_spanned(
                values,
                "expected `[terms]`, `low..high`, or `low..=high`",
            )),
        };
    }
    predicate_expression(syn::parse2(tokens)?)
}

fn predicate_expression(expression: syn::Expr) -> syn::Result<Constraint> {
    let expression = match expression {
        syn::Expr::Paren(paren) => return predicate_expression(*paren.expr),
        syn::Expr::Group(group) => return predicate_expression(*group.expr),
        expression => expression,
    };
    let syn::Expr::Binary(binary) = expression else {
        return Err(syn::Error::new_spanned(
            expression,
            "expected a relational comparison",
        ));
    };
    if matches!(binary.op, syn::BinOp::And(_) | syn::BinOp::Or(_)) {
        let disjunction = matches!(binary.op, syn::BinOp::Or(_));
        return Ok(boolean(
            predicate_expression(*binary.left)?,
            predicate_expression(*binary.right)?,
            disjunction,
        ));
    }
    let comparison = match binary.op {
        syn::BinOp::Eq(_) => Comparison::Eq(Default::default()),
        syn::BinOp::Ne(op) => Comparison::Ne(op),
        syn::BinOp::Lt(op) => Comparison::Lt(op),
        syn::BinOp::Le(op) => Comparison::Le(op),
        syn::BinOp::Gt(op) => Comparison::Gt(op),
        syn::BinOp::Ge(op) => Comparison::Ge(op),
        _ => {
            return Err(syn::Error::new_spanned(
                binary.op,
                "expected a relational comparison operator",
            ));
        }
    };
    let left = source_term(*binary.left);
    require_variable(&left)?;
    Ok(Constraint::Compare {
        left,
        comparison,
        right: source_term(*binary.right),
    })
}

fn boolean(left: Constraint, right: Constraint, disjunction: bool) -> Constraint {
    if disjunction {
        let branches = |value| match value {
            Constraint::Or(branches) => branches,
            value => vec![vec![value]],
        };
        let mut result = branches(left);
        result.extend(branches(right));
        literal_membership(result)
    } else {
        let conjunction = |value| match value {
            Constraint::Or(mut branches) if branches.len() == 1 => branches.remove(0),
            value => vec![value],
        };
        let mut result = conjunction(left);
        result.extend(conjunction(right));
        Constraint::Or(vec![compact_bounds(result)])
    }
}

// These are relational normalizations only. Rust guards never enter this path.
fn compact_bounds(constraints: Vec<Constraint>) -> Vec<Constraint> {
    let mut result = Vec::new();
    let mut constraints = constraints.into_iter().peekable();
    while let Some(constraint) = constraints.next() {
        if let Constraint::Compare {
            left,
            comparison: Comparison::Ge(_),
            right: low,
        } = &constraint
        {
            if let Some(Constraint::Compare {
                left: other,
                comparison: Comparison::Le(_),
                right: high,
            }) = constraints.peek()
            {
                if left == other {
                    result.push(Constraint::Between {
                        term: left.clone(),
                        low: low.clone(),
                        high: high.clone(),
                    });
                    constraints.next();
                    continue;
                }
            }
        }
        result.push(constraint);
    }
    result
}

fn literal_membership(branches: Vec<Vec<Constraint>>) -> Constraint {
    let mut membership = None;
    let mut literals = CommaList::new();
    for branch in &branches {
        let term = match branch.as_slice() {
            [
                Constraint::Compare {
                    left,
                    comparison: Comparison::Eq(_),
                    right: Term::Literal(literal),
                },
            ] => {
                literals.push((**literal).clone());
                left
            }
            [
                Constraint::In {
                    term,
                    literals: values,
                },
            ] => {
                literals.extend(values.iter().cloned());
                term
            }
            _ => return Constraint::Or(branches),
        };
        if membership.as_ref().is_some_and(|previous| previous != term) {
            return Constraint::Or(branches);
        }
        membership = Some(term.clone());
    }
    Constraint::In {
        term: membership.expect("a disjunction has operands"),
        literals,
    }
}

fn require_variable(term: &Term) -> syn::Result<()> {
    if matches!(term, Term::Variable(_)) {
        Ok(())
    } else {
        Err(syn::Error::new_spanned(
            term,
            "the left operand of `where` must be a query variable",
        ))
    }
}

fn source_term(expression: syn::Expr) -> Term {
    match expression {
        syn::Expr::Paren(paren) => source_term(*paren.expr),
        syn::Expr::Group(group) => source_term(*group.expr),
        expression => {
            if let Some(variable) = crate::rust::variable(&expression) {
                Term::Variable(variable.clone())
            } else if crate::equality::literal(&expression) {
                Term::Literal(Box::new(expression))
            } else {
                Term::Closed(expression)
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    Variable(Ident),
    Closed(syn::Expr),
    Literal(Box<syn::Expr>),
}

impl syn::parse::Parse for Term {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(Token![@]) {
            input.parse::<Token![@]>()?;
            let body;
            syn::parenthesized!(body in input);
            return Ok(Self::Closed(body.parse()?));
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
            Self::Closed(v) => tokens.extend(quote!(@(#v))),
            Self::Literal(v) => v.to_tokens(tokens),
        }
    }
}

impl Term {
    /// Borrow a relational operand; captured parameters may already be borrowed.
    pub fn reference(&self, parameters: &BTreeMap<String, Ident>) -> TokenStream {
        match self {
            Self::Variable(v) => quote!(#v),
            Self::Closed(v) => match parameters.get(&v.to_token_stream().to_string()) {
                Some(p) => quote!(#p),
                None => quote!(&(#v)),
            },
            Self::Literal(v) => quote!(&(#v)),
        }
    }

    pub fn string_literal(&self) -> bool {
        matches!(self, Self::Literal(l) if matches!(l.as_ref(),
            syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(_), .. })))
    }

    pub fn resolve(&mut self, logical: &BTreeSet<String>) {
        if let Self::Variable(v) = self {
            if !logical.contains(&symbol_name(v)) {
                *self = Self::Closed(syn::parse_quote!(#v));
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

    /// Emit the shared Eq/Ord obligations for borrowed relational operands.
    pub fn expression(self, left: TokenStream, right: TokenStream, string: bool) -> TokenStream {
        let equality = if string {
            quote!(::cq_storage::relational_string_eq(#left, #right))
        } else {
            quote!(::cq_storage::relational_eq(#left, #right))
        };
        match self {
            Self::Eq(_) => equality,
            Self::Ne(_) => quote!(!#equality),
            _ => {
                let ordering = if string {
                    quote!(::cq_storage::relational_str_cmp(#left, #right))
                } else {
                    quote!(::cq_storage::relational_cmp(#left, #right))
                };
                match self {
                    Self::Lt(_) => quote!(#ordering.is_lt()),
                    Self::Le(_) => quote!(!#ordering.is_gt()),
                    Self::Gt(_) => quote!(#ordering.is_gt()),
                    Self::Ge(_) => quote!(!#ordering.is_lt()),
                    _ => unreachable!(),
                }
            }
        }
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
            return Ok(Self::Or(branches));
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
    /// Formal rule E-Domain: Project every disjunct's query-constant restrictions onto a common variable, retaining the exact predicate.
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
                            let constant_domain = match c {
                                Self::Compare {
                                    left: Term::Variable(_),
                                    right,
                                    ..
                                } => right.constant(),
                                Self::Compare {
                                    left,
                                    right: Term::Variable(_),
                                    ..
                                } => left.constant(),
                                Self::Between {
                                    term: Term::Variable(_),
                                    low,
                                    high,
                                } => low.constant() && high.constant(),
                                Self::In {
                                    term: Term::Variable(_),
                                    ..
                                } => true,
                                _ => false,
                            };
                            constant_domain && c.variables().all(|v| symbol_name(v) == name)
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

    pub fn parameters(&self) -> impl Iterator<Item = &syn::Expr> {
        self.terms().into_iter().filter_map(|t| match t {
            Term::Closed(v) => Some(v),
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

    /// Resolve source operands; the predicate's left column must stay a query
    /// variable so the CQ scope checker diagnoses an unbound column.
    pub fn resolve(&mut self, logical: &BTreeSet<String>) {
        match self {
            Self::Compare { right, .. } => right.resolve(logical),
            Self::Between { low, high, .. } => {
                low.resolve(logical);
                high.resolve(logical);
            }
            Self::In { .. } => {}
            Self::Or(branches) => {
                for constraint in branches.iter_mut().flatten() {
                    constraint.resolve(logical);
                }
            }
        }
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
            Term::Closed(_) => return None,
        };
        Some(Equality {
            left: left.clone(),
            eq_token: *eq_token,
            right,
        })
    }

    /// Equalities required by this predicate, excluding alternatives.
    pub fn equalities(&self) -> Vec<Equality> {
        match self {
            Self::Or(branches) if branches.len() == 1 => {
                branches[0].iter().flat_map(Self::equalities).collect()
            }
            _ => self.equality().into_iter().collect(),
        }
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

    /// Source `where` spelling for normalized relational predicates.
    pub fn cq_tokens(&self) -> TokenStream {
        let term = |t: &Term| match t {
            Term::Variable(v) => quote!(#v),
            Term::Closed(p) => quote!((#p)),
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
                quote!(#t in #low..=#high)
            }
            Self::In { term: t, literals } => {
                let t = term(t);
                quote!(#t in [#literals])
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
                if branches.is_empty() || branches.iter().any(Vec::is_empty) {
                    return Err(syn::Error::new_spanned(
                        self,
                        "predicate requires nonempty conjunctions",
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
