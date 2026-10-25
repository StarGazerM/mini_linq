//! Realizations of checked key partitions. Geometry is supplied by the plan,
//! never by the executing pool. Canon requires full-valuation routing; keyed
//! grouping requires group-key routing and canonical complete valuations.
use rayon::prelude::*;

/// Dispatch contiguous hosts to borrowed tiles; non-contiguous row interfaces
/// retain their ordinary enumeration before partitioning.
pub struct TileSource<H>(pub H);

pub trait TileRows {
    type Row: Send;
    fn tile_rows(
        self,
        grain: usize,
    ) -> impl IndexedParallelIterator<Item = impl Iterator<Item = Self::Row> + Send>;
}

impl<'a, T: Sync> TileRows for &&TileSource<&'a [T]>
where
    &'a T: cq_storage::rows::Columns,
    <&'a T as cq_storage::rows::Columns>::Row: Send,
{
    type Row = <&'a T as cq_storage::rows::Columns>::Row;
    fn tile_rows(
        self,
        grain: usize,
    ) -> impl IndexedParallelIterator<Item = impl Iterator<Item = Self::Row> + Send> {
        self.0.par_chunks(grain).map(cq_storage::rows::Rows::rows)
    }
}

impl<'a, T: Sync> TileRows for &&TileSource<&'a Vec<T>>
where
    &'a T: cq_storage::rows::Columns,
    <&'a T as cq_storage::rows::Columns>::Row: Send,
{
    type Row = <&'a T as cq_storage::rows::Columns>::Row;
    fn tile_rows(
        self,
        grain: usize,
    ) -> impl IndexedParallelIterator<Item = impl Iterator<Item = Self::Row> + Send> {
        self.0.par_chunks(grain).map(cq_storage::rows::Rows::rows)
    }
}

impl<'a, T: Sync, const N: usize> TileRows for &&TileSource<&'a [T; N]>
where
    &'a T: cq_storage::rows::Columns,
    <&'a T as cq_storage::rows::Columns>::Row: Send,
{
    type Row = <&'a T as cq_storage::rows::Columns>::Row;
    fn tile_rows(
        self,
        grain: usize,
    ) -> impl IndexedParallelIterator<Item = impl Iterator<Item = Self::Row> + Send> {
        self.0.par_chunks(grain).map(cq_storage::rows::Rows::rows)
    }
}

impl<'a, T: Sync> TileRows for &&TileSource<&&'a [T]>
where
    &'a T: cq_storage::rows::Columns,
    <&'a T as cq_storage::rows::Columns>::Row: Send,
{
    type Row = <&'a T as cq_storage::rows::Columns>::Row;
    fn tile_rows(
        self,
        grain: usize,
    ) -> impl IndexedParallelIterator<Item = impl Iterator<Item = Self::Row> + Send> {
        self.0.par_chunks(grain).map(cq_storage::rows::Rows::rows)
    }
}

impl<H: cq_storage::rows::Rows> TileRows for &TileSource<H>
where
    H::Row: Send,
{
    type Row = H::Row;
    fn tile_rows(
        self,
        grain: usize,
    ) -> impl IndexedParallelIterator<Item = impl Iterator<Item = Self::Row> + Send> {
        self.0
            .rows()
            .collect::<Vec<_>>()
            .into_par_iter()
            .chunks(grain)
            .map(Vec::into_iter)
    }
}

/// Execute an existing serial row adapter independently in each checked part.
pub fn map_parts<R: Send, O: Send>(
    parts: Vec<Vec<R>>,
    map: impl Fn(Vec<R>) -> Vec<O> + Sync + Send,
) -> Vec<Vec<O>> {
    cq_storage::profile_select! {
        plain { parts.into_par_iter().map(map).collect() }
        counters {{
            let context = cq_storage::counters::TaskContext::capture();
            parts.into_par_iter().map(|part| context.run(|| map(part))).collect()
        }}
    }
}

/// Route with a borrowing key projection, avoiding owned key copies solely
/// for hashing. The router must return an index below the plan's positive count.
pub fn route_by<R: Send, Chunks, Chunk>(
    chunks: Chunks,
    count: usize,
    router: impl Fn(&R, usize) -> usize + Sync,
) -> Vec<Vec<R>>
where
    Chunks: IntoParallelIterator<Item = Chunk>,
    Chunk: IntoIterator<Item = R> + Send,
{
    assert!(count > 0, "key partition count must be positive");
    let _context = cq_storage::profile_select! {
        plain { () }
        counters { cq_storage::counters::TaskContext::capture() }
    };
    let buckets: Vec<Vec<Vec<R>>> = chunks
        .into_par_iter()
        .map(|chunk| {
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
        })
        .collect();
    let mut owners: Vec<Vec<Vec<R>>> = (0..count)
        .map(|_| Vec::with_capacity(buckets.len()))
        .collect();
    for buckets in buckets {
        for (owner, bucket) in owners.iter_mut().zip(buckets) {
            owner.push(bucket);
        }
    }
    owners
        .into_par_iter()
        .map(|chunks| {
            let mut rows = Vec::with_capacity(chunks.iter().map(Vec::len).sum());
            for chunk in chunks {
                rows.extend(chunk);
            }
            rows
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
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
    fn routing_borrows_owned_keys() {
        struct Row {
            key: String,
        }
        let chunks = vec![
            vec![Row {
                key: "alpha".into(),
            }],
            vec![],
            vec![
                Row { key: "beta".into() },
                Row {
                    key: "alpha".into(),
                },
            ],
        ];
        let parts = route_by(chunks, 7, |row, n| {
            cq_storage::sharded::route_index(&(&row.key,), n)
        });
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
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    for rows in [
                        vec![],
                        vec![7u64; 1000],
                        (0..20000).map(|i| i % 1301).collect(),
                    ] {
                        let mut chunks: Vec<_> = rows.chunks(113).map(<[_]>::to_vec).collect();
                        chunks.push(vec![]);
                        for count in [1, 7, 32] {
                            let routed = route_by(chunks.clone(), count, |row, count| {
                                cq_storage::sharded::route_index(row, count)
                            });
                            assert_eq!(routed.len(), count);
                            let actual: Vec<_> = map_parts(routed, |part| {
                                part.into_iter()
                                    .collect::<HashSet<_>>()
                                    .into_iter()
                                    .collect()
                            })
                            .into_iter()
                            .flatten()
                            .collect();
                            let expected: HashSet<_> = rows.iter().copied().collect();
                            assert_eq!(actual.len(), expected.len());
                            assert_eq!(actual.into_iter().collect::<HashSet<_>>(), expected);
                            for groups in [1, 997] {
                                let mut expected = HashMap::new();
                                for &r in &rows {
                                    *expected.entry(r % groups).or_insert(0) += r;
                                }
                                let routed = route_by(chunks.clone(), count, |row, count| {
                                    cq_storage::sharded::route_index(&(row % groups), count)
                                });
                                let actual: Vec<_> = map_parts(routed, |part| {
                                    let mut totals = HashMap::new();
                                    for row in part {
                                        *totals.entry(row % groups).or_insert(0) += row;
                                    }
                                    totals.into_iter().collect()
                                })
                                .into_iter()
                                .flatten()
                                .collect();
                                assert_eq!(actual.len(), expected.len());
                                assert_eq!(actual.into_iter().collect::<HashMap<_, _>>(), expected);
                            }
                        }
                    }
                });
        }
    }
}
