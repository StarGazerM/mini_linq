//! Shared certificate vocabulary; no planning algorithm or route selection.
use crate::Heading;
use std::collections::BTreeSet;

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
