//! Explicit independent exact-join branches, separate from union/fallback blocks.
use crate::{CommaList, Program, Step, symbol_name};
use std::collections::{BTreeMap, BTreeSet};
use syn::{Ident, LitInt, Token, token};

pub mod kw {
    syn::custom_keyword!(branches);
    syn::custom_keyword!(branch);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct JoinBranches {
    pub branches_token: kw::branches,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    #[parse(parse_branches)]
    #[to_tokens(|tokens, branches| quote::TokenStreamExt::append_all(tokens, branches))]
    pub branches: Vec<JoinBranch>,
}

fn parse_branches(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<JoinBranch>> {
    let mut branches = Vec::new();
    while !input.is_empty() {
        branches.push(input.parse()?);
    }
    Ok(branches)
}

pub(crate) fn parse_optional(
    input: syn::parse::ParseStream<'_>,
) -> syn::Result<Option<JoinBranches>> {
    if input.peek(kw::branches) {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct JoinBranch {
    pub branch_token: kw::branch,
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    #[parse(CommaList::parse_terminated)]
    pub clauses: CommaList<LitInt>,
    #[syn(parenthesized)]
    pub paren: token::Paren,
    #[syn(in = paren)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<Ident>,
    pub semi: Token![;],
}

impl JoinBranch {
    pub fn clauses(&self) -> BTreeSet<usize> {
        self.clauses
            .iter()
            .map(|c| c.base10_parse().expect("checked branch clause"))
            .collect()
    }
}

/// Exact interface: variables used by another atom, the head, or a predicate
/// spanning the branch boundary. Branch-local relational predicates execute inside.
pub fn interface(program: &crate::cq::Program, members: &BTreeSet<usize>) -> BTreeSet<String> {
    interface_with(program, members, &atoms(program))
}

fn atoms(program: &crate::cq::Program) -> BTreeMap<usize, BTreeSet<String>> {
    program
        .query
        .body
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            if let crate::cq::BodyItem::Positive { atom } = item {
                Some((i, atom.variables().map(symbol_name).collect()))
            } else {
                None
            }
        })
        .collect()
}

fn interface_with(
    program: &crate::cq::Program,
    members: &BTreeSet<usize>,
    atoms: &BTreeMap<usize, BTreeSet<String>>,
) -> BTreeSet<String> {
    let own: BTreeSet<_> = members
        .iter()
        .flat_map(|i| atoms[i].iter().cloned())
        .collect();
    let mut needed: BTreeSet<_> = program
        .query
        .valuation_variables()
        .map(symbol_name)
        .collect();
    for (i, variables) in atoms {
        if !members.contains(i) {
            needed.extend(variables.iter().cloned());
        }
    }
    for item in &program.query.body {
        let variables: BTreeSet<_> = match item {
            crate::cq::BodyItem::Constraint(c) => c.variables().map(symbol_name).collect(),
            crate::cq::BodyItem::Equality(e) => e.variables().map(symbol_name).collect(),
            _ => continue,
        };
        if !variables.is_subset(&own) {
            needed.extend(variables);
        }
    }
    own.intersection(&needed).cloned().collect()
}

/// Formal rule WF-JoinBranch: Require disjoint connected full relational atom groups, exact interfaces, and branch-local reductions before reads; opaque Rust remains ungrouped.
pub(crate) fn check(program: &Program) -> syn::Result<()> {
    let Some(groups) = &program.valuation.join_branches else {
        return Ok(());
    };
    let fail = |message| syn::Error::new_spanned(groups, message);
    if program.cq.query.body.iter().any(|item| {
        !matches!(
            item,
            crate::cq::BodyItem::Positive { .. }
                | crate::cq::BodyItem::Equality(_)
                | crate::cq::BodyItem::Constraint(_)
        )
    }) {
        return Err(fail(
            "independent join branches require a flat relational scope",
        ));
    }
    let atoms = atoms(&program.cq);
    let graph = crate::cq::hypergraph::Analysis::of(&program.cq);
    let mut seen = BTreeSet::new();
    for group in &groups.branches {
        let members: BTreeSet<usize> = group
            .clauses
            .iter()
            .map(|c| c.base10_parse())
            .collect::<syn::Result<_>>()?;
        if members.len() < 2
            || members.len() != group.clauses.len()
            || members
                .iter()
                .any(|i| !atoms.contains_key(i) || !seen.insert(*i))
        {
            return Err(fail(
                "join branches require at least two distinct, disjoint atom occurrences",
            ));
        }
        let mut reached = BTreeSet::from([*members.first().unwrap()]);
        loop {
            let previous = reached.len();
            let heading: BTreeSet<_> = reached
                .iter()
                .flat_map(|i| graph.clauses[i].iter().cloned())
                .collect();
            reached.extend(
                members
                    .iter()
                    .filter(|i| graph.clauses[i].iter().any(|v| heading.contains(v)))
                    .copied(),
            );
            if reached.len() == previous {
                break;
            }
        }
        if reached != members {
            return Err(fail("join branch atoms must be connected"));
        }
        let columns: BTreeSet<_> = group.columns.iter().map(symbol_name).collect();
        if columns.len() != group.columns.len()
            || columns != interface_with(&program.cq, &members, &atoms)
        {
            return Err(fail(
                "join branch must project exactly its external interface and head columns",
            ));
        }
        let mut reads = BTreeSet::new();
        let mut started = false;
        for step in &program.valuation.steps {
            match step {
                Step::Atom(a, _) if members.contains(&a.clause()) => {
                    started = true;
                    if a.variables().count() != a.columns.len() || !reads.insert(a.clause()) {
                        return Err(fail("join branches require one full read per member"));
                    }
                }
                Step::Reduce(r, _) if members.contains(&r.clause()) => {
                    let crate::Reduction::With(_, support) = &r.reduction;
                    if started || !members.contains(&support.clause()) {
                        return Err(fail(
                            "branch member reductions must precede its first grouped read and use only branch members",
                        ));
                    }
                }
                _ => (),
            }
        }
        if reads != members {
            return Err(fail("join branch is missing a full member read"));
        }
    }
    Ok(())
}
