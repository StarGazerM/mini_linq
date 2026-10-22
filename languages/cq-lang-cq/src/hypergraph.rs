//! Hypergraph facts of a normal-form query.
//!
//! A normal query's atoms form a hypergraph: the vertices are the variables
//! that occur in atoms and every atom is a hyperedge over its variables. The
//! [`Analysis`] records what that hypergraph *is*; it decides nothing. The
//! order pass reads it to rank clauses and variables; reduction computes
//! its Yannakakis schedule from the join tree.
//!
//! Relational variable equalities identify vertices; fresh names introduced
//! by normalization therefore preserve the original join graph. The analysis
//! expands each equality class in its incident edges while retaining actual
//! variable spellings for scheduling. Rust guards and bindings add no edges;
//! their opaque partial-function chain has denotation `bottom + Set X` and is
//! checked by the valuation contract. Other effects are outside that meaning.
//!
use std::collections::{BTreeMap, BTreeSet};

use super::{BodyItem, Program};
use crate::symbol_name;

/// One GYO ear-removal step, kept as the certificate of [`Analysis::acyclic`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GyoStep {
    /// `variable` occurs in no other remaining edge; drop it from `clause`.
    Private { clause: usize, variable: String },
    /// The remaining part of `child` lies inside `parent`; remove `child`.
    Absorb { child: usize, parent: usize },
}

/// The hypergraph of a normal-form query.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Analysis {
    /// Equality classes; aliases retain their actual spelling in CQ/VO.
    pub canonical_names: BTreeMap<String, String>,
    /// Atom variables in order of first occurrence over the body.
    pub variables: Vec<String>,
    /// Atom clause index → its variables, in column order.
    pub clauses: BTreeMap<usize, Vec<String>>,
    /// Variable → the atom clauses it occurs in.
    pub incidence: BTreeMap<String, BTreeSet<usize>>,
    /// Variable → the variables it shares an atom with.
    pub primal: BTreeMap<String, BTreeSet<String>>,
    /// The GYO reduction, in the order it was applied.
    pub gyo_steps: Vec<GyoStep>,
    /// The GYO reduction ended with one empty edge.
    pub acyclic: bool,
    /// `(clause, parent clause)` for every atom clause, ascending by clause;
    /// the one clause without a parent is the root. Present iff acyclic.
    pub join_tree: Option<Vec<(usize, Option<usize>)>>,
    /// `(clause, parent clause)` for every atom clause: the join tree when
    /// acyclic; otherwise the GYO ears plus a spanning tree of the cyclic core,
    /// each core edge joining clauses that share a variable (most shared first,
    /// then earliest clauses). Semijoins along it are sound reductions; only on
    /// acyclic queries are they a full reducer.
    pub reduction_tree: Vec<(usize, Option<usize>)>,
    /// Reversed maximum-cardinality search numbering of the primal graph.
    pub elimination_order: Vec<String>,
}

/// Static occurrence priorities, never estimates of data cardinality.
/// Smaller ranks mean point, range, derived/total/block, then unfiltered.
/// Opaque Rust filters are deliberately excluded.
/// Formal rule H-FilterRank: Rank local total predicates through equality classes, without data statistics.
pub fn filter_ranks(body: &super::CommaList<BodyItem>, graph: &Analysis) -> BTreeMap<usize, u8> {
    use cq_syntax::constraint::{Comparison, Constraint, Term};
    let mut ranks: BTreeMap<_, _> = graph
        .clauses
        .keys()
        .map(|&c| {
            (
                c,
                match &body[c] {
                    BodyItem::Positive { atom }
                        if atom.args.iter().any(cq_syntax::equality::literal) =>
                    {
                        0
                    }
                    BodyItem::Block(_) => 2,
                    _ => 3,
                },
            )
        })
        .collect();
    let mut apply = |variables: Vec<String>, rank: u8| {
        if variables.is_empty() {
            return;
        }
        for (&clause, columns) in &graph.clauses {
            if variables
                .iter()
                .all(|v| columns.iter().any(|c| graph.equivalent(c, v)))
            {
                ranks.entry(clause).and_modify(|r| *r = (*r).min(rank));
            }
        }
    };
    for item in body {
        match item {
            BodyItem::Equality(e) if !matches!(e.right, super::EqualityValue::Variable(_)) => {
                apply(vec![symbol_name(&e.left)], 0);
            }
            BodyItem::Constraint(c) => {
                let rank = match c {
                    Constraint::Compare {
                        left,
                        comparison: Comparison::Eq(_),
                        right,
                    } if left.constant() || right.constant() => 0,
                    Constraint::Compare {
                        left,
                        comparison,
                        right,
                    } if comparison.ordered() && (left.constant() || right.constant()) => 1,
                    Constraint::Between {
                        term: Term::Variable(_),
                        low,
                        high,
                    } if low.constant() && high.constant() => 1,
                    _ => 2,
                };
                apply(c.variables().map(symbol_name).collect(), rank);
                for domain in c.implied_domains() {
                    apply(domain.variables().map(symbol_name).collect(), 2);
                }
            }
            _ => (),
        }
    }
    ranks
}

impl Analysis {
    /// The facts of a normal-form CQ program; see [`check`] to establish normal
    /// form first. Non-variable atom arguments are not vertices.
    /// Formal rule H-Occurrence: Construct atom facts indexed by the complete normal-form body.
    /// Formal rule H-Incidence: Insert every atom clause into each variable incidence set.
    pub fn of(module: &Program) -> Analysis {
        Self::of_body(&module.query.body, &BTreeSet::new())
    }
    pub fn of_body(body: &super::CommaList<BodyItem>, context: &BTreeSet<String>) -> Analysis {
        let mut analysis = Analysis::default();
        let mut prefix = context.clone();
        let mut clauses = BTreeMap::new();
        for (clause, item) in body.iter().enumerate() {
            match item {
                BodyItem::Positive { atom } => {
                    let columns: Vec<_> = atom.variables().map(symbol_name).collect();
                    prefix.extend(columns.iter().cloned());
                    clauses.insert(clause, columns);
                }
                BodyItem::Block(block) => {
                    let facts = block.analyze(&prefix);
                    if !facts.rust {
                        clauses.insert(
                            clause,
                            facts
                                .free
                                .keys()
                                .chain(facts.interface.keys())
                                .cloned()
                                .collect(),
                        );
                    }
                    prefix.extend(facts.interface.into_keys());
                }
                BodyItem::Let(binding) => prefix.extend(
                    super::rust::bindings(&binding.pattern)
                        .iter()
                        .map(symbol_name),
                ),
                BodyItem::For(binding) => prefix.extend(
                    super::rust::bindings(&binding.pattern)
                        .iter()
                        .map(symbol_name),
                ),
                _ => (),
            }
        }
        let names: BTreeSet<_> = clauses.values().flatten().cloned().collect();
        analysis.canonical_names = names
            .iter()
            .map(|name| (name.clone(), name.clone()))
            .collect();
        for item in body {
            if let BodyItem::Equality(super::Equality {
                left,
                right: super::EqualityValue::Variable(right),
                ..
            }) = item
            {
                let left = symbol_name(left);
                let right = symbol_name(right);
                let a = analysis.canonical_names.get(&left).unwrap_or(&left).clone();
                let b = analysis
                    .canonical_names
                    .get(&right)
                    .unwrap_or(&right)
                    .clone();
                let representative = a.clone().min(b.clone());
                for value in analysis.canonical_names.values_mut() {
                    if *value == a || *value == b {
                        *value = representative.clone();
                    }
                }
                analysis
                    .canonical_names
                    .insert(left, representative.clone());
                analysis.canonical_names.insert(right, representative);
            }
        }
        let mut edges = BTreeMap::new();
        for (clause, columns) in clauses {
            // Expanding an equality class in each incident edge preserves the
            // relational join graph while retaining actual alias names for VO.
            let edge: BTreeSet<_> = names
                .iter()
                .filter(|name| {
                    columns
                        .iter()
                        .any(|column| analysis.equivalent(column, name))
                })
                .cloned()
                .collect();
            for variable in columns.iter().chain(&edge) {
                if !analysis.variables.contains(variable) {
                    analysis.variables.push(variable.clone());
                }
            }
            for variable in &edge {
                analysis
                    .incidence
                    .entry(variable.clone())
                    .or_default()
                    .insert(clause);
                analysis
                    .primal
                    .entry(variable.clone())
                    .or_default()
                    .extend(edge.iter().filter(|c| *c != variable).cloned());
            }
            analysis.clauses.insert(clause, columns);
            edges.insert(clause, edge);
        }
        analysis.reduce(edges);
        analysis.eliminate();
        analysis
    }

    /// Whether relational variable equalities place two names in one class.
    pub fn equivalent(&self, left: &str, right: &str) -> bool {
        self.canonical_names
            .get(left)
            .map(String::as_str)
            .unwrap_or(left)
            == self
                .canonical_names
                .get(right)
                .map(String::as_str)
                .unwrap_or(right)
    }

    /// Variables common to two atom clauses, in `a`'s column order.
    pub fn shared(&self, a: usize, b: usize) -> Vec<String> {
        let (Some(left), Some(right)) = (self.clauses.get(&a), self.clauses.get(&b)) else {
            return Vec::new();
        };
        left.iter()
            .filter(|v| right.iter().any(|r| self.equivalent(v, r)))
            .cloned()
            .collect()
    }

    /// GYO reduction on the residual edges; fills `gyo_steps`, `acyclic`
    /// and `join_tree`.
    /// Formal rule G-Private: Delete the least residual variable occurring in one edge.
    /// Formal rule G-Absorb: Delete the least contained residual edge and record its parent.
    /// Formal rule G-Accept: Accept a terminal residual with at most one edge, all empty.
    /// Formal rule G-Reject: Reject every other terminal residual.
    /// Formal rule H-JoinTree: Return the constructed GYO parent map exactly when acyclic.
    fn reduce(&mut self, mut residual: BTreeMap<usize, BTreeSet<String>>) {
        let edges = residual.clone();
        let mut parent: BTreeMap<usize, Option<usize>> =
            residual.keys().map(|&clause| (clause, None)).collect();
        loop {
            let private = residual.iter().find_map(|(&clause, edge)| {
                edge.iter()
                    .find(|v| {
                        residual
                            .iter()
                            .all(|(&j, k)| j == clause || !k.contains(*v))
                    })
                    .map(|v| (clause, v.clone()))
            });
            if let Some((clause, variable)) = private {
                residual.get_mut(&clause).unwrap().remove(&variable);
                self.gyo_steps.push(GyoStep::Private { clause, variable });
                continue;
            }
            let absorb = residual.iter().find_map(|(&child, edge)| {
                residual
                    .iter()
                    .find_map(|(&j, k)| (j != child && edge.is_subset(k)).then_some((child, j)))
            });
            let Some((child, parent_clause)) = absorb else {
                break;
            };
            residual.remove(&child);
            parent.insert(child, Some(parent_clause));
            self.gyo_steps.push(GyoStep::Absorb {
                child,
                parent: parent_clause,
            });
        }
        self.acyclic = residual.len() <= 1 && residual.values().all(BTreeSet::is_empty);
        // Formal rule H-SpanningCore: Connect the residual cyclic core by a
        // spanning tree over shared variables, rooted at its earliest clause.
        let mut core: Vec<usize> = residual.keys().copied().collect();
        if let Some(root) = core.first().copied() {
            core.remove(0);
            let mut tree = vec![root];
            while !core.is_empty() {
                let (position, parent_clause) = core
                    .iter()
                    .enumerate()
                    .flat_map(|(i, &c)| tree.iter().map(move |&t| (i, c, t)))
                    .max_by_key(|&(_, c, t)| {
                        (
                            edges[&c].intersection(&edges[&t]).count(),
                            std::cmp::Reverse((c, t)),
                        )
                    })
                    .map(|(i, _, t)| (i, t))
                    .expect("a pending core clause");
                let clause = core.remove(position);
                parent.insert(clause, Some(parent_clause));
                tree.push(clause);
            }
        }
        self.reduction_tree = parent.iter().map(|(&c, &p)| (c, p)).collect();
        self.join_tree = self.acyclic.then(|| parent.into_iter().collect());
    }

    /// Maximum-cardinality search; fills `elimination_order`.
    fn eliminate(&mut self) {
        let mut numbered: Vec<String> = Vec::new();
        let mut unnumbered: Vec<&String> = self.variables.iter().collect();
        while !unnumbered.is_empty() {
            let (position, _) = unnumbered
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let count = numbered
                        .iter()
                        .filter(|n| self.primal[*v].contains(*n))
                        .count();
                    (i, count)
                })
                .max_by(|(i, a), (j, b)| a.cmp(b).then(j.cmp(i)))
                .expect("a variable remains");
            numbered.push(unnumbered.remove(position).clone());
        }
        numbered.reverse();
        self.elimination_order = numbered;
    }
}
