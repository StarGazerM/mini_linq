//! Deterministic order selection and independent correspondence checks.
//!
//! The source meaning is the desugared written order in `bottom + Set`.
//! Relational steps may move earlier across opaque Rust, which keeps its
//! payload, relative order, free-variable binders and written Before sets.
//! Column equality assumes Leibniz identity. No execution counts are preserved.
//! All heuristics plan blocks recursively: Written retains position, Prefix ranks a block by connectivity and heading width, Elimination/GYO use its hyperedge, and Fine plans reuse derived prefix indices.
//! Hypergraph semijoins retain negative originals.
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
#![forbid(unsafe_code)]

pub use cq_lang_cq as cq;
pub use cq_lang_ra as ra;
pub use cq_lang_vo as vo;

use cq_order::{Heuristic, Options, Ranking, Split};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, Token};

/// The ten supported planning passes. Ranking selects written IR at compile
/// time; it introduces no configuration carried into execution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Planner {
    #[default]
    WrittenCoarse,
    WrittenFine,
    Hypergraph,
    /// Robust predicate transfer (Zhao et al., SIGMOD'25) on the written
    /// left-deep order: forward then backward semijoin transfer along the
    /// reduction tree rooted at the outer stream, before the join phase.
    Transfer,
    PrefixConnectedCoarse,
    PrefixConnectedFine,
    PrefixGreedyCoarse,
    PrefixGreedyFine,
    EliminationCoarse,
    EliminationFine,
}

impl Planner {
    pub const ALL: [Self; 10] = [
        Self::WrittenCoarse,
        Self::WrittenFine,
        Self::Hypergraph,
        Self::Transfer,
        Self::PrefixConnectedCoarse,
        Self::PrefixConnectedFine,
        Self::PrefixGreedyCoarse,
        Self::PrefixGreedyFine,
        Self::EliminationCoarse,
        Self::EliminationFine,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::WrittenCoarse => "written_coarse",
            Self::WrittenFine => "written_fine",
            Self::Hypergraph => "hypergraph",
            Self::Transfer => "transfer",
            Self::PrefixConnectedCoarse => "prefix_connected_coarse",
            Self::PrefixConnectedFine => "prefix_connected_fine",
            Self::PrefixGreedyCoarse => "prefix_greedy_coarse",
            Self::PrefixGreedyFine => "prefix_greedy_fine",
            Self::EliminationCoarse => "elimination_coarse",
            Self::EliminationFine => "elimination_fine",
        }
    }

    fn ordering(self) -> Options {
        // Formal rule Plan-Generic: Written/Fine records variable-at-a-time valuation steps.
        let (heuristic, split) = match self {
            Self::WrittenCoarse | Self::Hypergraph | Self::Transfer => {
                (Heuristic::Written, Split::Coarse)
            }
            Self::WrittenFine => (Heuristic::Written, Split::Fine),
            Self::PrefixConnectedCoarse => (Heuristic::Prefix(Ranking::Connected), Split::Coarse),
            Self::PrefixConnectedFine => (Heuristic::Prefix(Ranking::Connected), Split::Fine),
            Self::PrefixGreedyCoarse => (Heuristic::Prefix(Ranking::GreedyWidth), Split::Coarse),
            Self::PrefixGreedyFine => (Heuristic::Prefix(Ranking::GreedyWidth), Split::Fine),
            Self::EliminationCoarse => (Heuristic::Elimination, Split::Coarse),
            Self::EliminationFine => (Heuristic::Elimination, Split::Fine),
        };
        Options { heuristic, split }
    }

    fn reductions(self, source: &cq::Program) -> cq_reduce::Options {
        // Formal rule Plan-Graph: Hypergraph uses tree reductions only in acyclic scopes and exact block cuts in any scope.
        cq_reduce::Options {
            semijoins: if self == Self::Hypergraph
                && source
                    .query
                    .body
                    .iter()
                    .any(|item| matches!(item, cq::BodyItem::Block(_)))
            {
                cq_reduce::Reductions::Hypergraph
            } else if self == Self::Hypergraph && cq::hypergraph::Analysis::of(source).acyclic {
                cq_reduce::Reductions::Tree
            } else if self == Self::Transfer {
                // The forward pass into the stream and the backward pass are
                // added to the stream scope by `transfer`.
                cq_reduce::Reductions::Masks
            } else {
                cq_reduce::Reductions::None
            },
        }
    }
}

impl Parse for Planner {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Ok(Self::default());
        }
        let kind: Ident = input.parse()?;
        if kind == "hypergraph" {
            return Ok(Self::Hypergraph);
        }
        if kind == "transfer" {
            return Ok(Self::Transfer);
        }
        let ranking = if kind == "prefix" {
            let ranking: Ident = input.parse()?;
            match ranking.to_string().as_str() {
                "connected" => Some(false),
                "greedy" => Some(true),
                _ => {
                    return Err(syn::Error::new_spanned(
                        ranking,
                        "expected connected or greedy",
                    ));
                }
            }
        } else if kind == "written" || kind == "elimination" {
            None
        } else {
            return Err(syn::Error::new_spanned(
                kind,
                "expected written, hypergraph, transfer, prefix, or elimination",
            ));
        };
        input.parse::<Token![,]>()?;
        let split: Ident = input.parse()?;
        let fine = match split.to_string().as_str() {
            "coarse" => false,
            "fine" => true,
            _ => return Err(syn::Error::new_spanned(split, "expected coarse or fine")),
        };
        Ok(match (kind.to_string().as_str(), ranking, fine) {
            ("written", _, false) => Self::WrittenCoarse,
            ("written", _, true) => Self::WrittenFine,
            ("elimination", _, false) => Self::EliminationCoarse,
            ("elimination", _, true) => Self::EliminationFine,
            ("prefix", Some(false), false) => Self::PrefixConnectedCoarse,
            ("prefix", Some(false), true) => Self::PrefixConnectedFine,
            ("prefix", Some(true), false) => Self::PrefixGreedyCoarse,
            ("prefix", Some(true), true) => Self::PrefixGreedyFine,
            _ => unreachable!("validated planner grammar"),
        })
    }
}

impl ToTokens for Planner {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match self {
            Self::WrittenCoarse => quote!(written, coarse),
            Self::WrittenFine => quote!(written, fine),
            Self::Hypergraph => quote!(hypergraph),
            Self::Transfer => quote!(transfer),
            Self::PrefixConnectedCoarse => quote!(prefix connected, coarse),
            Self::PrefixConnectedFine => quote!(prefix connected, fine),
            Self::PrefixGreedyCoarse => quote!(prefix greedy, coarse),
            Self::PrefixGreedyFine => quote!(prefix greedy, fine),
            Self::EliminationCoarse => quote!(elimination, coarse),
            Self::EliminationFine => quote!(elimination, fine),
        });
    }
}

/// Normalize CQ and record its chosen schedule, split and tree reductions.
pub fn compile_valuation(source: &cq::Program, planner: Planner) -> syn::Result<vo::Program> {
    cq::contract::check(source)?;
    let normal = cq::normal::normalize(source)?;
    let ordering = planner.ordering();
    let ordered = cq_order::compile(&normal, &ordering)?;
    cq_order::check(&normal, &ordered, &ordering)?;
    let reductions = planner.reductions(&normal);
    let reduced = cq_reduce::compile(&ordered, &reductions)?;
    cq_reduce::check(&ordered, &reduced, &reductions)?;
    if planner == Planner::Transfer {
        transfer(&reduced)
    } else {
        Ok(reduced)
    }
}

/// Robust predicate transfer on a Masks-reduced written plan. In the stream
/// scope, along the reduction tree rooted at the stream (the largest relation
/// in fact-first written order, RPT's LargestRoot): after the Masks forward
/// pass into every other clause, the stream is reduced by each child whose
/// subtree carries a filter or block cut, then the backward pass reduces every
/// child atom, parents first, by its parent when a filter lies outside the
/// child's subtree. As in RPT, no transfer starts from a side without filters.
/// Inner block scopes keep their Masks reductions.
/// Formal rule Plan-Transfer: Masks forward pass, then the stream's forward edges, then the backward pass, all before the stream.
fn transfer(reduced: &vo::Program) -> syn::Result<vo::Program> {
    let analysis = vo::contract::check(reduced)?;
    let graph = cq::hypergraph::Analysis::of(&reduced.cq);
    let body = &reduced.cq.query.body;
    let steps = &reduced.valuation.steps;
    // Scopes with opaque Rust keep Masks only. There, the structural RA
    // correspondence check rejects some sound transfer schedules (a backward
    // reduction by a support already reduced by its target, with split
    // projected reads; fuzz seeds 91 and 97: "projection correlations differ").
    let opaque = steps
        .iter()
        .any(|step| matches!(step, vo::Step::Guard(..) | vo::Step::Let(..)));
    let Some(root) = cq_reduce::outer(steps, &graph).filter(|_| !opaque) else {
        return Ok(reduced.clone());
    };
    let parents = cq_reduce::rooted(&graph.reduction_tree, root);
    let mut children: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for (&child, &parent) in &parents {
        children.entry(parent).or_default().push(child);
    }
    let mut filtered: std::collections::BTreeSet<usize> =
        cq::hypergraph::filter_ranks(body, &graph)
            .into_iter()
            .filter(|&(_, rank)| rank < 3)
            .map(|(clause, _)| clause)
            .collect();
    filtered.extend(
        cq_reduce::block_cuts(&graph, body)
            .into_iter()
            .map(|(atom, _)| atom),
    );
    let subtree = |node: usize| {
        let mut nodes = vec![node];
        let mut pending = vec![node];
        while let Some(next) = pending.pop() {
            for &child in children.get(&next).into_iter().flatten() {
                nodes.push(child);
                pending.push(child);
            }
        }
        nodes
    };
    let is_atom = |clause: usize| body[clause].atom().is_some();
    // RPT transfers along join predicates only: an edge sharing no variable
    // (a product) carries no key. Blocks may depend on context bound by
    // earlier clauses; atom-to-block cuts are already Masks reductions, so
    // transfers use atom supports only.
    let edge = |target: usize, support: usize| {
        is_atom(target) && is_atom(support) && !graph.shared(target, support).is_empty()
    };
    let mut extra = Vec::new();
    for &child in children.get(&root).into_iter().flatten() {
        if edge(root, child) && subtree(child).iter().any(|c| filtered.contains(c)) {
            extra.push(cq_reduce::semijoin(&analysis, &graph, root, child));
        }
    }
    let mut pending = vec![root];
    while let Some(parent) = pending.pop() {
        for &child in children.get(&parent).into_iter().flatten() {
            let inside = subtree(child);
            if edge(child, parent) && filtered.iter().any(|c| !inside.contains(c)) {
                extra.push(cq_reduce::semijoin(&analysis, &graph, child, parent));
            }
        }
        pending.extend(children.get(&parent).into_iter().flatten().rev());
    }
    let mut target = reduced.clone();
    let first = target
        .valuation
        .steps
        .iter()
        .position(|step| matches!(step, vo::Step::Atom(..) | vo::Step::Block(..)))
        .unwrap_or(target.valuation.steps.len());
    target.valuation.steps.splice(first..first, extra);
    vo::contract::check(&target)?;
    Ok(target)
}

/// CQ→RA′ through the public order, reduce and elaborate APIs. Relational
/// pushdown is performed by elaboration; opaque Rust always stays in its chain.
/// Hypergraph plans share the facade's checked redundant-support absorption.
pub fn compile(source: &cq::Program, planner: Planner) -> syn::Result<ra::Program> {
    let valuation = compile_valuation(source, planner)?;
    let target = cq_elaborate::compile(&valuation)?;
    cq_elaborate::check(&valuation, &target)?;
    let target = if planner == Planner::Hypergraph {
        cq_elaborate::simplify_reductions(&target)?
    } else {
        target
    };
    check(source, &target)?;
    Ok(target)
}

/// Independently check an authored valuation, including split and selected
/// reducer obligations in every block branch. Rankings are not reselected.
pub fn check_valuation(
    source: &cq::Program,
    target: &vo::Program,
    planner: Planner,
) -> syn::Result<()> {
    cq::contract::check(source)?;
    let normal = cq::normal::normalize(source)?;
    let mut erased = target.clone();
    fn erase_reductions(steps: &mut Vec<vo::Step>) {
        steps.retain_mut(|step| match step {
            vo::Step::Reduce(..) => false,
            vo::Step::Block(block, _) => {
                for branch in &mut block.branches {
                    erase_reductions(&mut branch.steps);
                }
                true
            }
            _ => true,
        });
    }
    erase_reductions(&mut erased.valuation.steps);
    cq_order::check(&normal, &erased, &planner.ordering())?;
    if planner == Planner::Transfer {
        // Formal rule CK-Transfer: The transfer schedule equals the one recomputed from the erased plan.
        let reductions = planner.reductions(&normal);
        let expected = transfer(&cq_reduce::compile(&erased, &reductions)?)?;
        if expected != *target {
            return Err(syn::Error::new_spanned(
                &target.valuation,
                "reductions differ from the predicate-transfer schedule",
            ));
        }
        return Ok(());
    }
    cq_reduce::check(&erased, target, &planner.reductions(&normal))
}

/// Check CQ→RA′ refinement against the exact written normal-form schedule,
/// independently of the planner and its chosen ranking. This uses the core's
/// occurrence/Rust-chain contract; it never compiles or reselects a target plan.
pub fn check(source: &cq::Program, target: &ra::Program) -> syn::Result<()> {
    cq::contract::check(source)?;
    let normal = cq::normal::normalize(source)?;
    cq_elaborate::check(&written_valuation(&normal)?, target)
}

/// Construct the source meaning directly, with one full generator per written
/// clause and no optimization. This is deliberately independent of cq-order.
fn written_valuation(normal: &cq::Program) -> syn::Result<vo::Program> {
    cq::normal::check(normal)?;
    let steps = written_steps(&normal.query.body, &std::collections::BTreeSet::new(), None);
    let reference = vo::Program {
        cq: normal.clone(),
        valuation: vo::Valuation {
            valuation_token: Default::default(),
            join_branches: None,
            brace: Default::default(),
            steps,
        },
    };
    vo::contract::check(&reference)?;
    Ok(reference)
}

fn written_steps(
    body: &cq::CommaList<cq::BodyItem>,
    context: &std::collections::BTreeSet<String>,
    parent: Option<(vo::Occurrence, usize)>,
) -> Vec<vo::Step> {
    let mut bound = context.clone();
    let mut steps = Vec::new();
    for (clause, item) in body.iter().enumerate() {
        let path = parent.as_ref().map_or_else(
            || vo::Occurrence::new(clause),
            |(path, j)| path.child(*j, clause),
        );
        steps.push(match item {
            cq::BodyItem::Positive { atom } => {
                bound.extend(atom.variables().map(cq::symbol_name));
                vo::Step::Atom(
                    vo::Subatom {
                        relation: atom.relation.clone(),
                        at: Default::default(),
                        clause: path,
                        paren: Default::default(),
                        columns: atom
                            .variables()
                            .cloned()
                            .map(vo::Column::Variable)
                            .collect(),
                    },
                    Default::default(),
                )
            }
            cq::BodyItem::Block(block) => {
                let facts = block.analyze(&bound);
                let context = if facts.rust {
                    bound.clone()
                } else {
                    facts.free.keys().cloned().collect()
                };
                let branches = block
                    .branches
                    .iter()
                    .enumerate()
                    .map(|(j, branch)| vo::blocks::Branch {
                        brace: branch.brace,
                        steps: written_steps(&branch.body, &context, Some((path.clone(), j))),
                    })
                    .collect();
                bound.extend(facts.interface.keys().cloned());
                vo::Step::Block(
                    vo::blocks::Block {
                        source: vo::Subatom {
                            relation: syn::Ident::new("block", proc_macro2::Span::call_site()),
                            at: Default::default(),
                            clause: path,
                            paren: Default::default(),
                            columns: facts
                                .free
                                .values()
                                .chain(facts.interface.values())
                                .cloned()
                                .map(vo::Column::Variable)
                                .collect(),
                        },
                        kind: block.kind,
                        branches,
                    },
                    Default::default(),
                )
            }
            cq::BodyItem::Equality(e) => vo::Step::Equality(e.clone(), Default::default()),
            cq::BodyItem::Constraint(c) => vo::Step::Constraint(c.clone(), Default::default()),
            cq::BodyItem::Filter(g) => vo::Step::Guard(g.clone(), Default::default()),
            cq::BodyItem::Let(b) => {
                bound.extend(cq::rust::bindings(&b.pattern).iter().map(cq::symbol_name));
                vo::Step::Let(b.clone(), Default::default())
            }
            _ => unreachable!("checked CQ normal form"),
        });
    }
    steps
}
