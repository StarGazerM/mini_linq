//! G-PRIVATE / G-ABSORB reduce occurrence-labeled hypergraphs.
//! Analysis never deletes executable atoms. See doc/formal/planning.tex.
use super::{Heading, JoinPolicy, PlanningOptions, error};
use cq_ir::{cq, symbol_name};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GyoRule {
    Private { occurrence: usize, variable: String },
    Absorb { child: usize, parent: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinTree {
    /// Original occurrence IDs; the sole `None` is the root.
    pub parent: Vec<Option<usize>>,
}

impl JoinTree {
    /// Independently check connectivity and running intersection on original edges.
    pub fn check(&self, edges: &[Heading]) -> Result<(), String> {
        let n = edges.len();
        if n == 0
            || self.parent.len() != n
            || self.parent.iter().filter(|p| p.is_none()).count() != 1
        {
            return Err("join tree must have one root and every occurrence".into());
        }
        let mut neighbors = vec![BTreeSet::new(); n];
        for (i, p) in self.parent.iter().enumerate() {
            if let Some(p) = p {
                if *p >= n || *p == i {
                    return Err("invalid join-tree edge".into());
                }
                neighbors[i].insert(*p);
                neighbors[*p].insert(i);
            }
        }
        let connected = |nodes: &BTreeSet<usize>| {
            let mut pending = vec![*nodes.first().expect("nonempty induced graph")];
            let mut seen = BTreeSet::new();
            while let Some(i) = pending.pop() {
                if seen.insert(i) {
                    pending.extend(neighbors[i].intersection(nodes));
                }
            }
            seen == *nodes
        };
        if !connected(&(0..n).collect()) {
            return Err("disconnected join tree".into());
        }
        for x in edges.iter().flatten().collect::<BTreeSet<_>>() {
            let nodes = (0..n).filter(|i| edges[*i].contains(x)).collect();
            if !connected(&nodes) {
                return Err(format!("running intersection fails for {x}"));
            }
        }
        Ok(())
    }

    pub fn root(&self) -> usize {
        self.parent
            .iter()
            .position(Option::is_none)
            .expect("checked tree")
    }
    pub fn children(&self, parent: usize) -> impl Iterator<Item = usize> + '_ {
        self.parent
            .iter()
            .enumerate()
            .filter_map(move |(i, p)| (*p == Some(parent)).then_some(i))
    }
}

/// Residual graph and absorption forest are the complete GYO administrative state.
pub struct GyoState {
    residual: BTreeMap<usize, Heading>,
    parent: Vec<Option<usize>>,
}

impl GyoState {
    pub fn new(edges: &[Heading]) -> Self {
        Self {
            residual: edges.iter().cloned().enumerate().collect(),
            parent: vec![None; edges.len()],
        }
    }

    /// Premise checking and forest/residual updates are one atomic rule application.
    pub fn apply(&mut self, rule: &GyoRule) -> Result<(), String> {
        match rule {
            GyoRule::Private {
                occurrence,
                variable,
            } => self.remove_private(*occurrence, variable),
            GyoRule::Absorb { child, parent } => self.absorb(*child, *parent),
        }
    }

    /// G-PRIVATE: x belongs to this residual edge and to no other active edge.
    fn remove_private(&mut self, occurrence: usize, variable: &str) -> Result<(), String> {
        if !self
            .residual
            .get(&occurrence)
            .is_some_and(|h| h.contains(variable))
            || self
                .residual
                .iter()
                .any(|(i, h)| *i != occurrence && h.contains(variable))
        {
            return Err("GYO private-variable premise fails".into());
        }
        self.residual.get_mut(&occurrence).unwrap().remove(variable);
        Ok(())
    }

    /// G-ABSORB: the active child is contained in a distinct active parent.
    fn absorb(&mut self, child: usize, parent: usize) -> Result<(), String> {
        if child == parent
            || !self
                .residual
                .get(&child)
                .zip(self.residual.get(&parent))
                .is_some_and(|(child, parent)| child.is_subset(parent))
        {
            return Err("GYO containment premise fails".into());
        }
        self.residual.remove(&child);
        self.parent[child] = Some(parent);
        Ok(())
    }

    /// Deterministic choice of an applicable rule, separate from its meaning.
    pub fn next_rule(&self) -> Option<GyoRule> {
        for (&i, h) in &self.residual {
            for x in h {
                if self.residual.iter().all(|(j, k)| *j == i || !k.contains(x)) {
                    return Some(GyoRule::Private {
                        occurrence: i,
                        variable: x.clone(),
                    });
                }
            }
        }
        for (&i, h) in &self.residual {
            for (&j, k) in &self.residual {
                if i != j && h.is_subset(k) {
                    return Some(GyoRule::Absorb {
                        child: i,
                        parent: j,
                    });
                }
            }
        }
        None
    }

    pub fn finish(&self) -> Result<Option<JoinTree>, String> {
        if self.next_rule().is_some() {
            return Err("GYO derivation is not terminal".into());
        }
        Ok(
            (self.residual.len() == 1 && self.residual.values().all(BTreeSet::is_empty)).then(
                || JoinTree {
                    parent: self.parent.clone(),
                },
            ),
        )
    }
}

pub fn analyze(edges: &[Heading]) -> (Vec<GyoRule>, Option<JoinTree>) {
    let mut state = GyoState::new(edges);
    let mut rules = Vec::new();
    while let Some(rule) = state.next_rule() {
        state.apply(&rule).expect("chosen applicable GYO rule");
        rules.push(rule);
    }
    (rules, state.finish().expect("GYO reached normal form"))
}

/// Original atom incidence; this analysis never emits or removes a relation.
pub(crate) fn incidence(source: &cq::Module) -> Vec<Heading> {
    source
        .program
        .query
        .body
        .iter()
        .filter_map(cq::BodyItem::atom)
        .map(|atom| atom.variables().map(symbol_name).collect())
        .collect()
}

/// Validate the written GYO witness independently of query scheduling.
pub(crate) fn check_analysis(
    edges: &[Heading],
    options: PlanningOptions,
    analysis: &[GyoRule],
    tree: &Option<JoinTree>,
) -> syn::Result<()> {
    if options.strategy == JoinPolicy::Auto {
        let mut gyo = GyoState::new(edges);
        for rule in analysis {
            gyo.apply(rule).map_err(error)?;
        }
        if &gyo.finish().map_err(error)? != tree {
            return Err(error("GYO certificate does not match the chosen route"));
        }
    } else if !analysis.is_empty() || tree.is_some() {
        return Err(error(
            "binary route must not claim a Yannakakis certificate",
        ));
    }
    if let Some(tree) = tree {
        tree.check(edges).map_err(error)?;
    }
    Ok(())
}

#[cfg(test)]
#[allow(dead_code)]
#[path = "../../tests/stage1_reference/oracle.rs"]
mod oracle;
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gyo_matches_independent_tree_enumeration_on_all_small_hypergraphs() {
        // All 4,680 ordered hypergraphs on 1..=4 occurrence nodes and three
        // variables, including empty, duplicate and contained edges.
        for n in 1..=4 {
            for bits in 0..(1usize << (3 * n)) {
                let edges: Vec<Heading> = (0..n)
                    .map(|i| {
                        (0..3)
                            .filter(|x| bits & (1 << (3 * i + x)) != 0)
                            .map(|x| format!("x{x}"))
                            .collect()
                    })
                    .collect();
                let (rules, tree) = analyze(&edges);
                assert_eq!(
                    tree.is_some(),
                    oracle::has_join_tree_for(&edges),
                    "{edges:?}"
                );
                let mut replay = GyoState::new(&edges);
                for rule in &rules {
                    replay.apply(rule).unwrap();
                }
                assert_eq!(replay.finish().unwrap(), tree);
                if let Some(tree) = tree {
                    tree.check(&edges).unwrap();
                }
            }
        }
    }

    #[test]
    fn graph_rules_reject_false_premises_and_incomplete_certificates() {
        let edges = vec![
            BTreeSet::from(["a".into(), "b".into()]),
            BTreeSet::from(["b".into(), "c".into()]),
        ];
        let mut state = GyoState::new(&edges);
        assert!(
            state
                .apply(&GyoRule::Private {
                    occurrence: 0,
                    variable: "b".into()
                })
                .is_err()
        );
        assert!(
            state
                .apply(&GyoRule::Absorb {
                    child: 0,
                    parent: 1
                })
                .is_err()
        );
        assert!(
            state
                .apply(&GyoRule::Absorb {
                    child: 0,
                    parent: 0
                })
                .is_err()
        );
        assert!(state.finish().is_err());
        // Rejections must not damage the graph: the original valid proof still works.
        let (proof, tree) = analyze(&edges);
        for rule in &proof {
            state.apply(rule).unwrap();
        }
        assert_eq!(state.finish().unwrap(), tree);
    }
}
