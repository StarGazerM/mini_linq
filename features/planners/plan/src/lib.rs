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
use quote::{quote, ToTokens};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, Token};

/// The nine supported planning passes. Ranking selects written IR at compile
/// time; it introduces no configuration carried into execution.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Planner {
    #[default]
    WrittenCoarse,
    WrittenFine,
    Hypergraph,
    PrefixConnectedCoarse,
    PrefixConnectedFine,
    PrefixGreedyCoarse,
    PrefixGreedyFine,
    EliminationCoarse,
    EliminationFine,
}

impl Planner {
    pub const ALL: [Self; 9] = [
        Self::WrittenCoarse,
        Self::WrittenFine,
        Self::Hypergraph,
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
            Self::WrittenCoarse | Self::Hypergraph => (Heuristic::Written, Split::Coarse),
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
            guards: cq_reduce::Pushdown::None,
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
                "expected written, hypergraph, prefix, or elimination",
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
    Ok(reduced)
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
