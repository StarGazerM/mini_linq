//! Lock-free, shard-owned storage for parallel Datalog rounds.
//!
//! Construct providers inside the Rayon pool that will execute the program.
//! Their fixed shard count is the next power of two at least as large as that
//! pool's thread count. Routing and membership use the recorded build hasher
//! together: each candidate is hashed once, then its hash travels to its shard.
//! Equal values reach the same shard within a provider, independently of worker
//! scheduling. Random-seeded providers need not agree across instances.
//!
//! Admission routes by the entire tuple. Each parallel task owns an exclusive
//! mutable seen shard; native layout maps are published by the extension printer.
use cq_storage::HashState;
use hashbrown::HashTable;
use hashbrown::hash_table::Entry;
use rayon::prelude::*;
use std::hash::{BuildHasher, Hash};

type SeenShard<T> = HashTable<T>;

fn shard_count() -> usize {
    rayon::current_num_threads().next_power_of_two()
}

/// Full-tuple membership shared across admission barriers, not across workers.
///
/// Only successful admissions clone a tuple: one owned value remains in the
/// seen set and the other is returned for publication into relation layouts.
/// Initial `insert` does not require `Clone` or `Send`.
pub struct ShardedSeen<T, S = HashState> {
    shards: Vec<SeenShard<T>>,
    routing: S,
}

impl<T, S: Default> Default for ShardedSeen<T, S> {
    fn default() -> Self {
        Self::with_hasher(S::default())
    }
}

impl<T, S: Default> ShardedSeen<T, S> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<T, S> ShardedSeen<T, S> {
    pub fn with_hasher(routing: S) -> Self {
        Self {
            shards: (0..shard_count()).map(|_| SeenShard::new()).collect(),
            routing,
        }
    }
}

impl<T: Eq + Hash, S: BuildHasher> ShardedSeen<T, S> {
    /// Insert an initial tuple, returning whether it was absent.
    pub fn insert(&mut self, value: T) -> bool {
        let hash = self.routing.hash_one(&value);
        let shard = hash as usize & (self.shards.len() - 1);
        match self.shards[shard].entry(hash, |old| old == &value, |old| self.routing.hash_one(old))
        {
            Entry::Occupied(_) => false,
            Entry::Vacant(entry) => {
                entry.insert(value);
                true
            }
        }
    }
}

/// Owned full-tuple candidates for one evaluation task.
///
/// This is a local realization choice, not an admission owner: duplicate rows
/// from other tasks or earlier rounds are still rejected by `ShardedSeen`.
/// The table borrows that owner's hasher, preserving keyed/Fx selection without
/// cloning the hasher or any candidate row. First-occurrence order is retained,
/// so this optimization does not perturb admission and layout traversal order.
pub struct TaskCandidates<'a, T, S> {
    positions: HashTable<usize>,
    rows: Vec<T>,
    routing: &'a S,
}

impl<T, S> ShardedSeen<T, S> {
    /// Borrow only the hasher, without capturing membership storage in a task.
    pub fn hasher(&self) -> &S {
        &self.routing
    }
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

impl<T: Eq + Hash + Clone + Send, S: BuildHasher + Sync> ShardedSeen<T, S> {
    /// Formal rule DL-ShardedAdmission: Admit unseen full tuples by exclusive shard ownership.
    ///
    /// Results concatenate shards in ascending order and preserve candidate
    /// order within each shard. This order is identical for inline and parallel
    /// admission. Fewer than `threshold` candidates execute inline; zero forces
    /// parallel admission for any nonempty input when multiple shards exist.
    pub fn admit(&mut self, candidates: Vec<Vec<T>>, threshold: usize) -> Vec<T> {
        let count = candidates.iter().map(Vec::len).sum::<usize>();
        if count == 0 {
            return Vec::new();
        }
        if count < threshold || self.shards.len() == 1 {
            // Small rounds need neither S input buckets nor S output buffers.
            // Stable shard ordering preserves the same concatenation law.
            let mut admitted = Vec::with_capacity(count);
            for row in candidates.into_iter().flatten() {
                let hash = self.routing.hash_one(&row);
                let shard = hash as usize & (self.shards.len() - 1);
                if let Entry::Vacant(entry) = self.shards[shard].entry(
                    hash,
                    |old| old == &row,
                    |old| self.routing.hash_one(old),
                ) {
                    admitted.push((shard, row.clone()));
                    entry.insert(row);
                }
            }
            admitted.sort_by_key(|(shard, _)| *shard);
            return admitted.into_iter().map(|(_, row)| row).collect();
        }
        let mut buckets: Vec<Vec<(u64, T)>> = (0..self.shards.len()).map(|_| Vec::new()).collect();
        for row in candidates.into_iter().flatten() {
            let hash = self.routing.hash_one(&row);
            let shard = hash as usize & (buckets.len() - 1);
            buckets[shard].push((hash, row));
        }
        let admitted: Vec<Vec<T>> = self
            .shards
            .par_iter_mut()
            .zip(buckets.into_par_iter())
            .map(|(seen, rows)| admit_shard(seen, rows, &self.routing))
            .collect();
        let admitted_count = admitted.iter().map(Vec::len).sum();
        let mut result = Vec::with_capacity(admitted_count);
        for shard in admitted {
            result.extend(shard);
        }
        result
    }
}

fn admit_shard<T: Eq + Hash + Clone, S: BuildHasher>(
    seen: &mut SeenShard<T>,
    rows: Vec<(u64, T)>,
    routing: &S,
) -> Vec<T> {
    rows.into_iter()
        .filter_map(|(hash, row)| {
            match seen.entry(hash, |old| old == &row, |old| routing.hash_one(old)) {
                Entry::Occupied(_) => None,
                Entry::Vacant(entry) => {
                    let admitted = row.clone();
                    entry.insert(row);
                    Some(admitted)
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
