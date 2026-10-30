//! Ordinary index maintenance (P3d, `doc/PLANNING-REDESIGN.md`).
//!
//! Distinct and Datalog admission are one construct: each candidate batch is
//! anti-joined against an index of the output relation, and the fresh rows are
//! published into every index of that relation (`doc/formal/incremental.tex`,
//! ADM-Batch). Every index is a complete column permutation; the
//! index-selection pass records which ordinary member is canonical. Nothing
//! here names a canonical, `seen` or `all` store: these are the operations any
//! index provider offers, together with [`crate::ExactMembership`].
//!
//! How a provider deduplicates a batch (hashing, sorted-run merge, radix sort)
//! and when it physically combines storage (eagerly, or lazily as leveled runs)
//! are its own choices under the laws below. Rust bounds enforce capabilities,
//! not these laws.

// DESIGN index-protocol
// PLANNING P3d: canonical is an ordinary index chosen by index selection; no
// separate membership store, provider kind or printer branch may exist.

/// Formal rule IX-Fresh: the distinct rows of a batch absent from the represented relation.
///
/// For an index representing relation `O`, `fresh(batch)` returns exactly
/// `supp(batch) \ O`, without duplicates, in a provider-chosen order. It reads
/// the index only: the represented relation is unchanged. The batch is taken by
/// value so a provider may reuse its allocation.
pub trait Fresh {
    type Row;
    fn fresh(&self, batch: Vec<Self::Row>) -> Vec<Self::Row>;
}

/// Formal rule IX-Publish: extend the represented relation by disjoint, duplicate-free rows.
///
/// Precondition: the items are the index's projection of rows that are
/// pairwise distinct and absent from the represented relation (for example the
/// output of [`Fresh::fresh`] on the canonical index). Afterwards the index
/// represents the old relation plus exactly those rows. Reads between two
/// publications observe the same relation (IX-Stable); a provider may defer
/// physical merging as long as every read observes the published relation.
pub trait Publish {
    type Item;
    fn publish<I: IntoIterator<Item = Self::Item>>(&mut self, items: I);
}

impl<K: Eq + std::hash::Hash, S: std::hash::BuildHasher> Fresh for std::collections::HashSet<K, S> {
    type Row = K;

    fn fresh(&self, mut batch: Vec<K>) -> Vec<K> {
        batch.retain(|row| !self.contains(row));
        distinct_hash(batch)
    }
}

impl<K: Eq + std::hash::Hash, S: std::hash::BuildHasher> Publish
    for std::collections::HashSet<K, S>
{
    type Item = K;

    fn publish<I: IntoIterator<Item = K>>(&mut self, items: I) {
        self.extend(items);
    }
}

impl<K: Ord> Fresh for std::collections::BTreeSet<K> {
    type Row = K;

    fn fresh(&self, mut batch: Vec<K>) -> Vec<K> {
        batch.retain(|row| !self.contains(row));
        batch.sort_unstable();
        batch.dedup();
        batch
    }
}

impl<K: Ord> Publish for std::collections::BTreeSet<K> {
    type Item = K;

    fn publish<I: IntoIterator<Item = K>>(&mut self, items: I) {
        self.extend(items);
    }
}

// Complete-row membership scans the existing Vec bucket: O(g) per candidate
// for group size g. The key directory does not imply a payload index.
impl<K, V, S> Fresh for std::collections::HashMap<K, Vec<V>, S>
where
    K: Eq + std::hash::Hash,
    V: Eq + std::hash::Hash,
    S: std::hash::BuildHasher,
{
    type Row = (K, V);

    fn fresh(&self, mut batch: Vec<Self::Row>) -> Vec<Self::Row> {
        batch.retain(|(key, item)| !self.get(key).is_some_and(|items| items.contains(item)));
        distinct_hash(batch)
    }
}

impl<K, V, S> Publish for std::collections::HashMap<K, Vec<V>, S>
where
    K: Eq + std::hash::Hash,
    S: std::hash::BuildHasher,
{
    type Item = (K, V);

    fn publish<I: IntoIterator<Item = Self::Item>>(&mut self, items: I) {
        for (key, item) in items {
            self.entry(key).or_default().push(item);
        }
    }
}

impl<K, V, S, T> Fresh for std::collections::HashMap<K, std::collections::HashSet<V, T>, S>
where
    K: Eq + std::hash::Hash,
    V: Eq + std::hash::Hash,
    S: std::hash::BuildHasher,
    T: std::hash::BuildHasher,
{
    type Row = (K, V);

    fn fresh(&self, mut batch: Vec<Self::Row>) -> Vec<Self::Row> {
        batch.retain(|(key, item)| !self.get(key).is_some_and(|items| items.contains(item)));
        distinct_hash(batch)
    }
}

impl<K, V, S, T> Publish for std::collections::HashMap<K, std::collections::HashSet<V, T>, S>
where
    K: Eq + std::hash::Hash,
    V: Eq + std::hash::Hash,
    S: std::hash::BuildHasher,
    T: std::hash::BuildHasher + Default,
{
    type Item = (K, V);

    fn publish<I: IntoIterator<Item = Self::Item>>(&mut self, items: I) {
        for (key, item) in items {
            self.entry(key)
                .or_insert_with(|| std::collections::HashSet::with_hasher(T::default()))
                .insert(item);
        }
    }
}

// Payload membership is O(g) per candidate even though keys are ordered.
// A hashed/ordered payload bucket is a different, explicitly selected provider.
impl<K: Ord, V: Ord> Fresh for std::collections::BTreeMap<K, Vec<V>> {
    type Row = (K, V);

    fn fresh(&self, mut batch: Vec<Self::Row>) -> Vec<Self::Row> {
        batch.retain(|(key, item)| !self.get(key).is_some_and(|items| items.contains(item)));
        batch.sort_unstable();
        batch.dedup();
        batch
    }
}

impl<K: Ord, V> Publish for std::collections::BTreeMap<K, Vec<V>> {
    type Item = (K, V);

    fn publish<I: IntoIterator<Item = Self::Item>>(&mut self, items: I) {
        for (key, item) in items {
            self.entry(key).or_default().push(item);
        }
    }
}

/// Hash rows by reference, then compact their original batch allocation after
/// the references have been released. No row Clone or output Vec is needed.
fn distinct_hash<R: Eq + std::hash::Hash>(mut batch: Vec<R>) -> Vec<R> {
    if batch.len() < 2 {
        return batch;
    }
    let mut keep = vec![0u64; batch.len().div_ceil(64)];
    let mut distinct = crate::HashSet::default();
    distinct.reserve(batch.len());
    for (position, row) in batch.iter().enumerate() {
        if distinct.insert(row) {
            keep[position / 64] |= 1 << (position % 64);
        }
    }
    drop(distinct);
    let mut position = 0;
    batch.retain(|_| {
        let retained = keep[position / 64] & (1 << (position % 64)) != 0;
        position += 1;
        retained
    });
    batch
}

/// Deduplicate full rows after exact membership filtering. Payloads need only
/// equality: grouping by key confines duplicate comparisons to one payload group.
/// With equality-only payloads, a batch group of g rows needs O(g²) equality
/// comparisons in the worst case. Faster bucket layouts are separate providers.
pub(crate) fn distinct_pairs<K: Ord, V: Eq>(mut batch: Vec<(K, V)>) -> Vec<(K, V)> {
    batch.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let mut begin = 0;
    let mut end = 0;
    for next in 0..batch.len() {
        if end == 0 || batch[next].0 != batch[begin].0 {
            begin = end;
        } else if batch[begin..end].iter().any(|row| row.1 == batch[next].1) {
            continue;
        }
        batch.swap(end, next);
        end += 1;
    }
    batch.truncate(end);
    batch
}
