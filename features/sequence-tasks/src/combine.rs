//! Realizations of checked key partitions. Geometry is supplied by the plan,
//! never by the executing pool. Canon requires full-valuation routing; keyed
//! grouping requires group-key routing and canonical complete valuations.
use rayon::prelude::*;
use std::hash::Hash;
use std::collections::{HashMap, HashSet};
use cq_storage::HashState;

/// Dispatch contiguous hosts to borrowed tiles; non-contiguous row interfaces
/// retain their ordinary enumeration before partitioning.
pub struct TileSource<H>(pub H);

pub trait TileRows {
    type Row: Send;
    fn tile_rows(self, grain: usize) -> impl IndexedParallelIterator<Item = impl Iterator<Item = Self::Row> + Send>;
}

impl<'a, T: Sync> TileRows for &&TileSource<&'a [T]>
where
    &'a T: cq_storage::rows::Columns,
    <&'a T as cq_storage::rows::Columns>::Row: Send,
{
    type Row = <&'a T as cq_storage::rows::Columns>::Row;
    fn tile_rows(self, grain: usize) -> impl IndexedParallelIterator<Item = impl Iterator<Item = Self::Row> + Send> {
        self.0.par_chunks(grain).map(cq_storage::rows::Rows::rows)
    }
}

impl<'a, T: Sync> TileRows for &&TileSource<&'a Vec<T>>
where
    &'a T: cq_storage::rows::Columns,
    <&'a T as cq_storage::rows::Columns>::Row: Send,
{
    type Row = <&'a T as cq_storage::rows::Columns>::Row;
    fn tile_rows(self, grain: usize) -> impl IndexedParallelIterator<Item = impl Iterator<Item = Self::Row> + Send> {
        self.0.par_chunks(grain).map(cq_storage::rows::Rows::rows)
    }
}

impl<'a, T: Sync, const N: usize> TileRows for &&TileSource<&'a [T; N]>
where
    &'a T: cq_storage::rows::Columns,
    <&'a T as cq_storage::rows::Columns>::Row: Send,
{
    type Row = <&'a T as cq_storage::rows::Columns>::Row;
    fn tile_rows(self, grain: usize) -> impl IndexedParallelIterator<Item = impl Iterator<Item = Self::Row> + Send> {
        self.0.par_chunks(grain).map(cq_storage::rows::Rows::rows)
    }
}

impl<'a, T: Sync> TileRows for &&TileSource<&&'a [T]>
where
    &'a T: cq_storage::rows::Columns,
    <&'a T as cq_storage::rows::Columns>::Row: Send,
{
    type Row = <&'a T as cq_storage::rows::Columns>::Row;
    fn tile_rows(self, grain: usize) -> impl IndexedParallelIterator<Item = impl Iterator<Item = Self::Row> + Send> {
        self.0.par_chunks(grain).map(cq_storage::rows::Rows::rows)
    }
}

impl<H: cq_storage::rows::Rows> TileRows for &TileSource<H>
where H::Row: Send {
    type Row = H::Row;
    fn tile_rows(self, grain: usize) -> impl IndexedParallelIterator<Item = impl Iterator<Item = Self::Row> + Send> {
        self.0.rows().collect::<Vec<_>>().into_par_iter().chunks(grain).map(Vec::into_iter)
    }
}

/// Execute an existing serial row adapter independently in each checked part.
pub fn map_parts<R: Send, O: Send>(
    parts: Vec<Vec<R>>, map: impl Fn(Vec<R>) -> Vec<O> + Sync + Send,
) -> Vec<Vec<O>> {
    cq_storage::profile_select! {
        plain { parts.into_par_iter().map(map).collect() }
        counters {{
            let context = cq_storage::counters::TaskContext::capture();
            parts.into_par_iter().map(|part| context.run(|| map(part))).collect()
        }}
    }
}

/// Canonicalize without removing the partition dimension.
pub fn canon_parts<R: Eq + Hash + Send>(parts: Vec<Vec<R>>) -> Vec<Vec<R>> {
    map_parts(parts, |part| part.into_iter().collect::<HashSet<R, HashState>>().into_iter().collect())
}

/// Route task-local chunks, then transpose into exclusive, source-ordered parts.
pub fn route<R: Send, K: Hash>(chunks: Vec<Vec<R>>, count: usize, key: impl Fn(&R) -> K + Sync) -> Vec<Vec<R>> {
    route_by(chunks, count, |row, count| cq_storage::sharded::route_index(&key(row), count))
}

/// Route with a borrowing key projection, avoiding owned key copies solely
/// for hashing. The router must return an index below the plan's positive count.
pub fn route_by<R: Send>(
    chunks: Vec<Vec<R>>, count: usize, router: impl Fn(&R, usize) -> usize + Sync,
) -> Vec<Vec<R>> {
    assert!(count > 0, "key partition count must be positive");
    let _context = cq_storage::profile_select! {
        plain { () }
        counters { cq_storage::counters::TaskContext::capture() }
    };
    let buckets: Vec<Vec<Vec<R>>> = chunks.into_par_iter().map(|chunk| {
        let route = || {
            let mut buckets: Vec<_> = (0..count).map(|_| Vec::new()).collect();
            for row in chunk {
                buckets[router(&row, count)].push(row);
            }
            buckets
        };
        cq_storage::profile_select! {
            plain { route() }
            counters { _context.run(route) }
        }
    }).collect();
    let mut owners: Vec<Vec<Vec<R>>> = (0..count).map(|_| Vec::with_capacity(buckets.len())).collect();
    for buckets in buckets {
        for (owner, bucket) in owners.iter_mut().zip(buckets) { owner.push(bucket); }
    }
    owners.into_par_iter().map(|chunks| {
        let mut rows = Vec::with_capacity(chunks.iter().map(Vec::len).sum());
        for chunk in chunks { rows.extend(chunk); }
        rows
    }).collect()
}

/// Canonicalize each already key-disjoint part, then join. Output order is unspecified.
pub fn par_canon<R: Eq + Hash + Send>(parts: Vec<Vec<R>>) -> Vec<R> {
    canon_parts(parts).into_iter().flatten().collect()
}

/// Fold each already group-key-disjoint part. Empty input has no keyed groups.
pub fn par_group<K: Eq + Hash + Send, S: Send, R: Send>(
    parts: Vec<Vec<R>>, key: impl Fn(&R) -> K + Sync,
    init: impl Fn() -> S + Sync, step: impl Fn(&mut S, R) + Sync,
) -> Vec<(K, S)> {
    map_parts(parts, |part| {
        let mut groups = HashMap::<K, S, HashState>::default();
        for row in part { step(groups.entry(key(&row)).or_insert_with(&init), row); }
        groups.into_iter().collect::<Vec<_>>()
    }).into_iter().flatten().collect()
}

/// Partial monoid folds followed by one host fold, including the empty identity.
/// `merge` must be associative and commutative with identity `init`.
pub fn par_group_global<S: Send, R: Send>(
    parts: Vec<Vec<R>>, init: impl Fn() -> S + Sync,
    step: impl Fn(&mut S, R) + Sync, merge: impl Fn(&mut S, S) + Sync,
) -> S {
    par_group_global_with(parts, |part| {
        let mut state = init();
        for row in part { step(&mut state, row); }
        state
    }, &init, merge)
}

/// Run a serial global-group adapter per part, then merge its states on host.
/// `init` is the empty-input identity; `merge` obeys the commutative monoid laws.
pub fn par_group_global_with<R: Send, S: Send>(
    parts: Vec<Vec<R>>, fold: impl Fn(Vec<R>) -> S + Sync + Send,
    init: impl Fn() -> S, merge: impl Fn(&mut S, S),
) -> S {
    let partials: Vec<S> = cq_storage::profile_select! {
        plain { parts.into_par_iter().map(fold).collect() }
        counters {{
            let context = cq_storage::counters::TaskContext::capture();
            parts.into_par_iter().map(|part| context.run(|| fold(part))).collect()
        }}
    };
    let mut state = init();
    for partial in partials { merge(&mut state, partial); }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contiguous_tiles_borrow_the_original_columns() {
        let input = [(1u32, 2u32), (3, 4), (5, 6)];
        let source = TileSource(input.as_slice());
        let parts: Vec<Vec<_>> = (&&source).tile_rows(2).map(Iterator::collect).collect();
        assert_eq!(parts.iter().map(Vec::len).collect::<Vec<_>>(), [2, 1]);
        assert!(std::ptr::eq(parts[0][0].0, &input[0].0));
        assert!(std::ptr::eq(parts[1][0].1, &input[2].1));
    }
    #[test]
    fn global_adapter_merges_on_host_with_empty_identity() {
        let host = std::thread::current().id();
        for parts in [vec![], vec![vec![]], vec![vec![1u64, 2], vec![], vec![3]]] {
            let expected: u64 = parts.iter().flatten().sum();
            let actual = par_group_global_with(parts, |rows| rows.into_iter().sum::<u64>(), || 0, |sum, partial| {
                assert_eq!(std::thread::current().id(), host);
                *sum += partial;
            });
            assert_eq!(actual, expected);
        }
    }
    #[test]
    fn routing_borrows_owned_keys() {
        struct Row { key: String }
        let chunks = vec![vec![Row { key: "alpha".into() }], vec![], vec![Row { key: "beta".into() }, Row { key: "alpha".into() }]];
        let parts = route_by(chunks, 7, |row, n| cq_storage::sharded::route_index(&(&row.key,), n));
        assert_eq!(parts.iter().map(Vec::len).sum::<usize>(), 3);
        for (part, rows) in parts.iter().enumerate() {
            for row in rows {
                assert_eq!(part, cq_storage::sharded::route_index(&(&row.key,), 7));
            }
        }
    }
    #[test]
    fn serial_equivalence() {
        for threads in [1, 24] {
            rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap().install(|| {
                for rows in [vec![], vec![7u64; 1000], (0..20000).map(|i| i % 1301).collect()] {
                    let mut chunks: Vec<_> = rows.chunks(113).map(<[_]>::to_vec).collect();
                    chunks.push(vec![]);
                    for count in [1, 7, 32] {
                        let routed = route(chunks.clone(), count, |r| *r);
                        assert_eq!(routed.len(), count);
                        let actual = par_canon(routed);
                        let expected: HashSet<_> = rows.iter().copied().collect();
                        assert_eq!(actual.len(), expected.len());
                        assert_eq!(actual.into_iter().collect::<HashSet<_>>(), expected);
                        for groups in [1, 997] {
                            let mut expected = HashMap::new();
                            for &r in &rows { *expected.entry(r % groups).or_insert(0) += r; }
                            let actual = par_group(route(chunks.clone(), count, |r| r % groups), |r| r % groups, || 0, |s, r| *s += r);
                            assert_eq!(actual.len(), expected.len());
                            assert_eq!(actual.into_iter().collect::<HashMap<_, _>>(), expected);
                        }
                    }
                    assert_eq!(par_group_global(chunks, || 0, |s,r| *s += r, |s,r| *s += r), rows.iter().sum::<u64>());
                }
                assert_eq!(par_group_global::<u64,u64>(vec![], || 0, |s,r| *s += r, |s,r| *s += r), 0);
            });
        }
    }
}
