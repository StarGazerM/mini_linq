//! An independent physical layout: one flat payload, indirect directory entries,
//! sparse typed coordinates, and a sparse selection. Access operations exist
//! only as trait methods. Deliberately misleading inherent constructor names
//! panic, proving generated code calls the declared construction traits.
use std::cell::Cell;
use std::collections::{BTreeSet, HashSet};
use std::num::NonZeroUsize;
use std::sync::atomic::{AtomicUsize, Ordering};

pub use cq_storage::{
    BlockDescriptor, BlockRead, BlockSelection, BuildBlocks, CoordinateBounds, Counters,
    ExactMembership, MakeSelection, RangeSummary,
};

pub type BlockStore<const N: usize> = FlatBlocks<N>;
pub type BlockMask = SparseSelection;
pub type ZoneSummary<const K: usize> = KeyEnvelope<K>;

#[derive(Clone, Copy, Debug)]
pub struct SegmentId(u32);

#[derive(Clone, Copy, Debug)]
pub struct RecordId(u32);

struct Segment<const N: usize> {
    id: SegmentId,
    start: usize,
    count: usize,
    bounds: [(i64, i64); N],
}

pub struct FlatBlocks<const N: usize> {
    serial: usize,
    directory: Vec<Segment<N>>,
    records: Vec<[i64; N]>,
    opened: Cell<usize>,
}

static NEXT_SERIAL: AtomicUsize = AtomicUsize::new(1);
thread_local! {
    static PACKED: Cell<usize> = const { Cell::new(0) };
}

pub fn materialization_count() -> usize {
    PACKED.with(Cell::get)
}

impl<const N: usize> FlatBlocks<N> {
    #[allow(dead_code)]
    pub fn pack_unique(_: HashSet<[i64; N]>, _: NonZeroUsize) -> Self {
        panic!("inherent pack_unique must not replace BuildBlocks::pack_unique")
    }

    pub fn from_records(records: impl IntoIterator<Item = [i64; N]>, size: NonZeroUsize) -> Self {
        let mut distinct = HashSet::new();
        let records: Vec<_> = records
            .into_iter()
            .filter(|row| distinct.insert(*row))
            .collect();
        let mut directory: Vec<_> = records
            .chunks(size.get())
            .enumerate()
            .map(|(slot, chunk)| {
                let mut bounds = chunk[0].map(|v| (v, v));
                for row in chunk {
                    for (bound, &value) in bounds.iter_mut().zip(row) {
                        bound.0 = bound.0.min(value);
                        bound.1 = bound.1.max(value);
                    }
                }
                Segment {
                    id: SegmentId(101 + 13 * slot as u32),
                    start: slot * size.get(),
                    count: chunk.len(),
                    bounds,
                }
            })
            .collect();
        // Directory order deliberately differs from both ID order and payload
        // order. Neither the provider nor the mask can use an ID as a Vec index.
        directory.reverse();
        Self {
            serial: NEXT_SERIAL.fetch_add(1, Ordering::Relaxed),
            directory,
            records,
            opened: Cell::new(0),
        }
    }

    pub fn payload_opens(&self) -> usize {
        self.opened.get()
    }
}

fn row_id(offset: usize) -> RecordId {
    RecordId(17 + 3 * offset as u32)
}

impl<const N: usize> BlockRead for FlatBlocks<N> {
    type Row = [i64; N];
    type BlockId = SegmentId;
    type RowId = RecordId;
    type Identity = usize;
    type Descriptor<'a> = DirectoryEntry<'a, N>;

    fn identity(&self) -> Self::Identity {
        self.serial
    }

    fn blocks(&self) -> impl Iterator<Item = Self::Descriptor<'_>> {
        (0..self.directory.len()).map(|slot| DirectoryEntry { source: self, slot })
    }
}

impl<const N: usize> BuildBlocks for FlatBlocks<N> {
    fn pack_unique(rows: impl IntoIterator<Item = Self::Row>, size: NonZeroUsize) -> Self {
        PACKED.with(|count| count.set(count.get() + 1));
        Self::from_records(rows, size)
    }
}

#[derive(Clone, Copy)]
pub struct DirectoryEntry<'a, const N: usize> {
    source: &'a FlatBlocks<N>,
    slot: usize,
}

impl<'a, const N: usize> BlockDescriptor<'a> for DirectoryEntry<'a, N> {
    type Store = FlatBlocks<N>;

    fn store(&self) -> &'a Self::Store {
        self.source
    }

    fn id(&self) -> SegmentId {
        self.source.directory[self.slot].id
    }

    fn rows(self) -> impl Iterator<Item = (RecordId, &'a [i64; N])> {
        self.source.opened.set(self.source.opened.get() + 1);
        let entry = &self.source.directory[self.slot];
        self.source.records[entry.start..entry.start + entry.count]
            .iter()
            .enumerate()
            .map(|(offset, record)| (row_id(offset), record))
    }
}

impl<const N: usize> CoordinateBounds for DirectoryEntry<'_, N> {
    fn bounds(&self) -> &[(i64, i64)] {
        &self.source.directory[self.slot].bounds
    }
}

// Coordinate types intentionally implement neither equality nor hashing. Only
// this private selection implementation knows their numerical encoding.
pub struct SparseSelection(HashSet<(u32, u32)>);

impl BlockSelection for SparseSelection {
    type BlockId = SegmentId;
    type RowId = RecordId;

    fn contains(&self, block: SegmentId, row: RecordId) -> bool {
        self.0.contains(&(block.0, row.0))
    }

    fn insert(&mut self, block: SegmentId, row: RecordId) -> bool {
        self.0.insert((block.0, row.0))
    }

    fn block_nonempty(&self, block: SegmentId) -> bool {
        self.0.iter().any(|&(candidate, _)| candidate == block.0)
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<const N: usize> MakeSelection<FlatBlocks<N>> for SparseSelection {
    fn all(store: &FlatBlocks<N>) -> Self {
        Self(
            store
                .directory
                .iter()
                .flat_map(|entry| {
                    (0..entry.count).map(move |offset| (entry.id.0, row_id(offset).0))
                })
                .collect(),
        )
    }

    fn empty(_: &FlatBlocks<N>) -> Self {
        Self(HashSet::new())
    }
}

// This implementation stores distinct keys instead of coordinate-wise extrema.
// It obeys the same conservative test, with fewer false positives for composite
// keys. The unchanged zone recipe consumes it solely through RangeSummary.
pub struct KeyEnvelope<const K: usize>(BTreeSet<[i64; K]>);

impl<const K: usize> KeyEnvelope<K> {
    #[allow(dead_code)]
    pub fn default() -> Self {
        panic!("inherent default must not replace Default::default")
    }
}

impl<const K: usize> Default for KeyEnvelope<K> {
    fn default() -> Self {
        Self(BTreeSet::new())
    }
}

impl<const K: usize> RangeSummary<K> for KeyEnvelope<K> {
    fn observe(&mut self, key: &[i64; K]) {
        self.0.insert(*key);
    }

    fn may_match(&self, bounds: &[(i64, i64)], columns: &[usize]) -> bool {
        assert_eq!(columns.len(), K);
        assert!(columns.iter().all(|&column| column < bounds.len()));
        self.0.iter().any(|key| {
            columns.iter().zip(key).all(|(&column, &value)| {
                let (lo, hi) = bounds[column];
                lo <= value && value <= hi
            })
        })
    }
}

pub struct OrderedKeys<T>(Vec<T>);

impl<T: Ord> FromIterator<T> for OrderedKeys<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        let mut keys: Vec<_> = iter.into_iter().collect();
        keys.sort_unstable();
        keys.dedup();
        Self(keys)
    }
}

impl<T: Ord> ExactMembership<T> for OrderedKeys<T> {
    fn contains(&self, key: &T) -> bool {
        self.0.binary_search(key).is_ok()
    }
}
