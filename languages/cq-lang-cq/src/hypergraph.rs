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
//! The facts, and the theorem that ties them together:
//!
//! * **Acyclicity** is α-acyclicity, decided by the GYO reduction (Graham;
//!   Yu–Özsoyoğlu): repeatedly drop a variable that occurs in one remaining
//!   edge only, or remove an edge contained in another remaining edge,
//!   recording the container as its parent. The hypergraph is acyclic iff
//!   one empty edge remains; the parent pointers then form a **join tree**
//!   over clause indices whose root is the last remaining edge.
//! * The **primal graph** joins two variables that share an atom. The
//!   **elimination order** is the maximum-cardinality search numbering
//!   (Tarjan–Yannakakis 1984) read backwards, so that eliminating variables
//!   in this order is a perfect elimination order iff the primal graph is
//!   **chordal**; the analysis checks that property rather than trusting it.
//! * **Conformality**: every clique of the primal graph lies inside some
//!   hyperedge. It is tested by Gilmore's criterion (Berge, *Hypergraphs*,
//!   ch. 1): for any three edges, some edge contains the union of their
//!   pairwise intersections.
//! * Beeri, Fagin, Maier and Yannakakis (*On the desirability of acyclic
//!   database schemes*, JACM 1983) show the hypergraph is acyclic iff it is
//!   conformal and its primal graph is chordal. The three facts are computed
//!   independently, so `acyclic == chordal && conformal` is a check on the
//!   analysis itself. The triangle is the classic witness that chordality
//!   alone is not enough: its primal graph is complete, hence chordal, but
//!   the clique `{x, y, z}` is no atom.
//! * **Width** is the elimination width of the elimination order plus one:
//!   the largest number of later neighbours a variable has when it is
//!   eliminated, counting the fill edges elimination adds among those
//!   neighbours. On a chordal graph the order is perfect, no fill occurs,
//!   and the width is the clique number, i.e. treewidth plus one; otherwise
//!   it is the width of one triangulation, an upper bound on treewidth plus
//!   one.

use std::collections::{BTreeMap, BTreeSet};

use super::{BodyItem, Program, normal};
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
    /// Every clique of the primal graph lies inside some atom.
    pub conformal: bool,
    /// Reversed maximum-cardinality search numbering of the primal graph.
    pub elimination_order: Vec<String>,
    /// `elimination_order` is a perfect elimination order.
    pub chordal: bool,
    /// Elimination width of `elimination_order` plus one, with fill.
    pub width: usize,
}

/// Check normal form, then analyze.
pub fn check(module: &Program) -> syn::Result<Analysis> {
    normal::check(module)?;
    Ok(Analysis::of(module))
}

impl Analysis {
    /// The facts of a normal-form CQ program; see [`check`] to establish normal
    /// form first. Non-variable atom arguments are not vertices.
    /// Formal rule H-Occurrence: Construct atom facts indexed by the complete normal-form body.
    /// Formal rule H-Incidence: Insert every atom clause into each variable incidence set.
    pub fn of(module: &Program) -> Analysis {
        let mut analysis = Analysis::default();
        let names: BTreeSet<_> = module
            .query
            .body
            .iter()
            .filter_map(BodyItem::atom)
            .flat_map(super::Atom::variables)
            .map(symbol_name)
            .collect();
        analysis.canonical_names = names
            .iter()
            .map(|name| (name.clone(), name.clone()))
            .collect();
        for item in &module.query.body {
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
        for (clause, item) in module.query.body.iter().enumerate() {
            let BodyItem::Positive { atom } = item else {
                continue;
            };
            let columns: Vec<String> = atom.variables().map(symbol_name).collect();
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
        analysis.reduce(edges.clone());
        analysis.conformal = conformal(&edges);
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
        self.join_tree = self.acyclic.then(|| parent.into_iter().collect());
    }

    /// Maximum-cardinality search; fills `elimination_order`, `chordal` and
    /// `width`.
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
        let position: BTreeMap<&String, usize> =
            numbered.iter().enumerate().map(|(i, v)| (v, i)).collect();
        let mut filled = self.primal.clone();
        self.chordal = true;
        self.width = 0;
        for (i, variable) in numbered.iter().enumerate() {
            let later: Vec<String> = filled[variable]
                .iter()
                .filter(|u| position[*u] > i)
                .cloned()
                .collect();
            for (a, u) in later.iter().enumerate() {
                for w in &later[a + 1..] {
                    if !filled[u].contains(w) {
                        self.chordal = false;
                        filled.get_mut(u).unwrap().insert(w.clone());
                        filled.get_mut(w).unwrap().insert(u.clone());
                    }
                }
            }
            self.width = self.width.max(later.len() + 1);
        }
        self.elimination_order = numbered;
    }
}

/// Gilmore's criterion: for every three edges some edge contains the union
/// of their pairwise intersections.
fn conformal(edges: &BTreeMap<usize, BTreeSet<String>>) -> bool {
    let edges: Vec<&BTreeSet<String>> = edges.values().collect();
    let covered = |union: BTreeSet<&String>| {
        edges
            .iter()
            .any(|edge| union.iter().all(|v| edge.contains(*v)))
    };
    for (i, a) in edges.iter().enumerate() {
        for (j, b) in edges.iter().enumerate().skip(i) {
            for c in &edges[j..] {
                let union = a
                    .intersection(b)
                    .chain(b.intersection(c))
                    .chain(a.intersection(c))
                    .collect();
                if !covered(union) {
                    return false;
                }
            }
        }
    }
    true
}
