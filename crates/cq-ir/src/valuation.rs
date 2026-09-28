//! VO: a valuation plan — the order in which a query's valuations are built.
//!
//! CQ denotes the valuations `h : Vars → Dom` satisfying every atom. A plan
//! is a sequence of steps that extend a partial valuation: an atom step
//! binds the variables of one **subatom** — an atom restricted to some of
//! its columns, the rest written `_` — and constrains the already-bound
//! ones; a guard or binding step is placed once its variables are bound.
//! After each step the plan denotes the set of partial valuations on the
//! bound variables that satisfy every subatom seen so far, restricted to
//! their bound columns. The last step denotes `⟦Q⟧`.
//!
//! What the plan decides is only the order and the granularity: one subatom
//! per atom with all its columns is relation-at-a-time (a binary join
//! plan); one new variable per subatom is variable-at-a-time (generic
//! join); anything between is a Free Join split. Nothing here names an
//! operator, a key or a structure; the algebra below chooses them.
//!
//! A subatom names its source clause (`R@2`) so self-joins stay distinct.
//! It must list every variable of that clause that is already bound — a
//! plan may drop only unbound columns, never a constraint — and a clause
//! whose subatom adds no new variable is a consistency check, which erases
//! to nothing under set semantics.
//!
//! ```text
//! valuation {
//!     R@0(a, _);            // bind a from R
//!     T@2(a, _);            // check a against T
//!     R@0(a, b);            // bind b from R given a
//!     S@1(b, _);
//!     if (b != 5);
//!     S@1(b, c);  T@2(a, c);
//! }
//! ```
//!
//! ```text
//! Module  ::= CQ "valuation" "{" Step* "}"
//! Step    ::= Subatom ";" | "if" Expr ";" | "let" Pat [":" Type] "=" Expr ";"
//! Subatom ::= Ident "@" Int "(" (Ident | "_"),* ")"
//! ```

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, LitInt, Token, token};

use crate::cq::{self, normal::Kernel, rust};
use crate::{CommaList, symbol_name};

pub mod kw {
    syn::custom_keyword!(valuation);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Module {
    pub program: cq::Program,
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

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Step {
    #[parse(peek = Token![if])]
    Guard(cq::Filter, Token![;]),
    #[parse(peek = Token![let])]
    Let(Box<cq::Let>, Token![;]),
    Atom(Subatom, Token![;]),
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
    pub atoms: BTreeMap<usize, cq::Atom>,
}

pub mod contract {
    pub fn check(module: &super::Module) -> syn::Result<super::Analysis> {
        super::analyze(module)
    }

    pub fn well_formed(module: &super::Module) -> bool {
        check(module).is_ok()
    }
}

fn error<T>(spanned: &impl quote::ToTokens, message: impl std::fmt::Display) -> syn::Result<T> {
    Err(syn::Error::new_spanned(spanned, message))
}

fn analyze(module: &Module) -> syn::Result<Analysis> {
    let source = cq::Module {
        program: module.program.clone(),
    };
    cq::normal::check(&source)?;
    let query = &module.program.query;
    let mut analysis = Analysis::default();
    let mut clause_variables: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    let mut guards: Vec<&cq::Filter> = Vec::new();
    let mut lets: Vec<&cq::Let> = Vec::new();
    for (clause, item) in query.body.iter().enumerate() {
        match item {
            cq::BodyItem::Positive { atom } => {
                clause_variables.insert(clause, atom.variables().map(symbol_name).collect());
                analysis.atoms.insert(clause, atom.clone());
            }
            cq::BodyItem::Filter(filter) => guards.push(filter),
            cq::BodyItem::Let(binding) => lets.push(binding),
            _ => unreachable!("normal form"),
        }
    }
    let mut bound: Vec<String> = Vec::new();
    let mut covered: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    let mut guards_seen = 0usize;
    let mut lets_seen = 0usize;
    let is_bound = |bound: &[String], v: &str| bound.iter().any(|b| b == v);
    for step in &module.steps {
        analysis.bound_before.push(bound.clone());
        match step {
            Step::Atom(subatom, _) => {
                let clause = subatom.clause();
                let Some(variables) = clause_variables.get(&clause) else {
                    return error(subatom, format!("clause #{clause} is not an atom"));
                };
                let atom = &analysis.atoms[&clause];
                if symbol_name(&subatom.relation) != symbol_name(&atom.relation) {
                    return error(
                        &subatom.relation,
                        format!("clause #{clause} is `{}`", atom.relation),
                    );
                }
                if subatom.columns.len() != variables.len() {
                    return error(
                        subatom,
                        format!("clause #{clause} has {} columns", variables.len()),
                    );
                }
                let mut introduced = Vec::new();
                for (position, column) in subatom.columns.iter().enumerate() {
                    let expected = &variables[position];
                    match column {
                        Column::Variable(v) if symbol_name(v) == *expected => {
                            if !is_bound(&bound, expected) {
                                introduced.push(expected.clone());
                            }
                        }
                        Column::Variable(v) => {
                            return error(
                                v,
                                format!("column {position} of clause #{clause} is `{expected}`"),
                            );
                        }
                        Column::Dropped(_) if is_bound(&bound, expected) => {
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
                covered
                    .entry(clause)
                    .or_default()
                    .extend(subatom.variables().map(|(_, v)| symbol_name(v)));
                bound.extend(introduced.iter().cloned());
                analysis.introduces.push(introduced);
            }
            Step::Guard(filter, _) => {
                let Some(expected) = guards.get(guards_seen) else {
                    return error(filter, "more guards than the query has");
                };
                if expected.condition != filter.condition {
                    return error(filter, "guards must appear in query order");
                }
                guards_seen += 1;
                let environment: BTreeSet<String> = bound.iter().cloned().collect();
                let free = rust::free_variables(&filter.condition);
                if let Some(missing) = free.names.difference(&environment).next() {
                    if !free.opaque || clause_variables.values().flatten().any(|v| v == missing) {
                        return error(filter, format!("guard uses `{missing}` before it is bound"));
                    }
                }
                analysis.introduces.push(Vec::new());
            }
            Step::Let(binding, _) => {
                let Some(expected) = lets.get(lets_seen) else {
                    return error(binding, "more bindings than the query has");
                };
                if **expected != **binding {
                    return error(binding, "bindings must appear in query order");
                }
                lets_seen += 1;
                let environment: BTreeSet<String> = bound.iter().cloned().collect();
                let free = rust::free_variables(&binding.expression);
                if let Some(missing) = free.names.difference(&environment).next() {
                    if clause_variables.values().flatten().any(|v| v == missing) {
                        return error(
                            binding,
                            format!("binding uses `{missing}` before it is bound"),
                        );
                    }
                }
                let introduced: Vec<String> = rust::bindings(&binding.pattern)
                    .iter()
                    .map(symbol_name)
                    .collect();
                bound.extend(introduced.iter().cloned());
                analysis.introduces.push(introduced);
            }
        }
    }
    for (clause, variables) in &clause_variables {
        let seen = covered.get(clause).cloned().unwrap_or_default();
        if let Some(missing) = variables.iter().find(|v| !seen.contains(*v)) {
            return error(
                &module.valuation_token,
                format!("clause #{clause} never binds `{missing}`"),
            );
        }
    }
    if guards_seen != guards.len() {
        return error(&module.valuation_token, "every guard must be placed");
    }
    if lets_seen != lets.len() {
        return error(&module.valuation_token, "every binding must be placed");
    }
    Ok(analysis)
}

/// Erase the plan: the normal-form query it schedules.
pub fn erase(module: &Module) -> cq::Module {
    cq::Module {
        program: module.program.clone(),
    }
}

/// The kernel a plan computes; equal to the kernel of its query by construction.
pub fn kernel(module: &Module) -> Kernel {
    Kernel::of(&erase(module))
}
