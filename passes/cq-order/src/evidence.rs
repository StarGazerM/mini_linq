//! Default ordering from a checked dataset document: a key-functional root
//! and Lookup/Expand phases. CQ carries the sole explicit fact correspondence.
//!
//! An occurrence read is *keyed* when one recorded key of its relation is
//! bound (literal or bound variable) at that read: it matches at most one
//! row, so it filters and never fans out. A root is *functional* when every
//! other top-level occurrence can follow it as a keyed read. Streaming a root
//! builds every other occurrence into a provider, so the planner replaces the
//! written root only by a functional root that *dominates* it: no smaller
//! recorded count and no stronger filter than the written root (hypergraph
//! filter rank), and no neighbour other than the written root filtered at
//! least as strongly as the written root: reduction semi-joins a root by its
//! filtered neighbours, so such a neighbour filters the root by a predicate
//! the written root lacks. Under any selectivity monotone in filter rank the
//! dominating root then has the larger filtered input and the written root
//! becomes the build side. Without dominance, a range-filtered Lineitem
//! root against an unfiltered Orders written root measured 1.36-1.39x
//! slower on GPU (TPC-H d72_inner); an unfiltered Lineitem root reduced by a
//! date-filtered Orders neighbour against a filtered PartSupp written root
//! measured 1.36x slower (d85). Among dominating roots the largest
//! count streams (RPT's LargestRoot, restricted to key-functional trees:
//! Zhao et al., "Debunking the Myth of Join Ordering", VLDB 2024, §4). After the root,
//! each step expands the ready keyed occurrence with the strongest filter
//! (hypergraph filter rank, then written position); every other ready keyed
//! occurrence that carries a filter is looked up first (a non-introducing
//! read, L&E's `R@k(b̄)`), so filters reject rows before expansion work.
//! A lookup immediately followed by its own expansion is redundant and is
//! not emitted.
//!
//! Evidence chooses, it does not justify: the produced VO is checked by WF-VO
//! and CK-Le like authored step advice. Scopes without an equality count for every
//! top-level occurrence, with top-level blocks, or without a dominating
//! functional root keep the ordinary default order. Facts remain in the source's
//! checked `DeclaredEvidence`; VO neither duplicates them nor accepts inline facts.
use std::collections::{BTreeMap, BTreeSet};
use cq_evidence::{Claim, Source};
use cq_syntax::declared_evidence::EvidenceView;

use super::{
    Advice, BodyItem, Clause, Normal, Options, check, clauses, cq, le, order_advised, ready, rust,
    symbol_name, valuation,
};

struct Scope<'a> {
    clauses: Vec<Clause<'a>>,
    ranks: BTreeMap<usize, u8>,
    keys: BTreeMap<usize, Vec<Vec<usize>>>,
    counts: BTreeMap<usize, u64>,
}

/// One planned read: the clause, and the bound columns of a lookup.
type Read = (usize, Option<Vec<syn::Ident>>);

impl<'a> Scope<'a> {
    /// Evidence applies to a block-free scope with at least two top-level
    /// occurrences, each with a recorded count.
    fn of(source: &'a cq::Program) -> Option<Self> {
        let evidence = source.declared_evidence.as_ref()?;
        let body = &source.query.body;
        if body.iter().any(|item| matches!(item, BodyItem::Block(_))) {
            return None;
        }
        let declarations: BTreeMap<_, _> = source
            .inputs()
            .map(|declaration| (symbol_name(&declaration.name), declaration))
            .collect();
        let mut keys = BTreeMap::new();
        let mut counts = BTreeMap::new();
        for (clause, item) in body.iter().enumerate() {
            if let BodyItem::Positive { atom } = item {
                let name = symbol_name(&atom.relation);
                let binding = evidence.bindings.iter()
                    .find(|binding| symbol_name(&binding.relation) == name)?;
                if binding.view != EvidenceView::Full {
                    return None;
                }
                let relation = evidence.document.relations.get(&binding.source)?;
                let rows = relation.rows.as_ref()?;
                if rows.claim != Claim::Equal
                    || !matches!(rows.source, Source::Exact | Source::Declared | Source::Calibrated)
                {
                    return None;
                }
                counts.insert(clause, rows.value?);
                let declaration = declarations.get(&name)?;
                keys.insert(
                    clause,
                    relation.keys.iter().filter_map(|key| {
                        key.columns.iter().map(|column| {
                            let binding = binding.columns.iter()
                                .find(|binding| binding.source == *column)?;
                            declaration.columns.iter()
                                .position(|declared| declared.name == binding.column)
                        }).collect::<Option<Vec<_>>>()
                    }).collect(),
                );
            }
        }
        if counts.len() < 2 {
            return None;
        }
        let graph = cq::hypergraph::Analysis::of(source);
        Some(Self {
            clauses: clauses(body, &BTreeSet::new()),
            ranks: cq::hypergraph::filter_ranks(
                body,
                &graph,
                cq::hypergraph::local_predicates(body)
                    .iter()
                    .flat_map(cq::hypergraph::LocalPredicate::restrictions),
            ),
            keys,
            counts,
        })
    }

    fn atom(&self, clause: usize) -> &cq::Atom {
        self.clauses[clause].atom.as_ref().expect("positive atom")
    }

    fn rank(&self, clause: usize) -> u8 {
        self.ranks.get(&clause).copied().unwrap_or(3)
    }

    /// Rust bindings and generators provide their names once ready.
    fn close(&self, bound: &mut BTreeSet<String>) {
        loop {
            let before = bound.len();
            for (clause, facts) in self.clauses.iter().enumerate() {
                if facts.atom.is_none()
                    && !facts.provides.is_empty()
                    && ready(&self.clauses, bound, clause)
                {
                    bound.extend(facts.provides.iter().cloned());
                }
            }
            if bound.len() == before {
                return;
            }
        }
    }

    /// Whether a read of this occurrence under `bound` binds a recorded key.
    fn keyed(&self, clause: usize, bound: &BTreeSet<String>) -> bool {
        let atom = self.atom(clause);
        self.keys[&clause].iter().any(|key| {
            key.iter().all(|&position| {
                rust::variable(&atom.args[position])
                    .is_none_or(|variable| bound.contains(&symbol_name(variable)))
            })
        })
    }

    /// The keyed read sequence from `root`, or `None` if `root` is not functional.
    fn plan(&self, root: usize) -> Option<Vec<Read>> {
        let mut bound = BTreeSet::new();
        self.close(&mut bound);
        if !ready(&self.clauses, &bound, root) {
            return None;
        }
        bound.extend(self.clauses[root].provides.iter().cloned());
        self.close(&mut bound);
        let mut pending: BTreeSet<usize> =
            self.counts.keys().copied().filter(|&c| c != root).collect();
        let mut looked = BTreeSet::new();
        let mut reads = vec![(root, None)];
        while !pending.is_empty() {
            let mut keyed: Vec<usize> = pending
                .iter()
                .copied()
                .filter(|&c| ready(&self.clauses, &bound, c) && self.keyed(c, &bound))
                .collect();
            keyed.sort_by_key(|&c| (self.rank(c), c));
            let &next = keyed.first()?;
            for &clause in &keyed[1..] {
                if self.rank(clause) < 3 && looked.insert(clause) {
                    let columns: Vec<_> = self
                        .atom(clause)
                        .variables()
                        .filter(|variable| bound.contains(&symbol_name(variable)))
                        .cloned()
                        .collect();
                    reads.push((clause, Some(columns)));
                }
            }
            reads.push((next, None));
            bound.extend(self.clauses[next].provides.iter().cloned());
            self.close(&mut bound);
            pending.remove(&next);
        }
        Some(reads)
    }

    /// The written root: the first top-level occurrence ready with nothing bound.
    fn written_root(&self) -> Option<usize> {
        let mut bound = BTreeSet::new();
        self.close(&mut bound);
        self.counts
            .keys()
            .copied()
            .find(|&clause| ready(&self.clauses, &bound, clause))
    }

    /// Functional roots other than the written root that dominate it: no
    /// smaller recorded count, no stronger filter, and no neighbour (an
    /// occurrence sharing a variable) other than the written root filtered at
    /// least as strongly as the written root. Reduction semi-joins a root by
    /// its filtered neighbours, so such a neighbour filters the root by a
    /// predicate the written root lacks. Streaming a dominating root instead never
    /// trades a larger filtered input for a smaller one under any
    /// selectivities monotone in filter rank; each comes with its count.
    fn dominating(&self) -> BTreeMap<usize, u64> {
        let Some(written) = self.written_root() else {
            return BTreeMap::new();
        };
        let (rows, rank) = (self.counts[&written], self.rank(written));
        self.counts
            .iter()
            .filter(|&(&clause, &count)| {
                clause != written
                    && count >= rows
                    && self.rank(clause) >= rank
                    && !self.filtered_neighbour(clause, written, rank)
                    && self.plan(clause).is_some()
            })
            .map(|(&clause, &rows)| (clause, rows))
            .collect()
    }

    /// Whether an occurrence other than `written` that shares a variable with
    /// `clause` carries a filter of rank at most `rank`.
    fn filtered_neighbour(&self, clause: usize, written: usize, rank: u8) -> bool {
        let variables: BTreeSet<String> = self.atom(clause).variables().map(symbol_name).collect();
        self.counts.keys().any(|&other| {
            other != clause
                && other != written
                && self.rank(other) <= rank
                && self
                    .atom(other)
                    .variables()
                    .any(|variable| variables.contains(&symbol_name(variable)))
        })
    }
}

/// Front: check WF-NF and the source's dataset correspondence, then [`order`] (total).
pub fn compile(
    source: &cq::Program,
    options: &Options,
) -> syn::Result<valuation::Program> {
    let normal = Normal::check(source)?;
    Ok(order(normal, options))
}

/// Formal rule O-EvidenceLe: Read equality row counts and surviving key declarations through checked file-document bindings; stream the largest-count dominating functional root with unchanged filter/written ties, otherwise keep the default.
pub fn order(
    source: Normal<'_>,
    options: &Options,
) -> valuation::Program {
    let default = super::order(source, options);
    let scope = Scope::of(source.program());
    let reads = scope.as_ref().and_then(|scope| {
        let (&root, _) = scope
            .dominating()
            .iter()
            .min_by_key(|&(&clause, &rows)| (std::cmp::Reverse(rows), clause))?;
        scope.plan(root)
    });
    match reads {
        Some(reads) => {
            let written: Vec<_> = default
                .valuation
                .steps
                .iter()
                .filter_map(|step| match step {
                    valuation::Step::Atom(atom, _) => Some(atom.clause()),
                    _ => None,
                })
                .collect();
            let lookups = reads.iter().any(|(_, columns)| columns.is_some());
            if !lookups && reads.iter().map(|(clause, _)| *clause).eq(written) {
                default
            } else {
                // Planned reads name top-level positive atoms and subsets of
                // their variables: resolved advice by construction.
                let advice = Advice {
                    reads: reads
                        .into_iter()
                        .map(|(clause, columns)| {
                            (clause, columns.map(|columns| columns.iter().map(symbol_name).collect()))
                        })
                        .collect(),
                };
                order_advised(source, &advice)
            }
        }
        // Formal rule O-EvidenceFallback: Keep the default order when checked document counts/keys are unavailable or keyed planning adds no change; never synthesize statistics.
        None => default,
    }
}

/// Formal rule CK-EvidenceLe: Check the recorded root against the source's checked document counts/keys, keyed first reads, filtered lookups, WF-VO and CK-Le; unavailable statistics select CK-Order.
pub fn check_evidence(
    source: &cq::Program,
    target: &valuation::Program,
    options: &Options,
) -> syn::Result<()> {
    let analysis = valuation::contract::check(source, target)?;
    let roots = Scope::of(source).map(|scope| (scope.dominating(), scope));
    let Some((roots, scope)) = roots.filter(|(roots, _)| !roots.is_empty()) else {
        return check(source, target, options);
    };
    let mut first = BTreeSet::new();
    let mut lookups = BTreeSet::new();
    let mut root = None;
    for (index, step) in target.valuation.steps.iter().enumerate() {
        let valuation::Step::Atom(atom, _) = step else {
            continue;
        };
        let clause = atom.clause();
        if !first.insert(clause) {
            continue;
        }
        if root.is_none() {
            root = Some(clause);
            let largest = roots.values().max().copied();
            if roots.get(&clause).copied() != largest {
                return Err(syn::Error::new_spanned(
                    atom,
                    "evidence ordering must stream a dominating key-functional root of largest recorded count",
                ));
            }
            continue;
        }
        let bound = analysis.bound_before[index].iter().cloned().collect();
        if !scope.keyed(clause, &bound) {
            return Err(syn::Error::new_spanned(
                atom,
                "evidence ordering reads an occurrence before one of its recorded keys is bound",
            ));
        }
        if atom.variables().count() < atom.columns.len() && analysis.introduces[index].is_empty() {
            if scope.rank(clause) >= 3 {
                return Err(syn::Error::new_spanned(
                    atom,
                    "evidence ordering looks up an occurrence that carries no filter",
                ));
            }
            lookups.insert(clause);
        }
    }
    if lookups.is_empty() {
        check(source, target, options)
    } else {
        le::check_advice(source, target, &analysis, &lookups)
    }
}
