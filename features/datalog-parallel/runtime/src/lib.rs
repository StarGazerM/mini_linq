//! Lock-free, shard-owned storage for parallel Datalog rounds.
//!
//! Construct providers inside the Rayon pool that will execute the program.
//! Their fixed shard count is the next power of two at least as large as that
//! pool's thread count. Routing uses a process-local `HashState` cloned into
//! each provider at construction. Equal values reach the same shard for a
//! given shard count, independently of worker scheduling or provider instance.
//! Routing hashes are not stable across processes or a persistent wire format.
//!
//! Admission routes by the entire tuple. Each parallel task owns an exclusive
//! mutable seen shard; native layout maps are published by the extension printer.
use cq_storage::HashState;
use rayon::prelude::*;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::hash::{BuildHasher, Hash};
use std::sync::LazyLock;

type SeenShard<T> = HashMap<T, (), HashState>;

fn shard_count() -> usize {
    rayon::current_num_threads().next_power_of_two()
}

static ROUTING: LazyLock<HashState> = LazyLock::new(HashState::default);

#[inline]
fn shard_for<T: Hash + ?Sized>(value: &T, routing: &HashState, count: usize) -> usize {
    routing.hash_one(value) as usize & (count - 1)
}

/// Full-tuple membership shared across admission barriers, not across workers.
///
/// Only successful admissions clone a tuple: one owned value remains in the
/// seen set and the other is returned for publication into relation layouts.
/// Initial `insert` does not require `Clone` or `Send`.
pub struct ShardedSeen<T> {
    shards: Vec<SeenShard<T>>,
    routing: HashState,
}

impl<T> Default for ShardedSeen<T> {
    fn default() -> Self {
        Self {
            shards: (0..shard_count()).map(|_| SeenShard::default()).collect(),
            routing: ROUTING.clone(),
        }
    }
}

impl<T> ShardedSeen<T> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<T: Eq + Hash> ShardedSeen<T> {
    /// Insert an initial tuple, returning whether it was absent.
    pub fn insert(&mut self, value: T) -> bool {
        let shard = shard_for(&value, &self.routing, self.shards.len());
        self.shards[shard].insert(value, ()).is_none()
    }
}

impl<T: Eq + Hash + Clone + Send> ShardedSeen<T> {
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
                let shard = shard_for(&row, &self.routing, self.shards.len());
                if let Entry::Vacant(entry) = self.shards[shard].entry(row) {
                    admitted.push((shard, entry.key().clone()));
                    entry.insert(());
                }
            }
            admitted.sort_by_key(|(shard, _)| *shard);
            return admitted.into_iter().map(|(_, row)| row).collect();
        }
        let mut buckets: Vec<Vec<T>> = (0..self.shards.len()).map(|_| Vec::new()).collect();
        for row in candidates.into_iter().flatten() {
            let shard = shard_for(&row, &self.routing, buckets.len());
            buckets[shard].push(row);
        }
        let admitted: Vec<Vec<T>> = self
            .shards
            .par_iter_mut()
            .zip(buckets.into_par_iter())
            .map(|(seen, rows)| admit_shard(seen, rows))
            .collect();
        let admitted_count = admitted.iter().map(Vec::len).sum();
        let mut result = Vec::with_capacity(admitted_count);
        for shard in admitted {
            result.extend(shard);
        }
        result
    }
}

fn admit_shard<T: Eq + Hash + Clone>(seen: &mut SeenShard<T>, rows: Vec<T>) -> Vec<T> {
    rows.into_iter()
        .filter_map(|row| match seen.entry(row) {
            Entry::Occupied(_) => None,
            Entry::Vacant(entry) => {
                let admitted = entry.key().clone();
                entry.insert(());
                Some(admitted)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests;
