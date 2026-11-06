//! Valuation plans for set-valued comprehensions.
//! Rust lets, generators and guards are opaque, pure and total by premise.
//! Their expressions remain unchanged and may be placed wherever their free
//! variables are bound. Only explicit relational constraints support reductions.
//!
//! ```text
//! Step ::= Subatom ";" | "!" Atom ";" | Constraint ";"
//!        | "if" Expr ";" | "let" Pat [":" Type] "=" Expr ";"
//!        | "reduce" Ident "@" Int "with" Subatom ";"
//!        | "restrict" Ident "@" Int "where" Constraint ";"
//! ```
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

//! VO records block @path(columns) hoisted/correlated { steps } | { steps }; or else arms, retaining recursive branch steps and reduction occurrence paths R@c.j.k.
//! Its contract checks coverage and interface scope recursively.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Union siblings are unordered; fallback retains its relational anti steps.
//! Rust expressions are never recognized as relational predicates. Only differing
//! literals on one interface column prove union arms disjoint.
//!
pub use cq_lang_cq as cq;

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, Token, token};

pub use cq_syntax::{CommaList, Occurrence, symbol_name};
pub mod blocks;
pub use blocks::Placement;
pub mod evidence;
pub use evidence::Evidence;
pub mod restrict;
pub use restrict::Restrict;
pub mod order;
/// Recorded ordering inputs shared with CQ; printing never reconstructs them.
pub mod ordering {
    pub use cq_lang_cq::ordering::*;
}

pub mod kw {
    syn::custom_keyword!(reduce);
    syn::custom_keyword!(restrict);
    syn::custom_keyword!(valuation);
    syn::custom_keyword!(with);
    syn::custom_keyword!(block);
    syn::custom_keyword!(check);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Valuation {
    pub valuation_token: kw::valuation,
    #[parse(evidence::parse_optional)]
    pub evidence: Option<Evidence>,
    #[parse(ordering::parse_optional)]
    pub ordering: Option<ordering::OrderingFacts>,
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

// Syntax-tree enum, like syn's AST; boxing would change public construction.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Block(blocks::Block, Token![;]),
    Equality(cq::Equality, Token![;]),
    Constraint(cq::Constraint, Token![;]),
    Negation(cq::Negation, Token![;]),
    Guard(cq_syntax::Filter, Token![;]),
    Let(Box<cq_syntax::Let>, Token![;]),
    For(Box<cq_syntax::For>, Token![;]),
    Reduce(Reduce, Token![;]),
    Restrict(Restrict, Token![;]),
    Atom(Subatom, Token![;]),
}

impl quote::ToTokens for Step {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        tokens.extend(match self {
            Self::Block(value, semi) => quote::quote!(#value #semi),
            Self::Equality(value, semi) => quote::quote!(#value #semi),
            Self::Constraint(value, semi) => quote::quote!(where #value #semi),
            Self::Negation(value, semi) => quote::quote!(#value #semi),
            Self::Guard(value, semi) => quote::quote!(#value #semi),
            Self::Let(value, semi) => quote::quote!(#value #semi),
            Self::For(value, semi) => quote::quote!(#value #semi),
            Self::Reduce(value, semi) => quote::quote!(#value #semi),
            Self::Restrict(value, semi) => quote::quote!(#value #semi),
            Self::Atom(value, semi) => quote::quote!(#value #semi),
        });
    }
}

impl syn::parse::Parse for Step {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        Ok(if input.peek(kw::block) {
            Self::Block(input.parse()?, input.parse()?)
        } else if input.peek(Token![!]) {
            Self::Negation(input.parse()?, input.parse()?)
        } else if input.peek(Token![where]) {
            input.parse::<Token![where]>()?;
            let constraint: cq::Constraint = input.parse()?;
            let semi = input.parse()?;
            match constraint.equality() {
                Some(equality) => Self::Equality(equality, semi),
                None => Self::Constraint(constraint, semi),
            }
        } else if input.peek(Token![if]) {
            Self::Guard(input.parse()?, input.parse()?)
        } else if input.peek(Token![let]) {
            Self::Let(input.parse()?, input.parse()?)
        } else if input.peek(Token![for]) {
            Self::For(input.parse()?, input.parse()?)
        } else if input.peek(kw::reduce) {
            Self::Reduce(input.parse()?, input.parse()?)
        } else if input.peek(kw::restrict) {
            Self::Restrict(input.parse()?, input.parse()?)
        } else if input.peek(Ident) && input.peek2(Token![@]) {
            Self::Atom(input.parse()?, input.parse()?)
        } else {
            Self::Equality(input.parse()?, input.parse()?)
        })
    }
}

/// A restriction of one clause's relation, normally before its first subatom.
/// A hoisted block's finite prefix domain delays reductions until VO binds
/// that block's free columns; a late reduction also filters the current prefix.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Reduce {
    pub reduce_token: kw::reduce,
    pub relation: Ident,
    pub at: Token![@],
    pub clause: Occurrence,
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
        self.clause.clause()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Subatom {
    pub relation: Ident,
    pub at: Token![@],
    pub clause: Occurrence,
    #[syn(parenthesized)]
    pub paren: token::Paren,
    #[syn(in = paren)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<Column>,
    /// A standalone membership phase, never an alternate introducing provider.
    pub check: Option<kw::check>,
    #[parse(cq_syntax::algebra::parse_moved_after)]
    pub moved_after: Option<cq_syntax::algebra::MovedAfter>,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Column {
    #[parse(peek = Token![_])]
    Dropped(Token![_]),
    Variable(Ident),
}

impl Subatom {
    pub fn clause(&self) -> usize {
        self.clause.clause()
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
    pub branches: BTreeMap<usize, Vec<Analysis>>,
    /// Binding-only facts for independent ordering checks, indexed by step.
    pub order: Vec<order::StepFacts>,
    /// For each hoisted block that takes a finite prefix domain: its free
    /// columns the scope's seed does not provide. VO provides them before the
    /// block and before any reduction that reads it.
    pub domain_requires: BTreeMap<usize, BTreeSet<String>>,
    /// Variables bound before each step, in binding order.
    pub bound_before: Vec<Vec<String>>,
    /// Variables the scope's seed and its earlier steps provide before each
    /// step. It equals `bound_before` except in a hoisted arm without a finite
    /// domain, whose free columns are visible but provided by no seed.
    pub provided_before: Vec<BTreeSet<String>>,
    /// For atom steps: the variables the step newly binds.
    pub introduces: Vec<Vec<String>>,
    /// Clause index → the full atom of the normal-form query.
    pub atoms: BTreeMap<usize, cq_syntax::Atom>,
}

pub mod contract {
    /// Formal rule WF-VO: Check NF and the local valuation schedule with coverage and reduction premises.
    pub fn check(program: &super::Program) -> syn::Result<super::Analysis> {
        let analysis = super::order::analyze(program)?;
        super::order::check(program, &analysis)?;
        Ok(analysis)
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
    if program.valuation.ordering != program.cq.ordering {
        return error(&program.valuation, "VO ordering inputs must preserve the recorded CQ facts");
    }
    if let Some(evidence) = &program.valuation.evidence {
        evidence.check(&program.cq)?;
    }
    analyze_reference(&program.cq, &program.valuation.steps)
}

fn analyze_reference(cq: &cq::Program, steps: &[Step]) -> syn::Result<Analysis> {
    let schema = cq
        .inputs()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    analyze_steps(
        &cq.query.body,
        steps,
        Vec::new(),
        BTreeSet::new(),
        None,
        &schema,
        &BTreeMap::new(),
    )
}

/// Formal rule WF-VO-Rust: Place each unchanged Rust expression exactly once where its free variables are bound.
/// Formal rule WF-BlockPlacement: Hoisted arms are Rust-free and a hoisted block taking a finite prefix domain follows its free columns; correlated arms require their free prefix, and repeated reads preserve placement.
fn analyze_steps(
    body: &cq_syntax::CommaList<cq::BodyItem>,
    steps: &[Step],
    context: Vec<String>,
    seed: BTreeSet<String>,
    parent: Option<(Occurrence, usize)>,
    schema: &BTreeMap<String, &cq::RelationDecl>,
    inherited_types: &BTreeMap<String, syn::Type>,
) -> syn::Result<Analysis> {
    let types = cq::blocks::types(schema, body, inherited_types)?;
    for step in steps {
        match step {
            Step::Atom(a, _) => a.clause.check()?,
            Step::Restrict(r, _) => r.clause.check()?,
            Step::Block(b, _) => b.source.clause.check()?,
            Step::Reduce(r, _) => {
                r.clause.check()?;
                let Reduction::With(_, s) = &r.reduction;
                s.clause.check()?;
            }
            _ => (),
        }
    }
    let hypergraph = cq::hypergraph::Analysis::of_body(body, &context.iter().cloned().collect());
    let mut analysis = Analysis::default();
    let mut written_bound: BTreeSet<_> = context.iter().cloned().collect();
    let mut block_facts = BTreeMap::new();
    let mut dependencies = BTreeMap::new();
    let mut equalities = BTreeMap::new();
    let mut constraints = BTreeMap::new();
    let mut negations = BTreeMap::new();
    let logical: BTreeSet<_> = cq::blocks::bound(body)
        .into_keys()
        .chain(context.iter().cloned())
        .collect();
    let mut let_bound = BTreeSet::new();
    for (clause, item) in body.iter().enumerate() {
        match item {
            cq::BodyItem::Block(block) => {
                let facts = block.analyze(&written_bound);
                let atom = cq_syntax::Atom {
                    relation: syn::Ident::new("block", proc_macro2::Span::call_site()),
                    paren_token: Default::default(),
                    args: facts
                        .free
                        .values()
                        .chain(facts.interface.values())
                        .map(|v| -> syn::Expr { syn::parse_quote!(#v) })
                        .collect(),
                };
                analysis.atoms.insert(clause, atom);
                if !facts.rust
                    && block.needs_domain(&facts)
                    && steps.iter().any(|step| {
                        matches!(step, Step::Block(planned, _)
                            if planned.source.clause() == clause
                                && planned.placement == blocks::Placement::Hoisted)
                    })
                {
                    let required: BTreeSet<_> = facts
                        .free
                        .keys()
                        .filter(|name| !seed.contains(*name))
                        .cloned()
                        .collect();
                    if !required.is_empty() {
                        analysis.domain_requires.insert(clause, required);
                    }
                }
                if facts.rust {
                    let_bound.extend(facts.interface.keys().cloned());
                }
                written_bound.extend(facts.interface.keys().cloned());
                block_facts.insert(clause, facts);
            }
            cq::BodyItem::Positive { atom } => {
                analysis.atoms.insert(clause, atom.clone());
                written_bound.extend(atom.variables().map(symbol_name));
            }
            cq::BodyItem::Equality(equality) => {
                equalities.insert(clause, equality);
            }
            cq::BodyItem::Constraint(c) => {
                constraints.insert(clause, c);
            }
            cq::BodyItem::Negation(negation) => {
                negations.insert(clause, negation);
            }
            cq::BodyItem::Filter(_) | cq::BodyItem::Let(_) | cq::BodyItem::For(_) => {
                let step = item.rust_step().expect("Rust step");
                dependencies.insert(clause, step.free_variables.dependencies(&written_bound));
                written_bound.extend(step.bindings.iter().map(symbol_name));
                let_bound.extend(step.bindings.iter().map(symbol_name));
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
    let mut bound = context;
    let mut provided = seed;
    let mut seen_atoms = BTreeSet::new();
    let mut complete = BTreeSet::new();
    let mut placed_equalities = BTreeSet::new();
    let mut placed_constraints = BTreeSet::new();
    let mut placed_negations = BTreeSet::new();
    let mut restricted = BTreeSet::new();
    let mut started = false;
    for step in steps {
        analysis.bound_before.push(bound.clone());
        analysis.provided_before.push(provided.clone());
        if let Step::Restrict(restriction, _) = step {
            analysis.order.push(order::StepFacts {
                preprocessing: true,
                ..Default::default()
            });
            let expected_path = match &parent {
                Some((path, branch)) => path.child(*branch, restriction.clause()),
                None => Occurrence::new(restriction.clause()),
            };
            restricted.extend(restrict::check(body, restriction, &expected_path, started)?);
            analysis.introduces.push(Vec::new());
            continue;
        }
        started = true;
        let mut order = order::StepFacts::default();
        match step {
            Step::Restrict(..) => unreachable!("restrictions handled above"),
            Step::Block(block, _) => {
                let clause = block.source.clause();
                let Some(cq::BodyItem::Block(written)) = body.iter().nth(clause) else {
                    return error(block, "block occurrence does not name a CQ block");
                };
                let expected_path = match &parent {
                    Some((path, branch)) => path.child(*branch, clause),
                    None => Occurrence::new(clause),
                };
                if block.source.clause != expected_path
                    || block.source.relation != "block"
                    || block.kind != written.kind
                    || block.branches.len() != written.branches.len()
                {
                    return error(
                        block,
                        "block must preserve its source path, operator and every branch",
                    );
                }
                let facts = &block_facts[&clause];
                if block.placement == blocks::Placement::Hoisted && facts.rust {
                    return error(block, "hoisted blocks cannot contain Rust steps");
                }
                let correlated = block.placement == blocks::Placement::Correlated;
                order.atom = true;
                order.provides.extend(block.source.variables().map(|(_, v)| symbol_name(v)));
                if correlated {
                    order.required.extend(facts.free.keys().cloned());
                }
                if let Some(required) = analysis.domain_requires.get(&clause) {
                    order.required.extend(required.iter().cloned());
                }
                order.required.extend(
                    order.provides.iter()
                        .filter(|name| facts.free.contains_key(*name) && let_bound.contains(*name))
                        .cloned(),
                );
                if correlated && !facts.free.keys().all(|name| bound.contains(name)) {
                    return error(
                        block,
                        "correlated block requires every free prefix variable",
                    );
                }
                if analysis
                    .domain_requires
                    .get(&clause)
                    .is_some_and(|names| names.iter().any(|name| !provided.contains(name)))
                {
                    return error(block, "hoisted block precedes its finite prefix domain");
                }
                let branch_context: Vec<_> = facts.free.keys().cloned().collect();
                // A correlated arm runs over the prefix and a domain-taking
                // hoisted arm over its finite domain; any other hoisted arm
                // starts from no seed.
                let branch_seed: BTreeSet<String> = if correlated || written.needs_domain(facts) {
                    facts.free.keys().cloned().collect()
                } else {
                    BTreeSet::new()
                };
                let mut branches = Vec::new();
                for (j, (branch, source_branch)) in
                    block.branches.iter().zip(&written.branches).enumerate()
                {
                    branches.push(analyze_steps(
                        &source_branch.body,
                        &branch.steps,
                        branch_context.clone(),
                        branch_seed.clone(),
                        Some((expected_path.clone(), j)),
                        schema,
                        &types,
                    )?);
                }
                if let Some(existing) = analysis.branches.get(&clause) {
                    if !same_branch_steps(block, steps, clause) || existing.len() != branches.len()
                    {
                        return error(
                            block,
                            "repeated block reads must retain the same branch plans",
                        );
                    }
                }
                analysis.branches.insert(clause, branches);
                let variables = columns(&block.source)?;
                let mut introduced = Vec::new();
                for (position, column) in block.source.columns.iter().enumerate() {
                    match column {
                        Column::Variable(v) if symbol_name(v) != variables[position] => {
                            return error(v, "block column differs from its common heading");
                        }
                        Column::Variable(v) if !bound.contains(&symbol_name(v)) => {
                            if facts.free.contains_key(&symbol_name(v))
                                && let_bound.contains(&symbol_name(v))
                            {
                                return error(
                                    block,
                                    "block free column must retain its earlier Rust binder",
                                );
                            }
                            introduced.push(symbol_name(v))
                        }
                        Column::Dropped(_) if bound.contains(&variables[position]) => {
                            return error(block, "a block read cannot drop a bound column");
                        }
                        _ => (),
                    }
                }
                if facts.rust && block.source.variables().count() != variables.len() {
                    return error(block, "a Rust block cannot split its row across reads");
                }
                if facts.rust && seen_atoms.contains(&clause) {
                    return error(
                        block,
                        "each block containing Rust expressions must be placed exactly once",
                    );
                }
                seen_atoms.insert(clause);
                if block.source.variables().count() == variables.len() {
                    complete.insert(clause);
                }
                bound.extend(introduced.iter().cloned());
                analysis.introduces.push(introduced);
            }
            Step::Atom(subatom, _) => {
                let clause = subatom.clause();
                order.atom = true;
                order.provides.extend(subatom.variables().map(|(_, v)| symbol_name(v)));
                order.required.extend(
                    order.provides.intersection(&let_bound).cloned(),
                );
                let expected_path = match &parent {
                    Some((path, branch)) => path.child(*branch, clause),
                    None => Occurrence::new(clause),
                };
                if subatom.clause != expected_path {
                    return error(subatom, "atom occurrence escaped its source branch path");
                }
                let variables = columns(subatom)?;
                if let Some(after) = &subatom.moved_after {
                    for occurrence in &after.occurrences { occurrence.check()?; }
                    if subatom.check.is_none() || after.occurrences.is_empty() {
                        return Err(syn::Error::new_spanned(subatom, "moved support requires a check and crossed prefix"));
                    }
                }
                // Formal rule VO-CheckPhase: a standalone check consumes
                // supplied columns, never an alternate introducing phase.
                if subatom.check.is_some()
                    && subatom.variables().any(|(_, v)| !provided.contains(&symbol_name(v)))
                {
                    return error(subatom, "a recorded check phase cannot introduce columns");
                }
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
                seen_atoms.insert(clause);
                if subatom.variables().count() == variables.len() {
                    complete.insert(clause);
                }
                bound.extend(introduced.iter().cloned());
                analysis.introduces.push(introduced);
            }
            Step::Equality(equality, _) => {
                order.required.extend(equality.variables().map(symbol_name));
                let clause = equalities
                    .iter()
                    .find(|(clause, expected)| {
                        !complete.contains(*clause) && ***expected == *equality
                    })
                    .map(|(&clause, _)| clause)
                    .or_else(|| {
                        constraints
                            .iter()
                            .find(|(clause, expected)| {
                                !complete.contains(*clause)
                                    && expected.equality().as_ref() == Some(equality)
                            })
                            .map(|(&clause, _)| clause)
                    });
                let Some(clause) = clause else {
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
                placed_constraints.insert(clause);
                complete.insert(clause);
                analysis.introduces.push(Vec::new());
            }
            Step::Constraint(constraint, _) => {
                order.required.extend(constraint.variables().map(symbol_name));
                let clause = constraints
                    .iter()
                    .find(|(clause, expected)| {
                        !complete.contains(*clause) && ***expected == *constraint
                    })
                    .map(|(&clause, _)| clause)
                    .or_else(|| {
                        let equality = constraint.equality()?;
                        equalities
                            .iter()
                            .find(|(clause, expected)| {
                                !complete.contains(*clause) && ***expected == equality
                            })
                            .map(|(&clause, _)| clause)
                    });
                let Some(clause) = clause else {
                    return error(
                        constraint,
                        "every relational constraint must appear exactly once from the query",
                    );
                };
                if let Some(variable) = constraint
                    .variables()
                    .find(|v| !bound.contains(&symbol_name(v)))
                {
                    return error(
                        constraint,
                        format!("constraint uses `{variable}` before it is bound"),
                    );
                }
                placed_constraints.insert(clause);
                placed_equalities.insert(clause);
                complete.insert(clause);
                analysis.introduces.push(Vec::new());
            }
            Step::Negation(negation, _) => {
                order.required.extend(
                    negation.atom.variables().map(symbol_name).filter(|v| logical.contains(v)),
                );
                let Some((&clause, _)) = negations.iter().find(|(clause, expected)| {
                    !placed_negations.contains(*clause) && ***expected == *negation
                }) else {
                    return error(
                        negation,
                        "every relational negation must appear exactly once from the query",
                    );
                };
                if let Some(variable) = negation
                    .atom
                    .variables()
                    .find(|v| logical.contains(&symbol_name(v)) && !bound.contains(&symbol_name(v)))
                {
                    return error(
                        negation,
                        format!("negation uses `{variable}` before it is bound"),
                    );
                }
                placed_negations.insert(clause);
                complete.insert(clause);
                analysis.introduces.push(Vec::new());
            }
            Step::Guard(_, _) | Step::Let(_, _) | Step::For(_, _) => {
                let identical = |clause: usize| match (step, &body[clause]) {
                    (Step::Guard(a, _), cq::BodyItem::Filter(b)) => a == b,
                    (Step::Let(a, _), cq::BodyItem::Let(b)) => a == b,
                    (Step::For(a, _), cq::BodyItem::For(b)) => a == b,
                    _ => false,
                };
                let Some(clause) = dependencies
                    .keys()
                    .copied()
                    .find(|clause| !complete.contains(clause) && identical(*clause))
                else {
                    return error(
                        step,
                        "Rust steps must preserve each source expression exactly once",
                    );
                };
                order.required.extend(dependencies[&clause].iter().cloned());
                if let Some(missing) = dependencies[&clause].iter().find(|v| !bound.contains(*v)) {
                    return error(
                        step,
                        format!("Rust step uses `{missing}` before it is bound"),
                    );
                }
                let introduced = body[clause]
                    .rust_step()
                    .expect("Rust step")
                    .bindings
                    .iter()
                    .map(symbol_name)
                    .collect::<Vec<_>>();
                order.provides.extend(introduced.iter().cloned());
                bound.extend(introduced.iter().cloned());
                analysis.introduces.push(introduced);
                complete.insert(clause);
            }
            Step::Reduce(reduce, _) => {
                order.preprocessing = true;
                let clause = reduce.clause();
                let variables = atom_clause(reduce, &reduce.relation, clause)?;
                let domain_ready = !analysis.domain_requires.is_empty()
                    && analysis
                        .domain_requires
                        .values()
                        .flatten()
                        .all(|name| provided.contains(name));
                if seen_atoms.contains(&clause) && !domain_ready {
                    return error(
                        reduce,
                        format!("clause #{clause} is reduced after its first subatom"),
                    );
                }
                let Reduction::With(_, subatom) = &reduce.reduction;
                let support = subatom.clause();
                for dependent in [clause, support] {
                    if let Some(required) = analysis.domain_requires.get(&dependent) {
                        order.required.extend(required.iter().cloned());
                    }
                    if steps.iter().any(|step| {
                        matches!(step, Step::Block(block, _)
                            if block.source.clause() == dependent
                                && block.placement == blocks::Placement::Correlated)
                    }) {
                        return error(
                            reduce,
                            "a correlated block cannot be a reduction target or support",
                        );
                    }
                    if analysis
                        .domain_requires
                        .get(&dependent)
                        .is_some_and(|names| names.iter().any(|name| !provided.contains(name)))
                    {
                        return error(reduce, "reduction precedes its block context domain");
                    }
                }
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
        analysis.order.push(order);
        // Every column the step reads is now provided by this scope.
        match step {
            Step::Atom(subatom, _) => {
                provided.extend(subatom.variables().map(|(_, v)| symbol_name(v)))
            }
            Step::Block(block, _) => {
                provided.extend(block.source.variables().map(|(_, v)| symbol_name(v)))
            }
            _ => (),
        }
        provided.extend(analysis.introduces.last().into_iter().flatten().cloned());
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
        .find(|(clause, _)| !placed_equalities.contains(clause) && !restricted.contains(clause))
    {
        return error(*missing, "every relational equality must be placed");
    }
    if let Some((_, missing)) = constraints
        .iter()
        .find(|(clause, _)| !placed_constraints.contains(clause) && !restricted.contains(clause))
    {
        return error(*missing, "every relational constraint must be placed");
    }
    if let Some((_, missing)) = negations
        .iter()
        .find(|(clause, _)| !placed_negations.contains(clause))
    {
        return error(*missing, "every relational negation must be placed");
    }
    if let Some(&missing) = dependencies
        .keys()
        .find(|clause| !complete.contains(clause))
    {
        return error(
            &body[missing],
            "every Rust guard and binding must be placed",
        );
    }
    Ok(analysis)
}

fn same_branch_steps(block: &blocks::Block, steps: &[Step], clause: usize) -> bool {
    steps.iter().all(|step| match step {
        Step::Block(other, _) if other.source.clause() == clause => {
            block.kind == other.kind
                && block.placement == other.placement
                && block.branches == other.branches
        }
        _ => true,
    })
}

/// A query and the complete schedule of its valuations.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Program {
    pub cq: cq::Program,
    pub valuation: Valuation,
}

pub use cq_syntax as syntax;

pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    contract::check(source)?;
    contract::check(target)?;
    fn erase(steps: &[Step]) -> Vec<Step> {
        steps
            .iter()
            .filter_map(|step| match step {
                Step::Reduce(..) => None,
                Step::Block(block, semi) => {
                    let mut block = block.clone();
                    for arm in &mut block.branches {
                        arm.steps = erase(&arm.steps);
                    }
                    Some(Step::Block(block, *semi))
                }
                _ => Some(step.clone()),
            })
            .collect()
    }
    fn reductions(steps: &[Step]) -> Vec<&Step> {
        steps
            .iter()
            .flat_map(|step| match step {
                Step::Reduce(..) => vec![step],
                Step::Block(block, _) => block
                    .branches
                    .iter()
                    .flat_map(|arm| reductions(&arm.steps))
                    .collect(),
                _ => vec![],
            })
            .collect()
    }
    if source.cq != target.cq
        || source.valuation.ordering != target.valuation.ordering
        || erase(&source.valuation.steps) != erase(&target.valuation.steps)
        || reductions(&source.valuation.steps)
            .iter()
            .any(|s| !reductions(&target.valuation.steps).contains(s))
    {
        return Err(syn::Error::new_spanned(
            &target.valuation,
            "VO rewrite must preserve CQ, valuation erasure and existing reductions",
        ));
    }
    Ok(())
}
impl cq_stage::Language for Program {
    fn calibration_snapshot(&self) -> Option<cq_stage::calibration::Snapshot> {
        use quote::ToTokens;
        use cq_stage::calibration::{Node, Snapshot};
        fn collect(steps: &[Step], prefix: &str, nodes: &mut Vec<Node>) {
            for (i, step) in steps.iter().enumerate() {
                let name = format!("{prefix}{i}");
                nodes.push(Node { name: name.clone(),
                    operator: step.to_token_stream().to_string(), inputs: Vec::new() });
                if let Step::Block(block, _) = step {
                    for (branch, arm) in block.branches.iter().enumerate() {
                        collect(&arm.steps, &format!("{name}/{branch}/"), nodes);
                    }
                }
            }
        }
        let mut nodes = Vec::new();
        collect(&self.valuation.steps, "", &mut nodes);
        Some(Snapshot { language: "vo".into(),
            program: self.to_token_stream().to_string(), nodes })
    }
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite(source, target)
    }
}
