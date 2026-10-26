//! Realizations of checked key partitions. Geometry is supplied by the plan,
//! never by the executing pool. Canon requires full-valuation routing; keyed
//! grouping requires group-key routing and canonical complete valuations.
use cq_storage::parallel::map_index;
use std::sync::Mutex;

/// Indexed coarse work. Each index is consumed exactly once by a dispatch;
/// owned tiles may reject a second access to the same index.
pub trait Tiles: Sync {
    type Tile: Send;
    fn len(&self) -> usize;
    fn tile(&self, index: usize) -> Self::Tile;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A tile count and its accessor, without a parallel iterator type.
pub struct IndexedTiles<F> {
    count: usize,
    access: F,
}

impl<F> IndexedTiles<F> {
    pub fn new<T>(count: usize, access: F) -> Self
    where
        F: Fn(usize) -> T,
    {
        Self { count, access }
    }
}

impl<T: Send, F: Fn(usize) -> T + Sync> Tiles for IndexedTiles<F> {
    type Tile = T;
    fn len(&self) -> usize {
        self.count
    }
    fn tile(&self, index: usize) -> T {
        (self.access)(index)
    }
}

/// Transfer each owned part to its task without requiring its rows to be Sync.
pub fn owned_tiles<T: Send>(parts: Vec<T>) -> impl Tiles<Tile = T> {
    let slots: Vec<_> = parts
        .into_iter()
        .map(|part| Mutex::new(Some(part)))
        .collect();
    IndexedTiles::new(slots.len(), move |index: usize| {
        slots[index]
            .lock()
            .expect("tile slot poisoned")
            .take()
            .expect("tile consumed once")
    })
}

/// Enumerate an opaque source into owned contiguous tiles.
pub fn owned_row_tiles<R: Send>(
    rows: impl IntoIterator<Item = R>,
    grain: usize,
) -> impl Tiles<Tile = std::vec::IntoIter<R>> {
    assert!(grain > 0, "tile grain must be positive");
    let mut rows = rows.into_iter();
    let mut parts = Vec::new();
    loop {
        let part: Vec<_> = rows.by_ref().take(grain).collect();
        if part.is_empty() {
            break;
        }
        parts.push(part.into_iter());
    }
    owned_tiles(parts)
}

fn group_count(tiles: usize) -> usize {
    tiles.min(rayon::current_num_threads().saturating_mul(4))
}

fn group_range(tiles: usize, groups: usize, group: usize) -> std::ops::Range<usize> {
    let width = tiles / groups;
    let extra = tiles % groups;
    let start = group * width + group.min(extra);
    start..start + width + usize::from(group < extra)
}

/// Dispatch contiguous hosts to borrowed tiles; non-contiguous row interfaces
/// retain their ordinary enumeration before partitioning.
pub struct TileSource<H>(pub H);

pub trait TileRows {
    type Row: Send;
    fn tile_rows(self, grain: usize) -> impl Tiles<Tile: Iterator<Item = Self::Row>>;
}

impl<'a, T: Sync> TileRows for &&TileSource<&'a [T]>
where
    &'a T: cq_storage::rows::Columns,
    <&'a T as cq_storage::rows::Columns>::Row: Send,
{
    type Row = <&'a T as cq_storage::rows::Columns>::Row;
    fn tile_rows(self, grain: usize) -> impl Tiles<Tile: Iterator<Item = Self::Row>> {
        let rows = self.0;
        IndexedTiles::new(rows.len().div_ceil(grain), move |index| {
            cq_storage::rows::Rows::rows(&rows[index * grain..rows.len().min((index + 1) * grain)])
        })
    }
}

impl<'a, T: Sync> TileRows for &&TileSource<&'a Vec<T>>
where
    &'a T: cq_storage::rows::Columns,
    <&'a T as cq_storage::rows::Columns>::Row: Send,
{
    type Row = <&'a T as cq_storage::rows::Columns>::Row;
    fn tile_rows(self, grain: usize) -> impl Tiles<Tile: Iterator<Item = Self::Row>> {
        let rows = self.0;
        IndexedTiles::new(rows.len().div_ceil(grain), move |index| {
            cq_storage::rows::Rows::rows(&rows[index * grain..rows.len().min((index + 1) * grain)])
        })
    }
}

impl<'a, T: Sync, const N: usize> TileRows for &&TileSource<&'a [T; N]>
where
    &'a T: cq_storage::rows::Columns,
    <&'a T as cq_storage::rows::Columns>::Row: Send,
{
    type Row = <&'a T as cq_storage::rows::Columns>::Row;
    fn tile_rows(self, grain: usize) -> impl Tiles<Tile: Iterator<Item = Self::Row>> {
        let rows = self.0;
        IndexedTiles::new(rows.len().div_ceil(grain), move |index| {
            cq_storage::rows::Rows::rows(&rows[index * grain..rows.len().min((index + 1) * grain)])
        })
    }
}

impl<'a, T: Sync> TileRows for &&TileSource<&&'a [T]>
where
    &'a T: cq_storage::rows::Columns,
    <&'a T as cq_storage::rows::Columns>::Row: Send,
{
    type Row = <&'a T as cq_storage::rows::Columns>::Row;
    fn tile_rows(self, grain: usize) -> impl Tiles<Tile: Iterator<Item = Self::Row>> {
        let rows = *self.0;
        IndexedTiles::new(rows.len().div_ceil(grain), move |index| {
            cq_storage::rows::Rows::rows(&rows[index * grain..rows.len().min((index + 1) * grain)])
        })
    }
}

impl<H: cq_storage::rows::Rows> TileRows for &TileSource<H>
where
    H::Row: Send,
{
    type Row = H::Row;
    fn tile_rows(self, grain: usize) -> impl Tiles<Tile: Iterator<Item = Self::Row>> {
        owned_row_tiles(self.0.rows(), grain)
    }
}

/// Fold contiguous groups of checked tiles into ordered buffers.
///
/// The number of buffers is bounded by four times the executing worker count.
/// Only the coarse index body enters the shared Rayon dispatch; the tile body
/// is called once per tile, never once per row.
pub fn fold_tiles<P: Tiles, T: Send>(
    parts: P,
    body: &(dyn Fn(&mut Vec<T>, P::Tile) + Sync),
) -> Vec<Vec<T>> {
    let groups = group_count(parts.len());
    map_index(groups, &|group| {
        let mut buffer = Vec::new();
        for index in group_range(parts.len(), groups, group) {
            body(&mut buffer, parts.tile(index));
        }
        buffer
    })
}

/// Evaluate each checked tile into its own buffer, preserving tile order.
pub fn map_tiles<P: Tiles, T: Send>(
    parts: P,
    body: &(dyn Fn(P::Tile) -> Vec<T> + Sync),
) -> Vec<Vec<T>> {
    map_index(parts.len(), &|index| body(parts.tile(index)))
}

/// Execute an existing serial row adapter independently in each checked part.
/// The adapter is type-erased; it runs once per part, outside the row loop.
pub fn map_parts<R: Send, O: Send>(
    parts: Vec<Vec<R>>,
    map: &(dyn Fn(Vec<R>) -> Vec<O> + Sync),
) -> Vec<Vec<O>> {
    let parts = owned_tiles(parts);
    cq_storage::profile_select! {
        plain { map_index(parts.len(), &|index| map(parts.tile(index))) }
        counters {{
            let context = cq_storage::counters::TaskContext::capture();
            map_index(parts.len(), &|index| context.run(|| map(parts.tile(index))))
        }}
    }
}

/// Route with a borrowing key projection, avoiding owned key copies solely
/// for hashing. The router must return an index below the plan's positive count.
///
/// Each contiguous group accumulates chunks into shared owner buffers rather
/// than allocating a set of buffers per chunk. Ordered collection preserves
/// per-owner chunk and row encounter order. Scheduling never changes the
/// supplied partition geometry.
pub fn route_by<R: Send, Chunks, Chunk>(
    chunks: Chunks,
    count: usize,
    router: impl Fn(&R, usize) -> usize + Sync,
) -> Vec<Vec<R>>
where
    Chunks: Tiles<Tile = Chunk>,
    Chunk: IntoIterator<Item = R> + Send,
{
    assert!(count > 0, "key partition count must be positive");
    let _context = cq_storage::profile_select! {
        plain { () }
        counters { cq_storage::counters::TaskContext::capture() }
    };
    let groups = group_count(chunks.len());
    let buckets: Vec<Vec<Vec<R>>> = map_index(groups, &|group| {
        let route = || {
            let mut buckets: Vec<Vec<R>> = (0..count).map(|_| Vec::new()).collect();
            for index in group_range(chunks.len(), groups, group) {
                for row in chunks.tile(index) {
                    buckets[router(&row, count)].push(row);
                }
            }
            buckets
        };
        cq_storage::profile_select! {
            plain { route() }
            counters { _context.run(route) }
        }
    });
    let mut owners: Vec<Vec<Vec<R>>> = (0..count)
        .map(|_| Vec::with_capacity(buckets.len()))
        .collect();
    for buckets in buckets {
        for (owner, bucket) in owners.iter_mut().zip(buckets) {
            if !bucket.is_empty() {
                owner.push(bucket);
            }
        }
    }
    let owners = owned_tiles(owners);
    map_index(owners.len(), &|index| {
        // Reuse the first nonempty buffer; a single group needs no row copy.
        let mut chunks = owners.tile(index).into_iter();
        let mut rows = chunks.next().unwrap_or_default();
        rows.reserve_exact(chunks.as_slice().iter().map(Vec::len).sum());
        for mut chunk in chunks {
            rows.append(&mut chunk);
        }
        rows
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    #[test]
    fn folds_bound_buffers_and_preserve_tile_order() {
        for workers in [1, 2, 7] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| {
                    for count in [0, 1, 3, 29, 1001] {
                        let parts = IndexedTiles::new(count, |index| index);
                        let buffers = fold_tiles(parts, &|buffer, index| {
                            buffer.extend([index * 2, index * 2 + 1]);
                        });
                        assert_eq!(buffers.len(), count.min(workers * 4));
                        assert_eq!(
                            buffers.into_iter().flatten().collect::<Vec<_>>(),
                            (0..count * 2).collect::<Vec<_>>(),
                        );
                    }
                });
        }
    }

    #[test]
    fn owned_tiles_transfer_non_sync_rows_in_order() {
        let parts = owned_row_tiles((0..17).map(std::cell::Cell::new), 3);
        let rows = map_tiles(parts, &|rows| rows.map(|row| row.get()).collect());
        assert_eq!(
            rows.iter().map(Vec::len).collect::<Vec<_>>(),
            [3, 3, 3, 3, 3, 2]
        );
        assert_eq!(
            rows.into_iter().flatten().collect::<Vec<_>>(),
            (0..17).collect::<Vec<_>>()
        );
    }

    #[test]
    fn opaque_hosts_keep_enumeration_order() {
        #[derive(Clone, Copy)]
        struct Opaque<'a>(&'a [u32]);
        impl cq_storage::rows::Rows for Opaque<'_> {
            type Row = u32;
            fn rows(self) -> impl Iterator<Item = u32> {
                self.0.iter().copied()
            }
        }
        let source = TileSource(Opaque(&[3, 1, 4, 1, 5]));
        let parts = map_tiles((&&source).tile_rows(2), &Iterator::collect);
        assert_eq!(parts, [vec![3, 1], vec![4, 1], vec![5]]);
    }

    #[test]
    fn contiguous_tiles_borrow_the_original_columns() {
        let input = [(1u32, 2u32), (3, 4), (5, 6)];
        let source = TileSource(input.as_slice());
        let parts = map_tiles((&&source).tile_rows(2), &Iterator::collect);
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
        let mut expected = vec![Vec::new(); 7];
        for row in chunks.iter().flatten() {
            let owner = cq_storage::sharded::route_index(&(&row.key,), 7);
            expected[owner].push(row.key.as_ptr());
        }
        let parts = route_by(owned_tiles(chunks), 7, |row, n| {
            cq_storage::sharded::route_index(&(&row.key,), n)
        });
        assert_eq!(parts.iter().map(Vec::len).sum::<usize>(), 3);
        for (part, rows) in parts.iter().enumerate() {
            // The non-Clone rows keep the original owned key allocations and
            // their occurrence order, including repeated equal keys.
            assert_eq!(
                rows.iter().map(|row| row.key.as_ptr()).collect::<Vec<_>>(),
                expected[part]
            );
            for row in rows {
                assert_eq!(part, cq_storage::sharded::route_index(&(&row.key,), 7));
            }
        }
    }
    #[test]
    fn routing_preserves_ordered_occurrences() {
        for threads in [1, 2, 7] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    let mixed: Vec<Vec<_>> = (0..257usize)
                        .map(|chunk| {
                            let len = if chunk == 127 { 1024 } else { chunk % 13 };
                            (0..len)
                                .map(|row| ((chunk + row) % 11, chunk * 1024 + row))
                                .collect()
                        })
                        .collect();
                    let skewed: Vec<Vec<_>> = mixed
                        .iter()
                        .map(|chunk| {
                            chunk
                                .iter()
                                .map(|&(_, occurrence)| (7, occurrence))
                                .collect()
                        })
                        .collect();
                    for chunks in [
                        vec![],
                        vec![vec![]; 17],
                        vec![vec![(7, 0); 113]; 23],
                        mixed,
                        skewed,
                    ] {
                        for count in [1, 7, 32] {
                            let mut expected = vec![Vec::new(); count];
                            for &row in chunks.iter().flatten() {
                                let owner = cq_storage::sharded::route_index(&(row.0,), count);
                                expected[owner].push(row);
                            }
                            let routed = route_by(owned_tiles(chunks.clone()), count, |row, n| {
                                cq_storage::sharded::route_index(&(row.0,), n)
                            });
                            assert_eq!(routed, expected);

                            // Filtering before indexing retains source encounter
                            // order and every surviving occurrence.
                            let routed = route_by(
                                owned_tiles(
                                    chunks
                                        .clone()
                                        .into_iter()
                                        .filter(|chunk| !chunk.is_empty())
                                        .collect(),
                                ),
                                count,
                                |row, n| cq_storage::sharded::route_index(&(row.0,), n),
                            );
                            assert_eq!(routed, expected);
                        }
                    }
                });
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
                            let routed =
                                route_by(owned_tiles(chunks.clone()), count, |row, count| {
                                    cq_storage::sharded::route_index(row, count)
                                });
                            assert_eq!(routed.len(), count);
                            let actual: Vec<_> = map_parts(routed, &|part| {
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
                                let routed =
                                    route_by(owned_tiles(chunks.clone()), count, |row, count| {
                                        cq_storage::sharded::route_index(&(row % groups), count)
                                    });
                                let actual: Vec<_> = map_parts(routed, &|part| {
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
