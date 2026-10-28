//! Immutable sorted runs for CPU reference publication and admission.
//!
//! Runs are geometrically compacted, never deduplicating layout occurrences.
//! Lookup borrows a stored key and visits each run's contiguous matching range;
//! its cursor allocates nothing and outlives the temporary query key.
use cq_storage::rows::{Columns, Concat, Rows};
use cq_storage::{Candidates, ExactMembership, HashState, IndexRead, PointLookup, PointRows};
use rayon::prelude::*;
use std::borrow::Borrow;
use std::collections::BTreeSet;

const RATIO: usize = 4;

fn merge<T: Ord>(
    left: impl Iterator<Item = T>,
    right: impl Iterator<Item = T>,
) -> impl Iterator<Item = T> {
    let mut left = left.peekable();
    let mut right = right.peekable();
    std::iter::from_fn(move || match (left.peek(), right.peek()) {
        (Some(a), Some(b)) if a <= b => left.next(),
        (Some(_), Some(_)) => right.next(),
        (Some(_), None) => left.next(),
        (None, _) => right.next(),
    })
}

struct Run<K, V> {
    keys: Vec<K>,
    values: Vec<V>,
}

impl<K: Ord, V: Ord> Run<K, V> {
    fn from_sorted(rows: impl Iterator<Item = (K, V)>, len: usize) -> Self {
        let mut run = Self {
            keys: Vec::with_capacity(len),
            values: Vec::with_capacity(len),
        };
        for (key, value) in rows {
            run.keys.push(key);
            run.values.push(value);
        }
        run
    }

    fn range<Q: Ord + ?Sized>(&self, key: &Q) -> std::ops::Range<usize>
    where
        K: Borrow<Q>,
    {
        let start = self.keys.partition_point(|stored| stored.borrow() < key);
        let end = start + self.keys[start..].partition_point(|stored| stored.borrow() == key);
        start..end
    }

    fn merge(self, other: Self) -> Self {
        let len = self.keys.len() + other.keys.len();
        Self::from_sorted(
            merge(
                self.keys.into_iter().zip(self.values),
                other.keys.into_iter().zip(other.values),
            ),
            len,
        )
    }
}

/// A grouped bag represented by sorted struct-of-arrays runs.
pub struct RunLayout<K: Ord, V: Ord> {
    runs: Vec<Run<K, V>>,
    len: usize,
}

impl<K: Ord, V: Ord> Default for RunLayout<K, V> {
    fn default() -> Self {
        Self {
            runs: Vec::new(),
            len: 0,
        }
    }
}

impl<K: Ord, V: Ord> RunLayout<K, V> {
    fn append(&mut self, run: Run<K, V>) {
        if run.keys.is_empty() {
            return;
        }
        self.len += run.keys.len();
        self.runs.push(run);
        while self.runs.len() >= 2 {
            let last = self.runs.len() - 1;
            if self.runs[last - 1].keys.len() > self.runs[last].keys.len().saturating_mul(RATIO) {
                break;
            }
            let right = self.runs.pop().unwrap();
            let left = self.runs.pop().unwrap();
            self.runs.push(left.merge(right));
        }
    }

    pub fn push(&mut self, key: K, value: V) {
        self.append(Run {
            keys: vec![key],
            values: vec![value],
        });
    }

    /// Formal rule DL-RunLayout: append a sorted batch and merge adjacent runs without changing the represented grouped bag.
    pub fn extend_from<T, FK, FV>(&mut self, rows: &[T], key: FK, value: FV)
    where
        T: Sync,
        K: Send,
        V: Send,
        FK: Fn(&T) -> K + Sync,
        FV: Fn(&T) -> V + Sync,
    {
        if rows.is_empty() {
            return;
        }
        let mut batch: Vec<_> = rows.par_iter().map(|row| (key(row), value(row))).collect();
        batch.par_sort_unstable();
        self.append(Run::from_sorted(batch.into_iter(), rows.len()));
    }

    pub fn clear(&mut self) {
        self.runs.clear();
        self.len = 0;
    }
    /// Total row occurrences, not the number of distinct keys.
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn cursor(&self, key: &K) -> RunValues<'_, K, V> {
        for (index, run) in self.runs.iter().enumerate() {
            let range = run.range(key);
            if !range.is_empty() {
                return RunValues {
                    runs: &self.runs,
                    key: Some(&run.keys[range.start]),
                    run: index,
                    values: run.values[range].iter(),
                };
            }
        }
        RunValues {
            runs: &self.runs,
            key: None,
            run: self.runs.len(),
            values: [].iter(),
        }
    }
}

/// Allocation-free lazy cursor over the matching range of each run.
pub struct RunValues<'a, K, V> {
    runs: &'a [Run<K, V>],
    key: Option<&'a K>,
    run: usize,
    values: std::slice::Iter<'a, V>,
}

impl<'a, K: Ord, V: Ord> Iterator for RunValues<'a, K, V> {
    type Item = &'a V;
    fn next(&mut self) -> Option<Self::Item> {
        let key = self.key?;
        loop {
            if let Some(value) = self.values.next() {
                return Some(value);
            }
            self.run += 1;
            let run = self.runs.get(self.run)?;
            self.values = run.values[run.range(key)].iter();
        }
    }
}

impl<'a, K: Ord, V: Ord> IndexRead for &'a RunLayout<K, V> {
    type Key = K;
    type Item = &'a V;
    type Cursor = RunValues<'a, K, V>;
    fn lookup(&self, key: &K) -> Self::Cursor {
        self.cursor(key)
    }
}

impl<K: Ord + Borrow<Q>, V: Ord, Q: Ord + ?Sized> ExactMembership<Q> for RunLayout<K, V> {
    fn contains(&self, key: &Q) -> bool {
        self.runs.iter().any(|run| {
            run.keys
                .binary_search_by(|stored| stored.borrow().cmp(key))
                .is_ok()
        })
    }
}

/// Enumerate each payload once, without allocating a second candidate set.
pub struct RunCandidates<'a, K, V> {
    source: RunValues<'a, K, V>,
    previous: Option<(usize, &'a V)>,
}

impl<'a, K: Ord, V: Ord> Iterator for RunCandidates<'a, K, V> {
    type Item = &'a V;
    fn next(&mut self) -> Option<Self::Item> {
        while let Some(value) = self.source.next() {
            let index = self.source.run;
            let repeated = self
                .previous
                .is_some_and(|(run, old)| run == index && old == value);
            self.previous = Some((index, value));
            if repeated {
                continue;
            }
            let key = self.source.key.unwrap();
            if !self.source.runs[..index]
                .iter()
                .any(|run| run.values[run.range(key)].binary_search(value).is_ok())
            {
                return Some(value);
            }
        }
        None
    }
}

impl<'a, K: Ord, V: Ord> Candidates for &'a RunLayout<K, V> {
    type Key = K;
    type Item = &'a V;
    type Iter = RunCandidates<'a, K, V>;
    fn count(&self, key: &K) -> usize {
        self.runs.iter().map(|run| run.range(key).len()).sum()
    }
    fn keys(&self, key: &K) -> Self::Iter {
        RunCandidates {
            source: self.cursor(key),
            previous: None,
        }
    }
}

impl<'a, K: Ord, V: Ord> PointLookup for &'a RunLayout<K, V>
where
    &'a K: Columns,
    &'a V: Columns,
    <&'a K as Columns>::Row: Concat<<&'a V as Columns>::Row>,
{
    type Key = K;
    type Row = <<&'a K as Columns>::Row as Concat<<&'a V as Columns>::Row>>::Row;
    type Cursor = PointRows<&'a K, RunValues<'a, K, V>>;
    fn point_lookup(&self, key: &K) -> Self::Cursor {
        let cursor = self.cursor(key);
        PointRows::new(cursor.key, cursor)
    }
}

impl<'a, K: Ord, V: Ord> Rows for &'a RunLayout<K, V>
where
    &'a K: Columns,
    &'a V: Columns,
    <&'a K as Columns>::Row: Concat<<&'a V as Columns>::Row>,
{
    type Row = <<&'a K as Columns>::Row as Concat<<&'a V as Columns>::Row>>::Row;
    fn rows(self) -> impl Iterator<Item = Self::Row> {
        self.runs.iter().flat_map(|run| {
            run.keys
                .iter()
                .zip(&run.values)
                .map(|(key, value)| key.columns().concat(value.columns()))
        })
    }
}

/// Exact full-tuple admission. The hasher belongs only to task-local deduplication.
/// Seeds stage in an ordered set, so insertion stays exact without one run per row.
pub struct RunSeen<T: Ord + Clone, S = HashState> {
    runs: Vec<Vec<T>>,
    seeds: BTreeSet<T>,
    hasher: S,
}

impl<T: Ord + Clone, S: Default> Default for RunSeen<T, S> {
    fn default() -> Self {
        Self::with_hasher(S::default())
    }
}

impl<T: Ord + Clone, S: Default> RunSeen<T, S> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<T: Ord + Clone, S> RunSeen<T, S> {
    pub fn with_hasher(hasher: S) -> Self {
        Self {
            runs: Vec::new(),
            seeds: BTreeSet::new(),
            hasher,
        }
    }
    pub fn hasher(&self) -> &S {
        &self.hasher
    }

    pub fn insert(&mut self, row: T) -> bool {
        if self.runs.iter().any(|run| run.binary_search(&row).is_ok()) {
            return false;
        }
        self.seeds.insert(row)
    }

    fn append(&mut self, rows: Vec<T>) {
        if rows.is_empty() {
            return;
        }
        self.runs.push(rows);
        while self.runs.len() >= 2 {
            let last = self.runs.len() - 1;
            if self.runs[last - 1].len() > self.runs[last].len().saturating_mul(RATIO) {
                break;
            }
            let right = self.runs.pop().unwrap();
            let left = self.runs.pop().unwrap();
            let mut merged = Vec::with_capacity(left.len() + right.len());
            merged.extend(merge(left.into_iter(), right.into_iter()));
            self.runs.push(merged);
        }
    }
}

impl<T: Ord + Clone + Send + Sync, S> RunSeen<T, S> {
    /// Formal rule DL-RunAdmission: sort and deduplicate candidates, subtract accepted runs, then append exactly the fresh sorted tuples.
    pub fn admit(&mut self, candidates: Vec<Vec<T>>, threshold: usize) -> Vec<T> {
        if !self.seeds.is_empty() {
            let seeds = std::mem::take(&mut self.seeds).into_iter().collect();
            self.append(seeds);
        }
        let count = candidates.iter().map(Vec::len).sum();
        let mut rows = Vec::with_capacity(count);
        for chunk in candidates {
            rows.extend(chunk);
        }
        rows.par_sort_unstable();
        rows.dedup();
        let runs = &self.runs;
        let fresh: Vec<T> = if rows.len() < threshold {
            rows.into_iter()
                .filter(|row| !runs.iter().any(|run| run.binary_search(row).is_ok()))
                .collect()
        } else {
            rows.into_par_iter()
                .filter(|row| !runs.iter().any(|run| run.binary_search(row).is_ok()))
                .collect()
        };
        self.append(fresh.clone());
        fresh
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn assert_runs<K: Ord, V: Ord>(layout: &RunLayout<K, V>) {
        assert!(
            layout
                .runs
                .windows(2)
                .all(|pair| pair[0].keys.len() > RATIO * pair[1].keys.len())
        );
        let bound = if layout.is_empty() {
            0
        } else {
            layout.len().ilog(RATIO) as usize + 1
        };
        assert!(layout.runs.len() <= bound);
        for run in &layout.runs {
            assert_eq!(run.keys.len(), run.values.len());
            assert!(run.keys.iter().zip(&run.values).is_sorted());
        }
    }

    #[test]
    fn batches_preserve_grouped_bags_and_all_reads() {
        for threads in [1, 4, 8] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    let mut layout = RunLayout::<(u32,), (u32,)>::default();
                    let mut expected = HashMap::<(u32,), Vec<(u32,)>>::new();
                    for batch in 0..80 {
                        let rows: Vec<_> = (0..(batch * 97 % 511))
                            .map(|i| ((i + batch) % 23, i % 31))
                            .collect();
                        layout.extend_from(&rows, |r| (r.0,), |r| (r.1,));
                        for (k, v) in rows {
                            expected.entry((k,)).or_default().push((v,));
                        }
                        assert_runs(&layout);
                        assert_eq!(layout.len(), expected.values().map(Vec::len).sum());
                        for k in 0..25 {
                            let mut wanted = expected.get(&(k,)).cloned().unwrap_or_default();
                            wanted.sort_unstable();
                            // Query keys need not remain alive for their cursors.
                            let mut actual: Vec<_> = {
                                let key = (k,);
                                (&layout).lookup(&key)
                            }
                            .copied()
                            .collect();
                            actual.sort_unstable();
                            assert_eq!(actual, wanted);
                            assert_eq!(layout.contains(&(k,)), !wanted.is_empty());
                            assert_eq!((&layout).count(&(k,)), wanted.len());
                            let mut point: Vec<_> = (&layout)
                                .point_lookup(&(k,))
                                .map(|(key, value)| {
                                    assert_eq!(*key, k);
                                    (*value,)
                                })
                                .collect();
                            point.sort_unstable();
                            assert_eq!(point, wanted);
                            wanted.dedup();
                            let mut distinct: Vec<_> = (&layout).keys(&(k,)).copied().collect();
                            distinct.sort_unstable();
                            assert_eq!(distinct, wanted);
                        }
                        let mut scanned: Vec<_> = (&layout).rows().map(|(k, v)| (*k, *v)).collect();
                        scanned.sort_unstable();
                        let mut wanted: Vec<_> = expected
                            .iter()
                            .flat_map(|(k, vs)| vs.iter().map(move |v| (k.0, v.0)))
                            .collect();
                        wanted.sort_unstable();
                        assert_eq!(scanned, wanted);
                    }
                    layout.clear();
                    assert!(layout.is_empty());
                    assert_eq!(layout.len(), 0);
                    assert_eq!((&layout).lookup(&(0,)).next(), None);
                    layout.push((1,), (2,));
                    assert_eq!((&layout).lookup(&(1,)).copied().collect::<Vec<_>>(), [(2,)]);
                });
        }
    }

    #[test]
    fn many_small_appends_keep_geometric_run_bound() {
        let mut layout = RunLayout::<u32, u32>::default();
        for i in 0..20_000 {
            layout.push(i % 7, i);
            assert_runs(&layout);
        }
        assert_eq!(layout.len(), 20_000);
    }

    #[test]
    fn admission_is_sorted_set_difference_including_seeds() {
        for threads in [1, 4, 8] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    for threshold in [0, usize::MAX] {
                        let mut seen = RunSeen::<u32>::default();
                        let mut expected = BTreeSet::new();
                        for row in (0..1500).map(|i| i % 997) {
                            assert_eq!(seen.insert(row), expected.insert(row));
                        }
                        assert!(seen.runs.is_empty());
                        for batch in 0..70 {
                            let chunks: Vec<Vec<_>> = (0..5)
                                .map(|chunk| {
                                    (0..120)
                                        .map(|i| (batch * 103 + chunk * 17 + i * 3) % 7001)
                                        .collect()
                                })
                                .collect();
                            let candidates: BTreeSet<_> =
                                chunks.iter().flatten().copied().collect();
                            let wanted: Vec<_> =
                                candidates.difference(&expected).copied().collect();
                            let fresh = seen.admit(chunks, threshold);
                            assert_eq!(fresh, wanted);
                            expected.extend(fresh);
                            assert!(
                                seen.runs
                                    .windows(2)
                                    .all(|pair| pair[0].len() > RATIO * pair[1].len())
                            );
                            assert!(seen.runs.iter().all(|run| run.is_sorted()));
                            let accepted: BTreeSet<_> =
                                seen.runs.iter().flatten().copied().collect();
                            assert_eq!(accepted, expected);
                            assert_eq!(
                                seen.runs.iter().map(Vec::len).sum::<usize>(),
                                expected.len()
                            );
                            assert_eq!(seen.insert(batch), expected.insert(batch));
                            assert_eq!(
                                seen.insert(10_000 + batch),
                                expected.insert(10_000 + batch)
                            );
                        }
                        assert!(seen.admit(vec![], threshold).is_empty());
                        assert!(seen.seeds.is_empty());
                    }
                });
        }
    }
}
