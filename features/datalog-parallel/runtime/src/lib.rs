//! Lock-free, shard-owned storage for parallel Datalog rounds.
//!
//! Construct providers inside the Rayon pool that will execute the program.
//! Their fixed shard count is the next power of two at least as large as that
//! pool's thread count. Routing and membership use the recorded build hasher
//! together: each key is hashed once, then its hash travels to its shard.
//! Equal values reach the same shard within a provider, independently of worker
//! scheduling. Random-seeded providers need not agree across instances.
//!
//! Admission uses each relation's canonical layout (a `ShardedLayout` or
//! `RunLayout` keyed by every column); there is no separate seen store.
pub mod layout;
pub mod relational;
pub mod runs;
use hashbrown::HashTable;
use hashbrown::hash_table::Entry;
pub use layout::ShardedLayout;
pub use runs::RunLayout;
use std::hash::{BuildHasher, Hash};

/// Owned full-tuple candidates for one evaluation task.
///
/// This is a local realization choice, not an admission owner: duplicate rows
/// from other tasks or earlier rounds are still rejected by canonical admission.
/// The table borrows a hasher, without cloning it or any candidate row.
/// First-occurrence order is retained, so this optimization does not perturb
/// admission and layout traversal order.
pub struct TaskCandidates<'a, T, S> {
    positions: HashTable<usize>,
    rows: Vec<T>,
    routing: &'a S,
}

impl<'a, T, S> TaskCandidates<'a, T, S> {
    pub fn with_hasher(routing: &'a S) -> Self {
        Self {
            positions: HashTable::new(),
            rows: Vec::new(),
            routing,
        }
    }
}

impl<T: Eq + Hash, S: BuildHasher> TaskCandidates<'_, T, S> {
    pub fn insert(&mut self, row: T) {
        let hash = self.routing.hash_one(&row);
        if let Entry::Vacant(entry) = self.positions.entry(
            hash,
            |&index| self.rows[index] == row,
            |&index| self.routing.hash_one(&self.rows[index]),
        ) {
            entry.insert(self.rows.len());
            self.rows.push(row);
        }
    }
}

impl<T, S> TaskCandidates<'_, T, S> {
    pub fn into_rows(self) -> Vec<T> {
        self.rows
    }
}

#[cfg(test)]
mod tests;
