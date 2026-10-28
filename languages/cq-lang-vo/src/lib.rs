//! Valuation plans for `T X = bottom + Set X`.
//! Relational subatoms, negations, equalities and semijoin reductions commute. They may
//! move earlier across opaque Rust lets/guards; the Rust chain retains written
//! order and all its written relational predecessors. This chain/Before check
//! is a sufficient refinement condition: a target may panic only on an input
//! where the written reference panics. I/O, mutation and evaluation counts are
//! outside this denotation. A split atom is a predecessor only once its full
//! correlated row has been checked. Rust guards are never reductions.
//!
//! ```text
//! Step ::= Subatom ";" | "!" Atom ";" | Constraint ";"
//!        | "if" Expr ";" | "let" Pat [":" Type] "=" Expr ";"
//!        | "reduce" Ident "@" Int "with" Subatom ";"
//! ```
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

//! VO records block @path(columns) hoisted/correlated { steps } | { steps }; or else arms, retaining recursive branch steps and reduction occurrence paths R@c.j.k.
//! Its contract checks coverage, interface scope and Before sets recursively.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Rust keeps a branch ancestry tree: union siblings unordered, fallback ordered;
//! each arm retains prefix Before facts and fallback anti steps. Rust `||` keeps
//! its existing recognition. Only differing literals on one interface column
//! prove union arms disjoint; no data keys or constraints are assumed.
//!
pub use cq_lang_cq as cq;

use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, Token, token};

pub use cq_syntax::{CommaList, Occurrence, symbol_name};
pub mod blocks;
pub use blocks::Placement;
pub mod join_branches;
pub use join_branches::{JoinBranch, JoinBranches};

pub mod kw {
    syn::custom_keyword!(reduce);
    syn::custom_keyword!(valuation);
    syn::custom_keyword!(with);
    syn::custom_keyword!(block);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Valuation {
    pub valuation_token: kw::valuation,
    #[parse(join_branches::parse_optional)]
    pub join_branches: Option<JoinBranches>,
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
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub enum Step {
    Block(blocks::Block, Token![;]),
    Equality(cq::Equality, Token![;]),
    Constraint(cq::Constraint, Token![;]),
    Negation(cq::Negation, Token![;]),
    Guard(cq_syntax::Filter, Token![;]),
    Let(Box<cq_syntax::Let>, Token![;]),
    For(Box<cq_syntax::For>, Token![;]),
    Reduce(Reduce, Token![;]),
    Atom(Subatom, Token![;]),
}

impl syn::parse::Parse for Step {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        Ok(if input.peek(kw::block) {
            Self::Block(input.parse()?, input.parse()?)
        } else if input.peek(Token![!]) {
            Self::Negation(input.parse()?, input.parse()?)
        } else if input.peek(Token![if]) {
            Self::Guard(input.parse()?, input.parse()?)
        } else if input.peek(Token![let]) {
            Self::Let(input.parse()?, input.parse()?)
        } else if input.peek(Token![for]) {
            Self::For(input.parse()?, input.parse()?)
        } else if input.peek(kw::reduce) {
            Self::Reduce(input.parse()?, input.parse()?)
        } else if input.peek(Ident) && input.peek2(Token![@]) {
            Self::Atom(input.parse()?, input.parse()?)
        } else {
            let c: cq::Constraint = input.parse()?;
            match c.equality() {
                Some(e) => Self::Equality(e, input.parse()?),
                None => Self::Constraint(c, input.parse()?),
            }
        })
    }
}

/// A restriction of one clause's relation, normally before its first subatom.
/// A finite block domain that requires prefix Rust delays reductions until
/// that context exists; a late reduction also filters the current prefix.
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
    /// Rust-produced free columns required before a finite block domain exists.
    pub domain_requires: BTreeMap<usize, BTreeSet<String>>,
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
    join_branches::check(program)?;
    let schema = program
        .cq
        .inputs()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    analyze_steps(
        &program.cq.query.body,
        &program.valuation.steps,
        Vec::new(),
        None,
        &schema,
        &BTreeMap::new(),
    )
}

/// Formal rule WF-VO-Arithmetic: Typed CQ-Arith clauses leave the partial Rust chain but remain Before facts for later partial steps.
/// Formal rule WF-BlockPlacement: Hoisted arms are Rust-free; correlated arms require their free prefix, and repeated reads preserve placement.
fn analyze_steps(
    body: &cq_syntax::CommaList<cq::BodyItem>,
    steps: &[Step],
    context: Vec<String>,
    parent: Option<(Occurrence, usize)>,
    schema: &BTreeMap<String, &cq::RelationDecl>,
    inherited_types: &BTreeMap<String, syn::Type>,
) -> syn::Result<Analysis> {
    let types = cq::blocks::types(schema, body, inherited_types)?;
    let arithmetic: BTreeSet<_> = body
        .iter()
        .enumerate()
        .filter(|(_, item)| item.arithmetic(&types).is_some())
        .map(|(clause, _)| clause)
        .collect();
    for step in steps {
        match step {
            Step::Atom(a, _) => a.clause.check()?,
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
    let mut rust_chain = Vec::new();
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
                    && steps.iter().any(|step| {
                        matches!(step, Step::Block(planned, _)
                            if planned.source.clause() == clause
                                && planned.placement == blocks::Placement::Hoisted)
                    })
                {
                    let missing: BTreeSet<_> = block
                        .branches
                        .iter()
                        .flat_map(|arm| {
                            let generated: BTreeSet<_> = cq::blocks::walk(&arm.body)
                                .into_iter()
                                .filter_map(cq::BodyItem::atom)
                                .flat_map(cq_syntax::Atom::variables)
                                .map(symbol_name)
                                .collect();
                            let negative_context = cq::blocks::walk(&arm.body).into_iter().any(|item| {
                                matches!(item, cq::BodyItem::Negation(n)
                                    if n.atom.variables().any(|v| facts.free.contains_key(&symbol_name(v))))
                            });
                            facts
                                .free
                                .keys()
                                .filter(|name| negative_context || !generated.contains(*name))
                                .cloned()
                                .collect::<Vec<_>>()
                        })
                        .collect();
                    let required: BTreeSet<_> = missing.intersection(&let_bound).cloned().collect();
                    if !required.is_empty() {
                        analysis.domain_requires.insert(clause, required);
                    }
                }
                if facts.rust {
                    let_bound.extend(facts.interface.keys().cloned());
                    rust_chain.push(clause);
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
                if !arithmetic.contains(&clause) {
                    rust_chain.push(clause);
                }
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
    let mut seen_atoms = BTreeSet::new();
    let mut complete = BTreeSet::new();
    let mut placed_equalities = BTreeSet::new();
    let mut placed_constraints = BTreeSet::new();
    let mut placed_negations = BTreeSet::new();
    let mut next_rust = 0;
    for step in steps {
        analysis.bound_before.push(bound.clone());
        match step {
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
                if facts.rust {
                    if rust_chain.get(next_rust) != Some(&clause) {
                        return error(
                            block,
                            "Rust blocks must retain their written position in the parent chain tree",
                        );
                    }
                    next_rust += 1;
                }
                if facts.rust
                    && (0..clause).any(|earlier| {
                        !complete.contains(&earlier)
                            && (arithmetic.contains(&earlier)
                                || matches!(
                                    &body[earlier],
                                    cq::BodyItem::Positive { .. }
                                        | cq::BodyItem::Equality(_)
                                        | cq::BodyItem::Constraint(_)
                                        | cq::BodyItem::Negation(_)
                                        | cq::BodyItem::Block(_)
                                ))
                    })
                {
                    return error(
                        block,
                        "Before condition: a Rust branch must follow every written prefix clause",
                    );
                }
                if correlated && !facts.free.keys().all(|name| bound.contains(name)) {
                    return error(
                        block,
                        "correlated block requires every free prefix variable",
                    );
                }
                let branch_context: Vec<_> = if correlated {
                    bound.clone()
                } else {
                    facts.free.keys().cloned().collect()
                };
                let mut branches = Vec::new();
                for (j, (branch, source_branch)) in
                    block.branches.iter().zip(&written.branches).enumerate()
                {
                    branches.push(analyze_steps(
                        &source_branch.body,
                        &branch.steps,
                        branch_context.clone(),
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
                seen_atoms.insert(clause);
                if block.source.variables().count() == variables.len() {
                    complete.insert(clause);
                }
                bound.extend(introduced.iter().cloned());
                analysis.introduces.push(introduced);
            }
            Step::Atom(subatom, _) => {
                let clause = subatom.clause();
                let expected_path = match &parent {
                    Some((path, branch)) => path.child(*branch, clause),
                    None => Occurrence::new(clause),
                };
                if subatom.clause != expected_path {
                    return error(subatom, "atom occurrence escaped its source branch path");
                }
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
            Step::Constraint(constraint, _) => {
                let Some((&clause, _)) = constraints.iter().find(|(clause, expected)| {
                    !placed_constraints.contains(*clause) && ***expected == *constraint
                }) else {
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
                complete.insert(clause);
                analysis.introduces.push(Vec::new());
            }
            Step::Negation(negation, _) => {
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
                let total = arithmetic
                    .iter()
                    .copied()
                    .find(|clause| !complete.contains(clause) && identical(*clause));
                let Some(clause) = total.or_else(|| rust_chain.get(next_rust).copied()) else {
                    return error(step, "more Rust steps than the query has");
                };
                let identical = identical(clause);
                if !identical {
                    return error(
                        step,
                        "Rust lets, generators and guards must keep their written relative order (chain)",
                    );
                }
                if let Some(missing) = dependencies[&clause].iter().find(|v| !bound.contains(*v)) {
                    return error(
                        step,
                        format!("Rust step uses `{missing}` before it is bound"),
                    );
                }
                for earlier in (0..clause).filter(|_| total.is_none()) {
                    if (arithmetic.contains(&earlier)
                        || matches!(
                            &body[earlier],
                            cq::BodyItem::Positive { .. }
                                | cq::BodyItem::Equality(_)
                                | cq::BodyItem::Constraint(_)
                                | cq::BodyItem::Negation(_)
                                | cq::BodyItem::Block(_)
                        ))
                        && !complete.contains(&earlier)
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
                if total.is_none() {
                    next_rust += 1;
                } else {
                    complete.insert(clause);
                }
            }
            Step::Reduce(reduce, _) => {
                let clause = reduce.clause();
                let variables = atom_clause(reduce, &reduce.relation, clause)?;
                let domain_ready = !analysis.domain_requires.is_empty()
                    && analysis
                        .domain_requires
                        .values()
                        .flatten()
                        .all(|name| bound.contains(name));
                if seen_atoms.contains(&clause) && !domain_ready {
                    return error(
                        reduce,
                        format!("clause #{clause} is reduced after its first subatom"),
                    );
                }
                let Reduction::With(_, subatom) = &reduce.reduction;
                let support = subatom.clause();
                for dependent in [clause, support] {
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
                        .is_some_and(|names| names.iter().any(|name| !bound.contains(name)))
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
    if let Some((_, missing)) = constraints
        .iter()
        .find(|(clause, _)| !placed_constraints.contains(clause))
    {
        return error(*missing, "every relational constraint must be placed");
    }
    if let Some((_, missing)) = negations
        .iter()
        .find(|(clause, _)| !placed_negations.contains(clause))
    {
        return error(*missing, "every relational negation must be placed");
    }
    if let Some(&missing) = arithmetic.iter().find(|clause| !complete.contains(clause)) {
        return error(&body[missing], "every arithmetic clause must be placed");
    }
    if next_rust != rust_chain.len() {
        return error(
            &body[rust_chain[next_rust]],
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
        || source.valuation.join_branches != target.valuation.join_branches
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
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }
    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite(source, target)
    }
}
