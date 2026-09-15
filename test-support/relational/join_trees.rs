//! Independent bounded tree-enumeration oracle; no GYO implementation.
use cq_ir::{cq, symbol_name};
use std::collections::BTreeSet;

/// Independent small-query oracle for future GYO tests: enumerate trees and
/// check running intersection directly, without any hypergraph reductions.
pub fn has_join_tree(program: &cq::Program) -> bool {
    let headings = program
        .query
        .body
        .iter()
        .filter_map(cq::BodyItem::atom)
        .map(|atom| atom.variables().map(symbol_name).collect::<BTreeSet<_>>())
        .collect::<Vec<_>>();
    has_join_tree_for(&headings)
}

pub fn has_join_tree_for(headings: &[BTreeSet<String>]) -> bool {
    let n = headings.len();
    assert!(
        (1..=7).contains(&n),
        "bounded oracle supports one to seven occurrences"
    );
    if n == 1 {
        return true;
    }
    // Prüfer codes enumerate exactly the n^(n-2) labeled trees. At seven
    // occurrences this checks 16,807 trees instead of 2^21 edge subsets.
    let variables = headings
        .iter()
        .flat_map(|h| h.iter())
        .collect::<BTreeSet<_>>();
    for mut code in 0..n.pow((n - 2) as u32) {
        let mut sequence = Vec::new();
        let mut degree = vec![1; n];
        for _ in 0..n - 2 {
            let vertex = code % n;
            sequence.push(vertex);
            degree[vertex] += 1;
            code /= n;
        }
        let mut adjacent = vec![BTreeSet::new(); n];
        for vertex in sequence {
            let leaf = degree.iter().position(|d| *d == 1).unwrap();
            adjacent[leaf].insert(vertex);
            adjacent[vertex].insert(leaf);
            degree[leaf] -= 1;
            degree[vertex] -= 1;
        }
        let leaves = (0..n).filter(|i| degree[*i] == 1).collect::<Vec<_>>();
        adjacent[leaves[0]].insert(leaves[1]);
        adjacent[leaves[1]].insert(leaves[0]);
        if variables.iter().all(|x| {
            let containing = (0..n).filter(|i| headings[*i].contains(*x)).collect();
            connected(&containing, &adjacent)
        }) {
            return true;
        }
    }
    false
}

fn connected(nodes: &BTreeSet<usize>, adjacent: &[BTreeSet<usize>]) -> bool {
    let mut pending = vec![*nodes.first().unwrap()];
    let mut visited = BTreeSet::new();
    while let Some(i) = pending.pop() {
        if visited.insert(i) {
            pending.extend(adjacent[i].intersection(nodes));
        }
    }
    visited == *nodes
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tree_oracle_handles_degenerate_duplicate_and_seven_node_cases() {
        let h = |xs: &[&str]| xs.iter().map(|x| (*x).to_owned()).collect::<BTreeSet<_>>();
        assert!(has_join_tree_for(&[h(&["a", "b"])]));
        assert!(has_join_tree_for(&[h(&["a", "b"]), h(&["a", "b"])]));
        assert!(has_join_tree_for(&[h(&["a"]), h(&["b"])]));
        let path = (0..7)
            .map(|i| BTreeSet::from([format!("x{i}"), format!("x{}", i + 1)]))
            .collect::<Vec<_>>();
        assert!(has_join_tree_for(&path));
        let cycle = (0..7)
            .map(|i| BTreeSet::from([format!("x{i}"), format!("x{}", (i + 1) % 7)]))
            .collect::<Vec<_>>();
        assert!(!has_join_tree_for(&cycle));
    }
}
