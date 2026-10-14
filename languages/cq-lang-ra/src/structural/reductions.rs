//! A deliberately narrow RA-to-RA certificate for reduction absorption.
//!
//! The ordinary schedule contract preserves independent witness wiring. This
//! additional proof checks actual named-definition substitutions instead of
//! relaxing that contract or identifying witnesses by occurrence number.

use super::{Kind, Schedule};
use crate::{Operator, Program, symbol_name};
use std::collections::{HashMap, HashSet};

pub(super) fn check(source: &Program, target: &Program, schedule: &Schedule) -> bool {
    if source.plan.output != target.plan.output {
        return false;
    }
    // from_program emits exactly one schedule node per named definition.
    let indices: HashMap<_, _> = source
        .plan
        .definitions
        .iter()
        .enumerate()
        .map(|(index, definition)| (symbol_name(&definition.result), index))
        .collect();
    let mut negative = vec![false; schedule.nodes.len()];
    let mut opaque = vec![false; schedule.nodes.len()];
    for (index, node) in schedule.nodes.iter().enumerate() {
        if matches!(node.kind, Kind::Anti(_)) {
            negative[node.operands[1]] = true;
        }
        opaque[index] =
            matches!(node.kind, Kind::Rust(_)) || dependencies(node).any(|input| opaque[input]);
    }
    for (index, node) in schedule.nodes.iter().enumerate().rev() {
        if negative[index] {
            for input in dependencies(node) {
                negative[input] = true;
            }
        }
    }
    let mut retained = vec![false; schedule.nodes.len()];
    for definition in &target.plan.definitions {
        let Some(&index) = indices.get(&symbol_name(&definition.result)) else {
            return false;
        };
        retained[index] = true;
    }
    let proof = Proof {
        schedule,
        negative,
        opaque,
        retained,
    };
    let mut remaining = target.plan.definitions.iter().peekable();
    let mut substituted = false;
    for (index, definition) in source.plan.definitions.iter().enumerate() {
        let Some(other) = remaining
            .peek()
            .filter(|other| definition.result == other.result)
        else {
            // Only a bypassed semijoin can become dead: its base is retained,
            // and its support is still witnessed by the consumer's lineage.
            // The already-checked target contract proves no live reference to
            // this omitted definition remains.
            if !matches!(definition.operator, Operator::Semijoin(_))
                || proof.negative[index]
                || proof.opaque[index]
            {
                return false;
            }
            continue;
        };
        if definition.operator != other.operator {
            if proof.negative[index] {
                return false;
            }
            let replacement = match (&definition.operator, &other.operator) {
                (Operator::Semijoin(a), Operator::Semijoin(b)) if a.left == b.left => &b.right,
                (Operator::NaturalJoin(a), Operator::NaturalJoin(b)) if a.left == b.left => {
                    &b.right
                }
                _ => return false,
            };
            let Some(&replacement) = indices.get(&symbol_name(replacement)) else {
                return false;
            };
            let node = &schedule.nodes[index];
            if !proof.absorbed(node.operands[1], replacement, node.operands[0]) {
                return false;
            }
            substituted = true;
        }
        remaining.next();
    }
    substituted && remaining.next().is_none()
}

fn dependencies(node: &super::Node) -> impl Iterator<Item = usize> + '_ {
    node.operands
        .iter()
        .copied()
        .chain(node.correlation)
        .chain(node.domain)
}

struct Proof<'a> {
    schedule: &'a Schedule,
    negative: Vec<bool>,
    opaque: Vec<bool>,
    retained: Vec<bool>,
}

impl Proof<'_> {
    fn absorbed(&self, mut reduced: usize, replacement: usize, other: usize) -> bool {
        while reduced != replacement {
            let node = &self.schedule.nodes[reduced];
            if !matches!(node.kind, Kind::Join(true))
                || self.negative[reduced]
                || self.opaque[reduced]
                // A surviving restricted row still supplies correlated
                // projected witnesses. All its consumers must keep that
                // same row rather than partially bypass its restrictions.
                || self.retained[reduced]
            {
                return false;
            }
            let base = node.operands[0];
            let support = node.operands[1];
            let shared: Vec<_> = self.schedule.nodes[base]
                .heading
                .keys()
                .filter(|name| self.schedule.nodes[support].heading.contains_key(*name))
                .collect();
            if !self.contained(other, support, &shared) {
                return false;
            }
            reduced = base;
        }
        true
    }

    fn contained(&self, other: usize, support: usize, shared: &[&String]) -> bool {
        let mut pending = vec![other];
        let mut seen = HashSet::new();
        while let Some(index) = pending.pop() {
            let node = &self.schedule.nodes[index];
            if !seen.insert(index) || shared.iter().any(|name| !node.heading.contains_key(*name)) {
                continue;
            }
            if index == support {
                return true;
            }
            match node.kind {
                Kind::Project | Kind::Select | Kind::Join(true) => {
                    pending.push(node.operands[0]);
                }
                Kind::Join(false) => pending.extend(&node.operands),
                _ => {}
            }
        }
        false
    }
}
