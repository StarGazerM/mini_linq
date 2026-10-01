//! Valuation plans for `T X = bottom + Set X`.
//! Relational subatoms, equalities and semijoin reductions commute. They may
//! move earlier across opaque Rust lets/guards; the Rust chain retains written
//! order and all its written relational predecessors. This chain/Before check
//! is a sufficient refinement condition: a target may panic only on an input
//! where the written reference panics. I/O, mutation and evaluation counts are
//! outside this denotation. A split atom is a predecessor only once its full
//! correlated row has been checked. Rust guards are never reductions.
//!
//! ```text
//! Step ::= Subatom ";" | Ident "=" (Ident | Literal) ";"
//!        | "if" Expr ";" | "let" Pat [":" Type] "=" Expr ";"
//!        | "reduce" Ident "@" Int "with" Subatom ";"
//! ```

pub use cq_lang_cq as cq;

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, LitInt, Token, token};

pub use cq_syntax::{CommaList, symbol_name};

pub mod kw {
    syn::custom_keyword!(reduce);
    syn::custom_keyword!(valuation);
    syn::custom_keyword!(with);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Valuation {
    pub valuation_token: kw::valuation,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    #[parse(parse_steps)]
    #[to_tokens(|tokens, steps| quote::TokenStreamExt::append_all(tokens, steps))]
    pub steps: Vec<Step>,
}

fn parse_steps(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<Step>> {
    let mut steps = Vec::new();
    while !input.is_empty() {
        steps.push(input.parse()?);
    }
    Ok(steps)
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub enum Step {
    Equality(cq::Equality, Token![;]),
    Guard(cq_syntax::Filter, Token![;]),
    Let(Box<cq_syntax::Let>, Token![;]),
    Reduce(Reduce, Token![;]),
    Atom(Subatom, Token![;]),
}

impl syn::parse::Parse for Step {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        Ok(if input.peek(Token![if]) {
            Self::Guard(input.parse()?, input.parse()?)
        } else if input.peek(Token![let]) {
            Self::Let(input.parse()?, input.parse()?)
        } else if input.peek(kw::reduce) {
            Self::Reduce(input.parse()?, input.parse()?)
        } else if input.peek(Ident) && input.peek2(Token![=]) {
            Self::Equality(input.parse()?, input.parse()?)
        } else {
            Self::Atom(input.parse()?, input.parse()?)
        })
    }
}

/// A restriction of one clause's relation, applied before its first subatom.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Reduce {
    pub reduce_token: kw::reduce,
    pub relation: Ident,
    pub at: Token![@],
    pub clause: LitInt,
    pub reduction: Reduction,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Reduction {
    /// Semijoin with another clause on the variables the subatom lists.
    #[parse(peek = kw::with)]
    With(kw::with, Subatom),
}

impl Reduce {
    pub fn clause(&self) -> usize {
        self.clause.base10_parse().expect("clause index")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Subatom {
    pub relation: Ident,
    pub at: Token![@],
    pub clause: LitInt,
    #[syn(parenthesized)]
    pub paren: token::Paren,
    #[syn(in = paren)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<Column>,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Column {
    #[parse(peek = Token![_])]
    Dropped(Token![_]),
    Variable(Ident),
}

impl Subatom {
    pub fn clause(&self) -> usize {
        self.clause.base10_parse().expect("clause index")
    }

    /// Bound variables by column position.
    pub fn variables(&self) -> impl Iterator<Item = (usize, &Ident)> {
        self.columns
            .iter()
            .enumerate()
            .filter_map(|(i, c)| match c {
                Column::Variable(v) => Some((i, v)),
                Column::Dropped(_) => None,
            })
    }
}

/// What the contract establishes for later passes.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    /// Variables bound before each step, in binding order.
    pub bound_before: Vec<Vec<String>>,
    /// For atom steps: the variables the step newly binds.
    pub introduces: Vec<Vec<String>>,
    /// Clause index → the full atom of the normal-form query.
    pub atoms: BTreeMap<usize, cq_syntax::Atom>,
}

pub mod contract {
    /// Formal rule WF-VO: Check NF and the local valuation schedule with coverage and reduction premises.
    pub fn check(program: &super::Program) -> syn::Result<super::Analysis> {
        super::cq::normal::check(&program.cq)?;
        super::analyze(program)
    }
}

fn error<T>(
    spanned: &(impl quote::ToTokens + ?Sized),
    message: impl std::fmt::Display,
) -> syn::Result<T> {
    Err(syn::Error::new_spanned(
        spanned,
        format!("{}: {message}", spanned.to_token_stream()),
    ))
}

fn analyze(program: &Program) -> syn::Result<Analysis> {
    let body = &program.cq.query.body;
    let hypergraph = cq::hypergraph::Analysis::of(&program.cq);
    let observed = cq::normal::rust_observed(&program.cq);
    let mut first_atom = BTreeMap::new();
    let mut analysis = Analysis::default();
    let mut written_bound = BTreeSet::new();
    let mut rust_chain = Vec::new();
    let mut dependencies = BTreeMap::new();
    let mut equalities = BTreeMap::new();
    let mut let_bound = BTreeSet::new();
    for (clause, item) in body.iter().enumerate() {
        match item {
            cq::BodyItem::Positive { atom } => {
                analysis.atoms.insert(clause, atom.clone());
                for variable in atom.variables() {
                    first_atom.entry(symbol_name(variable)).or_insert(clause);
                }
                written_bound.extend(atom.variables().map(symbol_name));
            }
            cq::BodyItem::Equality(equality) => {
                equalities.insert(clause, equality);
            }
            cq::BodyItem::Filter(_) | cq::BodyItem::Let(_) => {
                let step = item.rust_step().expect("Rust step");
                dependencies.insert(clause, step.free_variables.dependencies(&written_bound));
                written_bound.extend(step.bindings.iter().map(symbol_name));
                let_bound.extend(step.bindings.iter().map(symbol_name));
                rust_chain.push(clause);
            }
            _ => unreachable!("normal form checked"),
        }
    }
    let atom_clause = |named: &dyn quote::ToTokens, relation: &Ident, clause: usize| {
        let Some(atom) = analysis.atoms.get(&clause) else {
            return error(named, format!("clause #{clause} is not an atom"));
        };
        if symbol_name(relation) != symbol_name(&atom.relation) {
            return error(relation, format!("clause #{clause} is `{}`", atom.relation));
        }
        Ok(atom.variables().map(symbol_name).collect::<Vec<_>>())
    };
    let columns = |subatom: &Subatom| {
        let variables = atom_clause(subatom, &subatom.relation, subatom.clause())?;
        if subatom.columns.len() != variables.len() {
            return error(
                subatom,
                format!(
                    "clause #{} has {} columns",
                    subatom.clause(),
                    variables.len()
                ),
            );
        }
        Ok(variables)
    };
    let mut bound = Vec::new();
    let mut seen_atoms = BTreeSet::new();
    let mut complete = BTreeSet::new();
    let mut placed_equalities = BTreeSet::new();
    let mut next_rust = 0;
    for step in &program.valuation.steps {
        analysis.bound_before.push(bound.clone());
        match step {
            Step::Atom(subatom, _) => {
                let clause = subatom.clause();
                let variables = columns(subatom)?;
                let mut introduced = Vec::new();
                for (position, column) in subatom.columns.iter().enumerate() {
                    let expected = &variables[position];
                    match column {
                        Column::Variable(v) if symbol_name(v) != *expected => {
                            return error(
                                v,
                                format!("column {position} of clause #{clause} is `{expected}`"),
                            );
                        }
                        Column::Variable(_) => {
                            if !bound.contains(expected) {
                                if let_bound.contains(expected) {
                                    return error(
                                        subatom,
                                        format!(
                                            "`{expected}` must be supplied by its Rust binding before this atom"
                                        ),
                                    );
                                }
                                introduced.push(expected.clone());
                            }
                        }
                        Column::Dropped(_) if bound.contains(expected) => {
                            return error(
                                subatom,
                                format!(
                                    "`{expected}` is bound; a subatom may drop only unbound columns"
                                ),
                            );
                        }
                        Column::Dropped(_) => {}
                    }
                }
                for variable in introduced.iter().filter(|v| observed.contains(*v)) {
                    if subatom.variables().count() != variables.len() {
                        return error(
                            subatom,
                            format!(
                                "partial subatom introduces Rust-observed `{variable}`; its full source row is required"
                            ),
                        );
                    }
                    if first_atom[variable] != clause {
                        return error(
                            subatom,
                            format!(
                                "Rust-observed `{variable}` must retain its first source atom's value"
                            ),
                        );
                    }
                }
                seen_atoms.insert(clause);
                if subatom.variables().count() == variables.len() {
                    complete.insert(clause);
                }
                bound.extend(introduced.iter().cloned());
                analysis.introduces.push(introduced);
            }
            Step::Equality(equality, _) => {
                let Some((&clause, _)) = equalities.iter().find(|(clause, expected)| {
                    !placed_equalities.contains(*clause) && ***expected == *equality
                }) else {
                    return error(
                        equality,
                        "every relational equality must appear exactly once from the query",
                    );
                };
                if let Some(variable) = equality
                    .variables()
                    .find(|v| !bound.contains(&symbol_name(v)))
                {
                    return error(
                        equality,
                        format!("equality uses `{variable}` before it is bound"),
                    );
                }
                placed_equalities.insert(clause);
                complete.insert(clause);
                analysis.introduces.push(Vec::new());
            }
            Step::Guard(_, _) | Step::Let(_, _) => {
                let Some(&clause) = rust_chain.get(next_rust) else {
                    return error(step, "more Rust steps than the query has");
                };
                let identical = match (step, &body[clause]) {
                    (Step::Guard(a, _), cq::BodyItem::Filter(b)) => a == b,
                    (Step::Let(a, _), cq::BodyItem::Let(b)) => a == b,
                    _ => false,
                };
                if !identical {
                    return error(
                        step,
                        "Rust lets and guards must keep their written relative order (chain)",
                    );
                }
                if let Some(missing) = dependencies[&clause].iter().find(|v| !bound.contains(*v)) {
                    return error(
                        step,
                        format!("Rust step uses `{missing}` before it is bound"),
                    );
                }
                for earlier in 0..clause {
                    if matches!(
                        &body[earlier],
                        cq::BodyItem::Positive { .. } | cq::BodyItem::Equality(_)
                    ) && !complete.contains(&earlier)
                    {
                        return error(
                            step,
                            format!(
                                "Before condition: relational clause #{earlier} must be completed before Rust clause #{clause}"
                            ),
                        );
                    }
                }
                let introduced = body[clause]
                    .rust_step()
                    .expect("Rust step")
                    .bindings
                    .iter()
                    .map(symbol_name)
                    .collect::<Vec<_>>();
                bound.extend(introduced.iter().cloned());
                analysis.introduces.push(introduced);
                next_rust += 1;
            }
            Step::Reduce(reduce, _) => {
                let clause = reduce.clause();
                let variables = atom_clause(reduce, &reduce.relation, clause)?;
                if seen_atoms.contains(&clause) {
                    return error(
                        reduce,
                        format!("clause #{clause} is reduced after its first subatom"),
                    );
                }
                let Reduction::With(_, subatom) = &reduce.reduction;
                let support = subatom.clause();
                let support_variables = columns(subatom)?;
                if support == clause {
                    return error(subatom, "a clause cannot reduce itself");
                }
                for (position, column) in subatom.columns.iter().enumerate() {
                    let expected = &support_variables[position];
                    let shared = variables
                        .iter()
                        .any(|variable| hypergraph.equivalent(variable, expected));
                    match column {
                        Column::Variable(v) if symbol_name(v) != *expected => {
                            return error(
                                v,
                                format!("column {position} of clause #{support} is `{expected}`"),
                            );
                        }
                        Column::Variable(_) if !shared => {
                            return error(
                                subatom,
                                format!(
                                    "`{expected}` is not shared with clause #{clause}; a reduction lists exactly the shared variables"
                                ),
                            );
                        }
                        Column::Dropped(_) if shared => {
                            return error(
                                subatom,
                                format!(
                                    "`{expected}` is shared with clause #{clause}; a reduction lists exactly the shared variables"
                                ),
                            );
                        }
                        _ => {}
                    }
                }
                analysis.introduces.push(Vec::new());
            }
        }
    }
    for (clause, atom) in &analysis.atoms {
        if !complete.contains(clause) {
            return error(
                atom,
                format!("clause #{clause} never completes its full correlated row"),
            );
        }
    }
    if let Some((_, missing)) = equalities
        .iter()
        .find(|(clause, _)| !placed_equalities.contains(clause))
    {
        return error(*missing, "every relational equality must be placed");
    }
    if next_rust != rust_chain.len() {
        return error(
            &body[rust_chain[next_rust]],
            "every Rust guard and binding must be placed",
        );
    }
    Ok(analysis)
}

/// A query and the complete schedule of its valuations.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Program {
    pub cq: cq::Program,
    pub valuation: Valuation,
}

pub use cq_syntax as syntax;
