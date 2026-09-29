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
//! A **reduction** restricts a clause's relation before the clause is ever
//! joined. `reduce R@0 with S@1(b, _)` keeps the tuples of `R@0` that agree
//! with some tuple of `S@1` on the variables the two clauses share — the
//! subatom lists exactly those, `_` elsewhere. `reduce R@0 if (e)` keeps the
//! tuples satisfying `e`, one of the query's guards over `R@0`'s own
//! variables; that guard is thereby placed and appears as no step. A
//! reduction may stand anywhere before the reduced clause's first subatom
//! step. Under set semantics it changes nothing the plan denotes — it is a
//! Yannakakis semijoin, or a guard pushed onto one atom, written down where
//! the algebra will apply it — so the erasure ignores reductions.
//!
//! ```text
//! valuation {
//!     reduce R@0 with T@2(a, _); // keep the R tuples whose a occurs in T
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
//! Module    ::= CQ "valuation" "{" Step* "}"
//! Step      ::= Subatom ";" | "if" Expr ";" | "let" Pat [":" Type] "=" Expr ";"
//!             | "reduce" Ident "@" Int Reduction ";"
//! Reduction ::= "with" Subatom | "if" Expr
//! Subatom   ::= Ident "@" Int "(" (Ident | "_"),* ")"
//! ```

#[cfg(feature = "reference-prefix")]
pub use cq_lang_cq as cq;

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, LitInt, Token, token};

#[cfg(feature = "reference-prefix")]
use cq::normal::Kernel;
use cq_syntax::rust;
pub use cq_syntax::{CommaList, symbol_name};

pub mod kw {
    syn::custom_keyword!(reduce);
    syn::custom_keyword!(valuation);
    syn::custom_keyword!(with);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Section {
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
    Guard(cq_syntax::Filter, Token![;]),
    #[parse(peek = Token![let])]
    Let(Box<cq_syntax::Let>, Token![;]),
    #[parse(peek = kw::reduce)]
    Reduce(Reduce, Token![;]),
    Atom(Subatom, Token![;]),
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
    /// One of the query's guards, over the reduced clause's variables.
    Guard(cq_syntax::Filter),
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

/// A normalized source clause; indices retain their source ordering.
#[derive(Clone, Debug)]
pub enum Clause {
    Atom(cq_syntax::Atom),
    Guard(cq_syntax::Filter),
    Let(Box<cq_syntax::Let>),
}

/// The normalized source clauses scheduled by a valuation section.
pub trait CqEnv {
    fn clauses(&self) -> syn::Result<Vec<Clause>>;
}

pub mod contract {
    pub fn check(
        section: &super::Section,
        env: &impl super::CqEnv,
    ) -> syn::Result<super::Analysis> {
        super::analyze(section, env)
    }
    pub fn well_formed(section: &super::Section, env: &impl super::CqEnv) -> bool {
        check(section, env).is_ok()
    }
    #[cfg(feature = "reference-prefix")]
    pub fn check_module(module: &super::Module) -> syn::Result<super::Analysis> {
        let source = super::cq::Module {
            program: module.program.clone(),
        };
        super::cq::normal::check(&source)?;
        check(&module.section, &source)
    }
    #[cfg(feature = "reference-prefix")]
    pub fn well_formed_module(module: &super::Module) -> bool {
        check_module(module).is_ok()
    }
}

fn error<T>(
    spanned: &(impl quote::ToTokens + ?Sized),
    message: impl std::fmt::Display,
) -> syn::Result<T> {
    Err(syn::Error::new_spanned(spanned, message))
}

fn analyze(module: &Section, env: &impl CqEnv) -> syn::Result<Analysis> {
    let clauses = env.clauses()?;
    let mut analysis = Analysis::default();
    let mut clause_variables: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    let mut guards: Vec<&cq_syntax::Filter> = Vec::new();
    let mut lets: Vec<&cq_syntax::Let> = Vec::new();
    for (clause, item) in clauses.iter().enumerate() {
        match item {
            Clause::Atom(atom) => {
                if !atom.is_normalized() {
                    return error(atom, "valuation requires normalized atoms");
                }
                clause_variables.insert(clause, atom.variables().map(symbol_name).collect());
                analysis.atoms.insert(clause, atom.clone());
            }
            Clause::Guard(filter) => guards.push(filter),
            Clause::Let(binding) => lets.push(binding),
        }
    }
    let let_bound: Vec<String> = lets
        .iter()
        .flat_map(|binding| rust::bindings(&binding.pattern))
        .map(|v| symbol_name(&v))
        .collect();
    let is_variable = |v: &str| {
        clause_variables.values().flatten().any(|c| c == v) || let_bound.iter().any(|c| c == v)
    };
    // The clause a subatom or a reduction names: an atom of that relation.
    let atom_clause = |named: &dyn quote::ToTokens, relation: &Ident, clause: usize| {
        let Some(variables) = clause_variables.get(&clause) else {
            return error(named, format!("clause #{clause} is not an atom"));
        };
        let atom = &analysis.atoms[&clause];
        if symbol_name(relation) != symbol_name(&atom.relation) {
            return error(relation, format!("clause #{clause} is `{}`", atom.relation));
        }
        Ok(variables)
    };
    let columns = |subatom: &Subatom| {
        let clause = subatom.clause();
        let variables = atom_clause(subatom, &subatom.relation, clause)?;
        if subatom.columns.len() != variables.len() {
            return error(
                subatom,
                format!("clause #{clause} has {} columns", variables.len()),
            );
        }
        Ok(variables)
    };
    // The guards the reductions place; a guard step is then the next guard
    // the reductions leave, in query order.
    let mut consumed = vec![false; guards.len()];
    for step in &module.steps {
        let Step::Reduce(reduce, _) = step else {
            continue;
        };
        let Reduction::Guard(filter) = &reduce.reduction else {
            continue;
        };
        let unplaced = guards
            .iter()
            .zip(&consumed)
            .position(|(guard, taken)| !taken && guard.condition == filter.condition);
        match unplaced {
            Some(guard) => consumed[guard] = true,
            None => {
                return error(
                    filter,
                    "a reduction guard is one of the query's guards, once",
                );
            }
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
                let variables = columns(subatom)?;
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
            Step::Reduce(reduce, _) => {
                let clause = reduce.clause();
                let variables = atom_clause(reduce, &reduce.relation, clause)?;
                if covered.contains_key(&clause) {
                    return error(
                        reduce,
                        format!("clause #{clause} is reduced after its first subatom"),
                    );
                }
                match &reduce.reduction {
                    Reduction::With(_, subatom) => {
                        let support = subatom.clause();
                        let support_variables = columns(subatom)?;
                        if support == clause {
                            return error(subatom, "a clause cannot reduce itself");
                        }
                        for (position, column) in subatom.columns.iter().enumerate() {
                            let expected = &support_variables[position];
                            let shared = variables.contains(expected);
                            match column {
                                Column::Variable(v) if symbol_name(v) != *expected => {
                                    return error(
                                        v,
                                        format!(
                                            "column {position} of clause #{support} is `{expected}`"
                                        ),
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
                                Column::Variable(_) | Column::Dropped(_) => {}
                            }
                        }
                    }
                    Reduction::Guard(filter) => {
                        // Names that are not query variables — callees,
                        // constants — are Rust's to resolve.
                        let free = rust::free_variables(&filter.condition);
                        if let Some(missing) = free
                            .names
                            .iter()
                            .find(|v| !variables.contains(*v) && is_variable(v))
                        {
                            return error(
                                filter,
                                format!(
                                    "reduction guard uses `{missing}`, not a variable of clause #{clause}"
                                ),
                            );
                        }
                    }
                }
                analysis.introduces.push(Vec::new());
            }
            Step::Guard(filter, _) => {
                while consumed.get(guards_seen).is_some_and(|taken| *taken) {
                    guards_seen += 1;
                }
                let Some(expected) = guards.get(guards_seen) else {
                    return error(filter, "more guards than the query has");
                };
                if expected.condition != filter.condition {
                    return error(filter, "guards must appear in query order");
                }
                guards_seen += 1;
                let free = rust::free_variables(&filter.condition);
                if let Some(missing) = free
                    .names
                    .iter()
                    .find(|v| !bound.contains(*v) && is_variable(v))
                {
                    return error(filter, format!("guard uses `{missing}` before it is bound"));
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
    while consumed.get(guards_seen).is_some_and(|taken| *taken) {
        guards_seen += 1;
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
#[cfg(feature = "reference-prefix")]
pub fn erase(module: &Module) -> cq::Module {
    cq::Module {
        program: module.program.clone(),
    }
}

/// The kernel a plan computes; equal to the kernel of its query by construction.
#[cfg(feature = "reference-prefix")]
pub fn kernel(module: &Module) -> Kernel {
    Kernel::of(&erase(module))
}

/// Shared syntax used by this language.
pub use cq_syntax as syntax;

#[cfg(feature = "reference-prefix")]
pub mod reference;
#[cfg(feature = "reference-prefix")]
pub use reference::Module;
