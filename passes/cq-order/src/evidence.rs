//! Default ordering from recorded evidence (P3.3): a key-functional root and
//! Lookup/Expand phases.
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
//! and CK-Le like authored step advice. Scopes without a count for every
//! top-level occurrence, with top-level blocks, or without a dominating
//! functional root keep the ordinary default order. The evidence is recorded in the VO in
//! every case, so the checker validates the recorded choice against the
//! recorded facts without rerunning this producer.
use std::collections::{BTreeMap, BTreeSet};

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
    fn of(source: &'a cq::Program, evidence: &valuation::Evidence) -> Option<Self> {
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
                counts.insert(clause, evidence.count(&name)?);
                keys.insert(
                    clause,
                    declarations
                        .get(&name)
                        .map(|declaration| evidence.keys(declaration))
                        .unwrap_or_default(),
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

/// Front: check WF-NF and WF-Evidence (user-authored input), then [`order`]
/// (total).
pub fn compile(
    source: &cq::Program,
    evidence: &valuation::Evidence,
    options: &Options,
) -> syn::Result<valuation::Program> {
    let normal = Normal::check(source)?;
    evidence.check(source)?;
    Ok(order(normal, evidence, options))
}

/// Formal rule O-EvidenceLe: With a recorded count for every top-level occurrence, stream the largest-count key-functional root that dominates the written root (no smaller count, no stronger filter, no neighbour besides the written root filtered as strongly), expand keyed reads by filter rank and look up other ready filtered keyed reads first; otherwise keep the default order. Record the evidence.
pub fn order(
    source: Normal<'_>,
    evidence: &valuation::Evidence,
    options: &Options,
) -> valuation::Program {
    let default = super::order(source, options);
    let scope = Scope::of(source.program(), evidence);
    let reads = scope.as_ref().and_then(|scope| {
        let (&root, _) = scope
            .dominating()
            .iter()
            .min_by_key(|&(&clause, &rows)| (std::cmp::Reverse(rows), clause))?;
        scope.plan(root)
    });
    let mut program = match reads {
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
        None => default,
    };
    program.valuation.evidence = Some(evidence.clone());
    program
}

/// Formal rule CK-EvidenceLe: Given recorded evidence, a scope with a functional root dominating its written root must stream such a root of maximal recorded count, read every other occurrence first through a recorded key, look up only filtered occurrences, and pass WF-VO with CK-Le phases; other scopes pass the default ordering check.
pub fn check_evidence(
    source: &cq::Program,
    target: &valuation::Program,
    options: &Options,
) -> syn::Result<()> {
    let analysis = valuation::contract::check(target)?;
    let Some(evidence) = &target.valuation.evidence else {
        return Err(syn::Error::new_spanned(
            &target.cq.name,
            "evidence ordering must record its evidence",
        ));
    };
    let roots = Scope::of(source, evidence).map(|scope| (scope.dominating(), scope));
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
