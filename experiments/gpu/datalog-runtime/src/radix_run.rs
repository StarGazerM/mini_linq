//! Occurrence-preserving grouped device runs indexed by an exact radix-prefix
//! directory: the device counterpart of `cq_storage::RadixRun`.
//!
//! A run stores each distinct packed key once, group offsets in the selected
//! carrier `O`, and the selected item columns `C` in stable occurrence order.
//! Keys are [`Word`]s: the DL-DevicePacking word is the one order-preserving
//! key embedding and is stored whole, never truncated into the directory.
//! Every policy is a caller parameter ([`RadixRunParams`]) or a type fact
//! (`W`, `O`, and `C`). Recorded block widths are checked against the kernels'
//! compiled launch contracts before construction.
//! Grid sizes follow the shared occupancy convention for the target device.
//!
//! Key reads ([`RadixKeysView`]) carry no item pointers; item columns are read
//! by position through the separate [`RadixItemsView`], or one demanded column
//! at a time through a [`RadixLayout`]'s [`ItemColumn`].
//!
//! Construction (build, merge, snapshot upload) runs on the run's own pool
//! stream. Reads name an execution pool that must share the snapshot's memory
//! (GPU-RadixLaneRead); shared memory is not a happens-before proof, so the
//! caller orders the read after publication (fork readiness) and consumers of
//! the results after the read (join readiness), and retains the snapshot owner
//! until every stream user has completed.
use crate::{
    layout::{PackedField, ScalarTuple},
    runs::{self, Native},
    Column, Packing, Result, Word,
};
use cuda_core::{DeviceCopy, LaunchConfig1D};
use gpu_device_columns::{Buffer, Pool, SourceView};
use rccl_cub::{DeviceArithmeticRange, DeviceMap, DeviceSliceRead, DeviceSliceWrite};
use std::{
    any::Any,
    marker::PhantomData,
    ops::RangeInclusive,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

const SIDE: u64 = 1 << 63;

mod sealed {
    pub trait Offset {}
    pub trait Lane {}
    pub trait Columns {}
}

/// A group-offset carrier, selected upstream from item-count bound evidence.
/// Builds and merges whose terminal item count exceeds `MAX` are rejected.
pub trait OffsetWord:
    sealed::Offset
    + Copy
    + Ord
    + std::fmt::Debug
    + DeviceCopy
    + rccl_cub::DeviceArithmetic
    + Send
    + Sync
    + 'static
{
    const MAX: u64;
    /// Device-side conversion; host callers check [`offset_fits`] first.
    fn from_u64(value: u64) -> Self;
    fn to_u64(self) -> u64;
}
impl sealed::Offset for u32 {}
impl OffsetWord for u32 {
    const MAX: u64 = u32::MAX as u64;
    #[inline(always)]
    fn from_u64(value: u64) -> Self {
        value as u32
    }
    #[inline(always)]
    fn to_u64(self) -> u64 {
        self as u64
    }
}
impl sealed::Offset for u64 {}
impl OffsetWord for u64 {
    const MAX: u64 = u64::MAX;
    #[inline(always)]
    fn from_u64(value: u64) -> Self {
        value
    }
    #[inline(always)]
    fn to_u64(self) -> u64 {
        self
    }
}

/// Whether carrier `O` represents the terminal offset of `items` occurrences.
pub fn offset_fits<O: OffsetWord>(items: usize) -> bool {
    u64::try_from(items).is_ok_and(|items| items <= O::MAX)
}

/// Stored width and signedness of one item lane: a type fact of [`ItemLane`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LaneKind {
    pub bits: u32,
    pub signed: bool,
}

impl LaneKind {
    /// Whether every value of `column`'s numeric domain is stored exactly:
    /// same signedness and at least the column's width, or an unsigned
    /// column in a strictly wider signed lane. Never a narrowing.
    pub const fn holds(self, column: Column) -> bool {
        if self.signed == column.is_signed() {
            self.bits >= column.bits()
        } else {
            self.signed && self.bits > column.bits()
        }
    }
}

/// A fixed-width, byte-addressable item column element.
pub trait ItemLane: sealed::Lane + Copy + DeviceCopy + Send + Sync + 'static {
    const KIND: LaneKind;
}
macro_rules! lanes {
    ($($ty:ty => $signed:expr),*) => {$(
        impl sealed::Lane for $ty {}
        impl ItemLane for $ty {
            const KIND: LaneKind = LaneKind { bits: <$ty>::BITS, signed: $signed };
        }
    )*};
}
lanes!(u8 => false, i8 => true, u16 => false, i16 => true, u32 => false, i32 => true,
    u64 => false, i64 => true, u128 => false);
impl<const L: usize> sealed::Lane for [u64; L] {}
impl<const L: usize> ItemLane for [u64; L] {
    const KIND: LaneKind = LaneKind {
        bits: 64 * L as u32,
        signed: false,
    };
}

/// Formal rule GPU-RadixItemColumns:
/// An explicit item layout: one `Buffer<T>` column, a tuple of layouts stored as
/// independent columns, or `()` for key-only runs. Position `i` names the same
/// occurrence in every column; grouping and merging gather whole columns by
/// position and never reconstruct rows. The layout is selected by the provider
/// type, never by inspecting a row type.
pub trait DeviceItemColumns: sealed::Columns + Sized + Send + Sync + 'static {
    /// Downloaded diagnostic columns.
    type Host;
    const COLUMNS: usize;
    /// Every column shares `pool`'s memory and has at least `rows` entries.
    /// Shared memory is an allocation fact; the caller orders initialization.
    fn check(&self, pool: &Arc<Pool>, rows: usize) -> Result<()>;
    /// Append each column's lane kind in layout order.
    fn lanes(out: &mut Vec<LaneKind>);
    #[doc(hidden)]
    fn gather(&self, kernels: &ItemKernels, order: *const u32, rows: usize) -> Result<Self>;
    #[doc(hidden)]
    fn merge<O: OffsetWord>(
        left: &Self,
        right: &Self,
        kernels: &ItemKernels,
        left_items: usize,
        sources: *const O,
        rows: usize,
    ) -> Result<Self>;
    /// Append each column's device address in layout order.
    fn column_pointers(&self, out: &mut Vec<u64>);
    fn to_host(&self, rows: usize) -> Result<Self::Host>;
}

/// Loaded column-gather kernels; constructed only by [`DeviceRadixRun`].
pub struct ItemKernels {
    pool: Arc<Pool>,
    module: radix_kernels::LoadedModule,
    block_threads: u32,
}

impl ItemKernels {
    fn config(&self, function: &cuda_core::CudaFunction, parts: usize) -> Result<LaunchConfig1D> {
        let grid = cq_gpu_runtime::launch_grid_for_kernel(
            function,
            parts.div_ceil(self.block_threads as usize),
            self.block_threads,
            0,
        )?;
        Ok(LaunchConfig1D::new(grid, self.block_threads, 0))
    }
    fn shape(&self) -> LaunchConfig1D {
        LaunchConfig1D::new(1, self.block_threads, 0)
    }
}

impl<T: ItemLane> sealed::Columns for Buffer<T> {}
impl<T: ItemLane> DeviceItemColumns for Buffer<T> {
    type Host = Vec<T>;
    const COLUMNS: usize = 1;
    fn check(&self, pool: &Arc<Pool>, rows: usize) -> Result<()> {
        if !pool.shares_memory(self.pool()) || rows > self.len() {
            return Err("item column memory or live length differs from its keys".into());
        }
        Ok(())
    }
    fn lanes(out: &mut Vec<LaneKind>) {
        out.push(T::KIND);
    }
    fn gather(&self, kernels: &ItemKernels, order: *const u32, rows: usize) -> Result<Self> {
        // SAFETY: the gather initializes every output position before publication.
        let output = unsafe { Buffer::<T>::uninitialized(kernels.pool.clone(), rows)? };
        if rows != 0 {
            let module = &kernels.module;
            let initial = module.prepare_gather_items::<T>(kernels.shape())?;
            let config = kernels.config(initial.function(), rows)?;
            module.gather_items::<T>(
                kernels.pool.stream(),
                &module.prepare_gather_items::<T>(config)?,
                self.ptr(),
                order,
                rows as u64,
                output.ptr(),
            )?;
        }
        Ok(output)
    }
    fn merge<O: OffsetWord>(
        left: &Self,
        right: &Self,
        kernels: &ItemKernels,
        left_items: usize,
        sources: *const O,
        rows: usize,
    ) -> Result<Self> {
        // SAFETY: every output position has exactly one source occurrence.
        let output = unsafe { Buffer::<T>::uninitialized(kernels.pool.clone(), rows)? };
        if rows != 0 {
            let module = &kernels.module;
            let initial = module.prepare_merge_items::<T, O>(kernels.shape())?;
            let config = kernels.config(initial.function(), rows)?;
            module.merge_items::<T, O>(
                kernels.pool.stream(),
                &module.prepare_merge_items::<T, O>(config)?,
                left.ptr(),
                right.ptr(),
                left_items as u64,
                sources,
                rows as u64,
                output.ptr(),
            )?;
        }
        Ok(output)
    }
    fn column_pointers(&self, out: &mut Vec<u64>) {
        out.push(self.cu_deviceptr());
    }
    fn to_host(&self, rows: usize) -> Result<Vec<T>> {
        self.to_host_prefix(rows)
    }
}

impl sealed::Columns for () {}
impl DeviceItemColumns for () {
    type Host = ();
    const COLUMNS: usize = 0;
    fn check(&self, _: &Arc<Pool>, _: usize) -> Result<()> {
        Ok(())
    }
    fn lanes(_: &mut Vec<LaneKind>) {}
    fn gather(&self, _: &ItemKernels, _: *const u32, _: usize) -> Result<()> {
        Ok(())
    }
    fn merge<O: OffsetWord>(
        _: &(),
        _: &(),
        _: &ItemKernels,
        _: usize,
        _: *const O,
        _: usize,
    ) -> Result<()> {
        Ok(())
    }
    fn column_pointers(&self, _: &mut Vec<u64>) {}
    fn to_host(&self, _: usize) -> Result<()> {
        Ok(())
    }
}

macro_rules! tuple_columns {
    ($($C:ident:$field:tt),+) => {
        impl<$($C: DeviceItemColumns),+> sealed::Columns for ($($C,)+) {}
        impl<$($C: DeviceItemColumns),+> DeviceItemColumns for ($($C,)+) {
            type Host = ($($C::Host,)+);
            const COLUMNS: usize = 0 $(+ $C::COLUMNS)+;
            fn check(&self, pool: &Arc<Pool>, rows: usize) -> Result<()> {
                $(self.$field.check(pool, rows)?;)+
                Ok(())
            }
            fn lanes(out: &mut Vec<LaneKind>) {
                $($C::lanes(out);)+
            }
            fn gather(&self, kernels: &ItemKernels, order: *const u32, rows: usize) -> Result<Self> {
                Ok(($(self.$field.gather(kernels, order, rows)?,)+))
            }
            fn merge<O: OffsetWord>(
                left: &Self, right: &Self, kernels: &ItemKernels, left_items: usize,
                sources: *const O, rows: usize,
            ) -> Result<Self> {
                Ok(($($C::merge(&left.$field, &right.$field, kernels, left_items, sources, rows)?,)+))
            }
            fn column_pointers(&self, out: &mut Vec<u64>) {
                $(self.$field.column_pointers(out);)+
            }
            fn to_host(&self, rows: usize) -> Result<Self::Host> {
                Ok(($(self.$field.to_host(rows)?,)+))
            }
        }
    };
}
tuple_columns!(A:0);
tuple_columns!(A:0, B:1);
tuple_columns!(A:0, B:1, C:2);
tuple_columns!(A:0, B:1, C:2, D:3);
tuple_columns!(A:0, B:1, C:2, D:3, E:4);
tuple_columns!(A:0, B:1, C:2, D:3, E:4, F:5);
tuple_columns!(A:0, B:1, C:2, D:3, E:4, F:5, G:6);
tuple_columns!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7);
tuple_columns!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8);
tuple_columns!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9);
tuple_columns!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10);
tuple_columns!(A:0, B:1, C:2, D:3, E:4, F:5, G:6, H:7, I:8, J:9, K:10, L:11);

/// An integer item lane filled from a decoded resident source value (its
/// i64 bit pattern). The recorded value schema guarantees the source column's
/// domain fits the lane ([`LaneKind::holds`]), so the conversion is exact.
pub trait SourceLane: ItemLane {
    fn from_source(value: i64) -> Self;
}
macro_rules! source_lanes {
    ($($ty:ty),*) => {$(
        impl SourceLane for $ty {
            #[inline(always)]
            fn from_source(value: i64) -> Self {
                value as $ty
            }
        }
    )*};
}
source_lanes!(u8, i8, u16, i16, u32, i32, u64, i64);

/// Item layouts that a build fills from resident source columns, one source
/// per lane in layout order.
pub trait SourceItemColumns: DeviceItemColumns {
    #[doc(hidden)]
    fn from_sources(
        kernels: &ItemKernels,
        sources: &[SourceView],
        positions: *const u32,
        rows: usize,
    ) -> Result<Self>;
}

impl SourceItemColumns for () {
    fn from_sources(_: &ItemKernels, _: &[SourceView], _: *const u32, _: usize) -> Result<()> {
        Ok(())
    }
}

impl<T: SourceLane> SourceItemColumns for Buffer<T> {
    fn from_sources(
        kernels: &ItemKernels,
        sources: &[SourceView],
        positions: *const u32,
        rows: usize,
    ) -> Result<Self> {
        let [source] = sources else {
            return Err("one source column per item lane".into());
        };
        // SAFETY: the lane kernel initializes every position before publication.
        let output = unsafe { Buffer::<T>::uninitialized(kernels.pool.clone(), rows)? };
        if rows != 0 {
            let module = &kernels.module;
            let initial = module.prepare_source_lane::<T>(kernels.shape())?;
            let config = kernels.config(initial.function(), rows)?;
            module.source_lane::<T>(
                kernels.pool.stream(),
                &module.prepare_source_lane::<T>(config)?,
                *source,
                positions,
                rows as u64,
                output.ptr(),
            )?;
        }
        Ok(output)
    }
}

macro_rules! source_columns {
    ($($C:ident),+) => {
        impl<$($C: SourceItemColumns),+> SourceItemColumns for ($($C,)+) {
            fn from_sources(
                kernels: &ItemKernels,
                sources: &[SourceView],
                positions: *const u32,
                rows: usize,
            ) -> Result<Self> {
                if sources.len() != Self::COLUMNS {
                    return Err("one source column per item lane".into());
                }
                let mut _at = 0;
                Ok(($({
                    let part = &sources[_at.._at + $C::COLUMNS];
                    _at += $C::COLUMNS;
                    $C::from_sources(kernels, part, positions, rows)?
                },)+))
            }
        }
    };
}
source_columns!(A);
source_columns!(A, B);
source_columns!(A, B, C);
source_columns!(A, B, C, D);
source_columns!(A, B, C, D, E);
source_columns!(A, B, C, D, E, F);
source_columns!(A, B, C, D, E, F, G);
source_columns!(A, B, C, D, E, F, G, H);
source_columns!(A, B, C, D, E, F, G, H, I);
source_columns!(A, B, C, D, E, F, G, H, I, J);
source_columns!(A, B, C, D, E, F, G, H, I, J, K);
source_columns!(A, B, C, D, E, F, G, H, I, J, K, L);

/// Caller-recorded provider parameters; the runtime chooses none of them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RadixRunParams {
    /// Geometric item-count ratio between adjacent published runs (`>= 2`).
    pub ratio: usize,
    /// Directory resolution: `2^radix_bits + 1` lower bounds of min-relative
    /// key prefixes per run. Zero selects exact binary search with no directory.
    pub radix_bits: u32,
    /// Merged key slots per merge-path tile (`>= 1`).
    pub merge_tile_rows: u32,
    /// Threads per block for radix auxiliary kernels, including stable ordering.
    /// Must equal the kernels' compiled 256-thread launch contract.
    pub block_threads: u32,
}

impl RadixRunParams {
    pub fn validate(&self) -> Result<()> {
        if self.ratio < 2 {
            return Err("radix run ratio must be at least two".into());
        }
        if self.radix_bits > 32 {
            return Err("radix directory bits must be at most 32".into());
        }
        if self.merge_tile_rows == 0 {
            return Err("merge tile must contain at least one slot".into());
        }
        if self.block_threads != 256 {
            return Err("radix kernels require exactly 256 block threads".into());
        }
        Ok(())
    }
}

/// One immutable run, as a borrowed kernel ABI. Contains no item pointers.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RadixRunView<W: Word, O: OffsetWord> {
    /// `keys_len` strictly increasing distinct keys.
    pub keys: *const W,
    /// `keys_len + 1` group offsets; the last is `items`.
    pub offsets: *const O,
    /// `2^radix_bits + 1` slot lower bounds, or null when `radix_bits == 0`.
    pub directory: *const O,
    pub keys_len: u64,
    pub items: u64,
    pub radix_bits: u32,
    pub reserved: u32,
}
// SAFETY: this C-layout descriptor contains only device pointers and integers.
unsafe impl<W: Word, O: OffsetWord> DeviceCopy for RadixRunView<W, O> {}
// SAFETY: the pointers name immutable device storage and are never
// dereferenced on the host, so moving or sharing descriptors is sound.
unsafe impl<W: Word, O: OffsetWord> Send for RadixRunView<W, O> {}
unsafe impl<W: Word, O: OffsetWord> Sync for RadixRunView<W, O> {}

/// All published runs of one snapshot, oldest first. Reads visit every run.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RadixKeysView<W: Word, O: OffsetWord> {
    pub runs: *const RadixRunView<W, O>,
    pub run_count: u32,
    pub reserved: u32,
    /// Total occurrences across all runs; the exact absent-key position.
    pub items: u64,
    /// The snapshot identity every [`RadixMatch`] read through this view carries.
    pub snapshot: u64,
}
// SAFETY: this C-layout descriptor contains only device pointers and integers.
unsafe impl<W: Word, O: OffsetWord> DeviceCopy for RadixKeysView<W, O> {}

/// Item column addresses, run-major: `columns[run * column_count + column]`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RadixItemsView {
    pub columns: *const u64,
    pub column_count: u32,
    pub run_count: u32,
    pub snapshot: u64,
}
// SAFETY: this C-layout descriptor contains only a device pointer and integers.
unsafe impl DeviceCopy for RadixItemsView {}

/// One demanded item column of a snapshot (narrow item access): reads touch
/// no other column. `T` of the typed [`ItemColumn`] is the column's lane.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RadixItemColumn {
    /// The snapshot's run-major column address table.
    pub columns: *const u64,
    pub snapshot: u64,
    pub column: u32,
    pub column_count: u32,
    pub run_count: u32,
    pub reserved: u32,
}
// SAFETY: this C-layout descriptor contains only a device pointer and integers.
unsafe impl DeviceCopy for RadixItemColumn {}
// SAFETY: the column table is immutable device storage, never dereferenced on the host.
unsafe impl Send for RadixItemColumn {}
unsafe impl Sync for RadixItemColumn {}

/// Batch key-read result: membership, occurrence count and the exact range in
/// the concatenated stable order of all runs (`(items, items)` when absent).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyProbe {
    pub contains: u64,
    pub count: u64,
    pub begin: u64,
    pub end: u64,
}
// SAFETY: plain integers.
unsafe impl DeviceCopy for KeyProbe {}

/// Formal rule GPU-RadixCursor:
/// A match cursor names one run's group: the snapshot identity, the run index
/// and the half-open item positions `[begin, end)` local to that run's item
/// columns (`(items, items)` of the run when absent). Logical merged-order
/// positions ([`KeyProbe`]) are counts, never item addresses; expansion reads
/// each run's cursor through a column of the same snapshot.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RadixMatch {
    pub snapshot: u64,
    pub run: u32,
    pub reserved: u32,
    pub begin: u64,
    pub end: u64,
}
// SAFETY: plain integers.
unsafe impl DeviceCopy for RadixMatch {}

impl RadixMatch {
    pub fn len(&self) -> u64 {
        self.end - self.begin
    }
    pub fn is_empty(&self) -> bool {
        self.begin == self.end
    }
}

/// Inclusive key-interval read summed over runs: group slots and occurrences.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct IntervalProbe {
    pub groups: u64,
    pub items: u64,
}
// SAFETY: plain integers.
unsafe impl DeviceCopy for IntervalProbe {}

struct Segment<W: Word, O: OffsetWord, C> {
    keys: Buffer<W>,
    offsets: Buffer<O>,
    directory: Buffer<O>,
    items: C,
    key_count: usize,
    item_count: usize,
    radix_bits: u32,
}

impl<W: Word, O: OffsetWord, C> Segment<W, O, C> {
    fn view(&self) -> RadixRunView<W, O> {
        RadixRunView {
            keys: self.keys.ptr(),
            offsets: self.offsets.ptr(),
            directory: if self.radix_bits == 0 {
                std::ptr::null()
            } else {
                self.directory.ptr()
            },
            keys_len: self.key_count as u64,
            items: self.item_count as u64,
            radix_bits: self.radix_bits,
            reserved: 0,
        }
    }
}

/// Downloaded diagnostic copy of one run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostRadixRun<W, O, H> {
    pub keys: Vec<W>,
    pub offsets: Vec<O>,
    pub directory: Vec<O>,
    pub items: H,
}

/// Formal rule GPU-RadixRun:
/// A stable lane radix permutation groups packed-key occurrences into distinct
/// keys, `O` offsets and gathered item columns; an exact radix-prefix directory
/// bounds a lower-bound search, so membership, counts, ranges and absent-key
/// insertion positions equal the CPU RadixRun's for the same occurrences.
/// Auxiliary launch widths are caller-recorded in `RadixRunParams`, checked
/// against compiled contracts before construction, and retained by snapshots.
///
/// Each nonempty build downloads one group count; key reads never touch items.
pub struct DeviceRadixRun<W: Word, O: OffsetWord, C: DeviceItemColumns> {
    pool: Arc<Pool>,
    params: RadixRunParams,
    runs: Vec<Arc<Segment<W, O, C>>>,
    /// The current publication's descriptors, uploaded once on first read and
    /// replaced only when a publication changes the runs.
    published: Mutex<Option<Arc<RadixSnapshot<W, O, C>>>>,
    kernels: ItemKernels,
    order: runs::run_kernels::LoadedModule,
}

impl<W: Word, O: OffsetWord, C: DeviceItemColumns> DeviceRadixRun<W, O, C> {
    pub fn new(pool: Arc<Pool>, params: RadixRunParams) -> Result<Self> {
        params.validate()?;
        // SAFETY: modules are loaded in, and retained with, the pool's context.
        let module = unsafe { radix_kernels::load(pool.context())? };
        let order = unsafe { runs::run_kernels::load(pool.context())? };
        Ok(Self {
            kernels: ItemKernels {
                pool: pool.clone(),
                module,
                block_threads: params.block_threads,
            },
            pool,
            params,
            runs: Vec::new(),
            published: Mutex::new(None),
            order,
        })
    }

    /// Group `rows` occurrences: the first `rows` keys with the same positions
    /// of every item column. Equal keys keep their input occurrence order.
    pub fn build(
        pool: Arc<Pool>,
        params: RadixRunParams,
        keys: &Buffer<W>,
        items: &C,
        rows: usize,
    ) -> Result<Self> {
        let mut run = Self::new(pool, params)?;
        run.publish(keys, items, rows)?;
        Ok(run)
    }

    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }
    pub fn params(&self) -> RadixRunParams {
        self.params
    }
    pub fn run_count(&self) -> usize {
        self.runs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }
    /// Total occurrences across all runs.
    pub fn item_count(&self) -> usize {
        self.runs.iter().map(|run| run.item_count).sum()
    }
    /// `(distinct keys, occurrences)` per run, oldest first.
    pub fn run_lengths(&self) -> Vec<(usize, usize)> {
        self.runs
            .iter()
            .map(|run| (run.key_count, run.item_count))
            .collect()
    }

    /// Formal rule GPU-RadixPublish:
    /// Publication groups only the delta, appends its run, and merges adjacent
    /// newest runs by stable merge-path until every older run has at least
    /// `ratio` times the next run's occurrences: the CPU RadixRun ratio law.
    /// Reads of every snapshot include all unmerged runs. Empty deltas are no-ops;
    /// a failed build or merge leaves the published runs unchanged.
    pub fn publish(&mut self, keys: &Buffer<W>, items: &C, rows: usize) -> Result<()> {
        let Some(segment) = self.group(keys, items, rows)? else {
            return Ok(());
        };
        let mut merged = Arc::new(segment);
        let mut keep = self.runs.len();
        while keep > 0 && self.runs[keep - 1].item_count / self.params.ratio < merged.item_count {
            merged = Arc::new(self.merge(&self.runs[keep - 1], &merged)?);
            keep -= 1;
        }
        self.runs.truncate(keep);
        self.runs.push(merged);
        *self.published.get_mut().expect("radix publication cache") = None;
        Ok(())
    }

    /// The current publication's immutable snapshot. Descriptors, the item
    /// column table and the resident count are uploaded once per publication
    /// on [`Self::pool`]'s stream without a host synchronization; until the
    /// runs change, every call returns the same snapshot (same identity).
    /// A snapshot retains its runs across later publications and release.
    pub fn snapshot(&self) -> Result<Arc<RadixSnapshot<W, O, C>>> {
        let mut published = self.published.lock().expect("radix publication cache");
        if let Some(snapshot) = published.as_ref() {
            return Ok(snapshot.clone());
        }
        let views = self.runs.iter().map(|run| run.view()).collect::<Vec<_>>();
        u32::try_from(views.len()).map_err(|_| "radix run count exceeds u32")?;
        let mut columns = Vec::with_capacity(views.len() * C::COLUMNS);
        for run in &self.runs {
            run.items.column_pointers(&mut columns);
        }
        let items = self.item_count() as u64;
        let snapshot = Arc::new(RadixSnapshot {
            id: NEXT_SNAPSHOT.fetch_add(1, Ordering::Relaxed),
            descriptors: Buffer::from_host_async(self.pool.clone(), &views)?,
            columns: Buffer::from_host_async(self.pool.clone(), &columns)?,
            count: Buffer::from_host_async(self.pool.clone(), &[items])?,
            items,
            runs: self.runs.clone(),
            module: self.kernels.module.clone(),
            pool: self.pool.clone(),
            block_threads: self.params.block_threads,
        });
        *published = Some(snapshot.clone());
        Ok(snapshot)
    }

    /// Explicit diagnostic download of every run, oldest first.
    pub fn to_host(&self) -> Result<Vec<HostRadixRun<W, O, C::Host>>> {
        self.runs
            .iter()
            .map(|run| {
                Ok(HostRadixRun {
                    keys: run.keys.to_host_prefix(run.key_count)?,
                    offsets: run.offsets.to_host_prefix(run.key_count + 1)?,
                    directory: if run.radix_bits == 0 {
                        Vec::new()
                    } else {
                        run.directory.to_host()?
                    },
                    items: run.items.to_host(run.item_count)?,
                })
            })
            .collect()
    }

    fn launch_config(
        &self,
        function: &cuda_core::CudaFunction,
        parts: usize,
    ) -> Result<LaunchConfig1D> {
        self.kernels.config(function, parts)
    }

    fn group(&self, keys: &Buffer<W>, items: &C, rows: usize) -> Result<Option<Segment<W, O, C>>> {
        if !self.pool.shares_memory(keys.pool()) || rows > keys.len() {
            return Err("radix run keys use another pool's memory or exceed their capacity".into());
        }
        items.check(&self.pool, rows)?;
        if rows > u32::MAX as usize {
            return Err("radix run build exceeds the u32 row domain".into());
        }
        if !offset_fits::<O>(rows) {
            return Err("radix run items exceed the offset carrier".into());
        }
        if rows == 0 {
            return Ok(None);
        }
        let pool = &self.pool;
        let module = &self.kernels.module;
        let cub = rccl_cub::cuda::Program::load(pool.context())?;
        let launch = cub.on(pool.stream())?;
        let permutation = runs::stable_order(
            pool,
            &self.order,
            &launch,
            keys,
            rows,
            self.params.block_threads,
        )?;
        let order = permutation.get().cu_deviceptr() as *const u32;
        // SAFETY: the flag kernel initializes all rows + 1 entries.
        let flags = unsafe { Buffer::<u32>::uninitialized(pool.clone(), rows + 1)? };
        let initial = module.prepare_group_flags::<W>(self.kernels.shape())?;
        let config = self.launch_config(initial.function(), rows + 1)?;
        module.group_flags::<W>(
            pool.stream(),
            &module.prepare_group_flags::<W>(config)?,
            keys.ptr(),
            order,
            rows as u64,
            flags.ptr(),
        )?;
        // SAFETY: stable selections initialize the prefixes read below.
        let offsets = unsafe { Buffer::<O>::uninitialized(pool.clone(), rows + 1)? };
        let distinct = unsafe { Buffer::<W>::uninitialized(pool.clone(), rows)? };
        let mut offset_count = Native::<u64>::new(pool, 1)?;
        let mut key_count = Native::<u64>::new(pool, 1)?;
        let mut workspace = rccl_cub::thrust_integration::compaction_workspace_in(
            pool.resource(),
            &launch,
            rows + 1,
        )?;
        rccl_cub::thrust_integration::device_select_flagged(
            &launch,
            DeviceArithmeticRange::new(O::from_u64(0), O::from_u64(1)),
            DeviceSliceRead::new(flags.ptr() as *const u32),
            DeviceSliceWrite::new(offsets.ptr()),
            offset_count.get_mut(),
            rows + 1,
            &mut workspace,
        )?;
        let source = keys.ptr() as *const W;
        let sorted = DeviceMap::new(DeviceSliceRead::new(order), move |row: u32| unsafe {
            *source.add(row as usize)
        });
        let mut workspace =
            rccl_cub::thrust_integration::compaction_workspace_in(pool.resource(), &launch, rows)?;
        rccl_cub::thrust_integration::device_select_flagged(
            &launch,
            sorted,
            DeviceSliceRead::new(flags.ptr() as *const u32),
            DeviceSliceWrite::new(distinct.ptr()),
            key_count.get_mut(),
            rows,
            &mut workspace,
        )?;
        let items = items.gather(&self.kernels, order, rows)?;
        let key_count = runs::read_count(pool, offset_count.get())? - 1;
        let directory = self.directory(&distinct, key_count)?;
        Ok(Some(Segment {
            keys: distinct,
            offsets,
            directory,
            items,
            key_count,
            item_count: rows,
            radix_bits: self.params.radix_bits,
        }))
    }

    fn merge(&self, left: &Segment<W, O, C>, right: &Segment<W, O, C>) -> Result<Segment<W, O, C>> {
        let total = left
            .item_count
            .checked_add(right.item_count)
            .ok_or("merged item count overflow")?;
        if !offset_fits::<O>(total) {
            return Err("merged radix run items exceed the offset carrier".into());
        }
        let slots = left.key_count + right.key_count;
        let pool = &self.pool;
        let module = &self.kernels.module;
        // SAFETY: the merge-path kernel initializes all slots and the terminal entry.
        let tags = unsafe { Buffer::<u64>::uninitialized(pool.clone(), slots)? };
        let positions = unsafe { Buffer::<O>::uninitialized(pool.clone(), slots + 1)? };
        let flags = unsafe { Buffer::<u32>::uninitialized(pool.clone(), slots + 1)? };
        let tile = self.params.merge_tile_rows as usize;
        let initial = module.prepare_merge_slots::<W, O>(self.kernels.shape())?;
        let config = self.launch_config(initial.function(), slots.div_ceil(tile))?;
        module.merge_slots::<W, O>(
            pool.stream(),
            &module.prepare_merge_slots::<W, O>(config)?,
            left.view(),
            right.view(),
            tile as u64,
            tags.ptr(),
            positions.ptr(),
            flags.ptr(),
        )?;
        let cub = rccl_cub::cuda::Program::load(pool.context())?;
        let launch = cub.on(pool.stream())?;
        // SAFETY: stable selections initialize the prefixes read below.
        let offsets = unsafe { Buffer::<O>::uninitialized(pool.clone(), slots + 1)? };
        let distinct = unsafe { Buffer::<W>::uninitialized(pool.clone(), slots)? };
        let mut offset_count = Native::<u64>::new(pool, 1)?;
        let mut key_count = Native::<u64>::new(pool, 1)?;
        let mut workspace = rccl_cub::thrust_integration::compaction_workspace_in(
            pool.resource(),
            &launch,
            slots + 1,
        )?;
        rccl_cub::thrust_integration::device_select_flagged(
            &launch,
            DeviceSliceRead::new(positions.ptr() as *const O),
            DeviceSliceRead::new(flags.ptr() as *const u32),
            DeviceSliceWrite::new(offsets.ptr()),
            offset_count.get_mut(),
            slots + 1,
            &mut workspace,
        )?;
        let (left_keys, right_keys) = (left.keys.ptr() as *const W, right.keys.ptr() as *const W);
        let merged = DeviceMap::new(
            DeviceSliceRead::new(tags.ptr() as *const u64),
            move |tag: u64| unsafe {
                if tag & SIDE == 0 {
                    *left_keys.add(tag as usize)
                } else {
                    *right_keys.add((tag & !SIDE) as usize)
                }
            },
        );
        let mut workspace =
            rccl_cub::thrust_integration::compaction_workspace_in(pool.resource(), &launch, slots)?;
        rccl_cub::thrust_integration::device_select_flagged(
            &launch,
            merged,
            DeviceSliceRead::new(flags.ptr() as *const u32),
            DeviceSliceWrite::new(distinct.ptr()),
            key_count.get_mut(),
            slots,
            &mut workspace,
        )?;
        // SAFETY: each merged item position has exactly one source occurrence.
        let sources = unsafe { Buffer::<O>::uninitialized(pool.clone(), total)? };
        let initial = module.prepare_item_sources::<W, O>(self.kernels.shape())?;
        let config = self.launch_config(initial.function(), total)?;
        module.item_sources::<W, O>(
            pool.stream(),
            &module.prepare_item_sources::<W, O>(config)?,
            left.view(),
            right.view(),
            tags.ptr(),
            positions.ptr(),
            slots as u64,
            total as u64,
            sources.ptr(),
        )?;
        let items = C::merge(
            &left.items,
            &right.items,
            &self.kernels,
            left.item_count,
            sources.ptr(),
            total,
        )?;
        let key_count = runs::read_count(pool, offset_count.get())? - 1;
        let directory = self.directory(&distinct, key_count)?;
        Ok(Segment {
            keys: distinct,
            offsets,
            directory,
            items,
            key_count,
            item_count: total,
            radix_bits: self.params.radix_bits,
        })
    }

    fn directory(&self, keys: &Buffer<W>, key_count: usize) -> Result<Buffer<O>> {
        let bits = self.params.radix_bits;
        if bits == 0 || key_count == 0 {
            // SAFETY: zero-length storage is never read.
            return unsafe { Buffer::uninitialized(self.pool.clone(), 0) };
        }
        let entries = (1_usize << bits) + 1;
        // SAFETY: one thread initializes each directory entry.
        let directory = unsafe { Buffer::<O>::uninitialized(self.pool.clone(), entries)? };
        let module = &self.kernels.module;
        let initial = module.prepare_build_directory::<W, O>(self.kernels.shape())?;
        let config = self.launch_config(initial.function(), entries)?;
        module.build_directory::<W, O>(
            self.pool.stream(),
            &module.prepare_build_directory::<W, O>(config)?,
            keys.ptr(),
            key_count as u64,
            bits,
            directory.ptr(),
        )?;
        Ok(directory)
    }
}

static NEXT_SNAPSHOT: AtomicU64 = AtomicU64::new(1);

/// Immutable descriptors over one publication's runs, retained independently
/// of later publications and of the [`DeviceRadixRun`] itself. Its identity is
/// unique per process; reads launch on a caller-named execution pool.
pub struct RadixSnapshot<W: Word, O: OffsetWord, C> {
    id: u64,
    descriptors: Buffer<RadixRunView<W, O>>,
    columns: Buffer<u64>,
    /// One-entry resident logical occurrence count for count-gated kernels.
    count: Buffer<u64>,
    items: u64,
    runs: Vec<Arc<Segment<W, O, C>>>,
    module: radix_kernels::LoadedModule,
    pool: Arc<Pool>,
    block_threads: u32,
}

/// Formal rule GPU-RadixLaneRead:
/// Reads of an immutable radix snapshot launch on an execution pool that
/// shares the snapshot's memory: the publication pool or one of its lanes.
/// Shared memory is no happens-before proof: a lane reads only after fork
/// readiness on the publication stream, its results are consumed after join
/// readiness, and every retained owner outlives those stream users.
fn check_execution(publication: &Pool, execution: &Pool) -> Result<()> {
    if !execution.shares_memory(publication) {
        return Err("radix reads must run on a pool sharing the snapshot's memory".into());
    }
    Ok(())
}

impl<W: Word, O: OffsetWord, C: DeviceItemColumns> RadixSnapshot<W, O, C> {
    /// The publication pool: the stream that uploaded these descriptors.
    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn run_count(&self) -> usize {
        self.runs.len()
    }
    pub fn item_count(&self) -> usize {
        self.items as usize
    }
    /// Resident logical occurrence count (one `u64`), readable by count-gated
    /// device consumers after publication readiness, without a host download.
    pub fn count_ptr(&self) -> *const u64 {
        self.count.ptr()
    }
    /// Descriptor, column-table and count bytes uploaded for this snapshot.
    pub fn descriptor_bytes(&self) -> usize {
        self.descriptors.allocated_bytes()
            + self.columns.allocated_bytes()
            + self.count.allocated_bytes()
    }
    /// Key-read ABI: keys, offsets and directories only.
    pub fn keys_view(&self) -> RadixKeysView<W, O> {
        RadixKeysView {
            runs: self.descriptors.ptr(),
            run_count: self.runs.len() as u32,
            reserved: 0,
            items: self.items,
            snapshot: self.id,
        }
    }
    /// Item-read ABI, separate from every key read.
    pub fn items_view(&self) -> RadixItemsView {
        RadixItemsView {
            columns: self.columns.ptr(),
            column_count: C::COLUMNS as u32,
            run_count: self.runs.len() as u32,
            snapshot: self.id,
        }
    }

    fn shape(&self) -> LaunchConfig1D {
        LaunchConfig1D::new(1, self.block_threads, 0)
    }

    fn config(&self, function: &cuda_core::CudaFunction, parts: usize) -> Result<LaunchConfig1D> {
        let grid = cq_gpu_runtime::launch_grid_for_kernel(
            function,
            parts.div_ceil(self.block_threads as usize),
            self.block_threads,
            0,
        )?;
        Ok(LaunchConfig1D::new(grid, self.block_threads, 0))
    }

    fn check_probes(&self, pool: &Pool, probes: &Buffer<W>, count: usize) -> Result<()> {
        check_execution(&self.pool, pool)?;
        if !pool.shares_memory(probes.pool()) || count > probes.len() {
            return Err("probe keys use another pool's memory or exceed their capacity".into());
        }
        Ok(())
    }

    /// Membership, count and logical merged-order range for each probe key,
    /// on `pool`'s stream. Range positions are counts, not item addresses.
    pub fn probe_keys(
        &self,
        pool: &Arc<Pool>,
        probes: &Buffer<W>,
        count: usize,
    ) -> Result<Buffer<KeyProbe>> {
        self.check_probes(pool, probes, count)?;
        // SAFETY: one thread initializes each result.
        let output = unsafe { Buffer::<KeyProbe>::uninitialized(pool.clone(), count)? };
        if count != 0 {
            let initial = self.module.prepare_probe_keys::<W, O>(self.shape())?;
            let config = self.config(initial.function(), count)?;
            self.module.probe_keys::<W, O>(
                pool.stream(),
                &self.module.prepare_probe_keys::<W, O>(config)?,
                self.keys_view(),
                probes.ptr(),
                count as u64,
                output.ptr(),
            )?;
        }
        Ok(output)
    }

    /// Per-run match cursors for each probe, `[probe * run_count + run]`, on
    /// `pool`'s stream. The result retains this snapshot as its witness.
    pub fn probe_matches(
        self: &Arc<Self>,
        pool: &Arc<Pool>,
        probes: &Buffer<W>,
        count: usize,
    ) -> Result<RadixMatches<W, O, C>> {
        self.check_probes(pool, probes, count)?;
        let len = count
            .checked_mul(self.runs.len())
            .ok_or("probe match count overflow")?;
        // SAFETY: one thread initializes every run cursor of its probe.
        let matches = unsafe { Buffer::<RadixMatch>::uninitialized(pool.clone(), len)? };
        if len != 0 {
            let initial = self.module.prepare_probe_matches::<W, O>(self.shape())?;
            let config = self.config(initial.function(), count)?;
            self.module.probe_matches::<W, O>(
                pool.stream(),
                &self.module.prepare_probe_matches::<W, O>(config)?,
                self.keys_view(),
                probes.ptr(),
                count as u64,
                matches.ptr(),
            )?;
        }
        Ok(RadixMatches {
            matches,
            probes: count,
            snapshot: self.clone(),
        })
    }

    /// Groups and occurrences with keys in each inclusive interval `[low, high]`.
    pub fn probe_intervals(
        &self,
        pool: &Arc<Pool>,
        low: &Buffer<W>,
        high: &Buffer<W>,
        count: usize,
    ) -> Result<Buffer<IntervalProbe>> {
        self.check_probes(pool, low, count)?;
        self.check_probes(pool, high, count)?;
        // SAFETY: one thread initializes each result.
        let output = unsafe { Buffer::<IntervalProbe>::uninitialized(pool.clone(), count)? };
        if count != 0 {
            let initial = self.module.prepare_probe_intervals::<W, O>(self.shape())?;
            let config = self.config(initial.function(), count)?;
            self.module.probe_intervals::<W, O>(
                pool.stream(),
                &self.module.prepare_probe_intervals::<W, O>(config)?,
                self.keys_view(),
                low.ptr(),
                high.ptr(),
                count as u64,
                output.ptr(),
            )?;
        }
        Ok(output)
    }
}

/// A retained match set: per-run cursors of a batch of probes and the
/// snapshot they address. It survives later publications and release of the
/// run and layout, and is consumed only where its owner forces it.
pub struct RadixMatches<W: Word, O: OffsetWord, C> {
    matches: Buffer<RadixMatch>,
    probes: usize,
    snapshot: Arc<RadixSnapshot<W, O, C>>,
}

impl<W: Word, O: OffsetWord, C: DeviceItemColumns> RadixMatches<W, O, C> {
    pub fn snapshot_id(&self) -> u64 {
        self.snapshot.id
    }
    pub fn probe_count(&self) -> usize {
        self.probes
    }
    pub fn run_count(&self) -> usize {
        self.snapshot.runs.len()
    }
    /// Cursors `[probe * run_count + run]`, on the reading pool's memory.
    pub fn cursors(&self) -> &Buffer<RadixMatch> {
        &self.matches
    }
    /// Explicit diagnostic download of every cursor.
    pub fn to_host(&self) -> Result<Vec<RadixMatch>> {
        if self.matches.is_empty() {
            Ok(Vec::new())
        } else {
            self.matches.to_host()
        }
    }

    /// Expand every cursor's occurrences of one demanded column on `pool`'s
    /// stream: cursor `i` writes its items, in stable run order, from
    /// `bases[i]` of a `total`-entry output. Writes beyond `total` are dropped.
    /// The column must belong to this match set's snapshot.
    pub fn gather<T: ItemLane>(
        &self,
        pool: &Arc<Pool>,
        column: &ItemColumn<T>,
        bases: &Buffer<u64>,
        total: usize,
    ) -> Result<Buffer<T>> {
        check_execution(&self.snapshot.pool, pool)?;
        if column.view.snapshot != self.snapshot.id {
            return Err("item column and match cursors belong to different snapshots".into());
        }
        let cursors = self.matches.len();
        if !pool.shares_memory(bases.pool()) || bases.len() < cursors {
            return Err("gather bases use another pool's memory or miss cursors".into());
        }
        // SAFETY: callers supply disjoint bases covering `total`; unwritten
        // positions are unspecified, never read past the output.
        let output = unsafe { Buffer::<T>::uninitialized(pool.clone(), total)? };
        if cursors != 0 && total != 0 {
            let module = &self.snapshot.module;
            let initial = module.prepare_gather_matches::<T>(self.snapshot.shape())?;
            let config = self.snapshot.config(initial.function(), cursors)?;
            module.gather_matches::<T>(
                pool.stream(),
                &module.prepare_gather_matches::<T>(config)?,
                column.view,
                self.matches.ptr(),
                cursors as u64,
                bases.ptr(),
                total as u64,
                output.ptr(),
            )?;
        }
        Ok(output)
    }
}

/// A typed, narrow item-column handle; `T` equals the column's lane type.
/// It borrows the snapshot's addresses: retain the layout or match set owner.
#[derive(Clone, Copy, Debug)]
pub struct ItemColumn<T> {
    pub view: RadixItemColumn,
    marker: PhantomData<fn() -> T>,
}

/// Device field ABI of a [`RadixLayout`]: key reads plus the key packing's
/// field descriptors and the resident count. Contains no item pointers.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RadixLayoutView<W: Word, O: OffsetWord> {
    pub keys: RadixKeysView<W, O>,
    /// Most-significant-first key fields of `K`.
    pub fields: *const PackedField,
    pub field_count: u32,
    /// Significant key bits of the packing (at most `W::BITS`).
    pub bits: u32,
    /// Resident logical occurrence count.
    pub count: *const u64,
}
// SAFETY: this C-layout descriptor contains only device pointers and integers.
unsafe impl<W: Word, O: OffsetWord> DeviceCopy for RadixLayoutView<W, O> {}

struct RadixPublication<W: Word, O: OffsetWord, C> {
    snapshot: Arc<RadixSnapshot<W, O, C>>,
    // Constant per packing: a republished layout shares its predecessor's.
    fields: Arc<Buffer<PackedField>>,
    packing: Packing,
}

/// Formal rule GPU-RadixLayout:
/// A typed relational view over one immutable radix publication: key tuple
/// `K` packed by a checked most-significant-first packing into whole `W`
/// keys, value tuple `V` stored one item column per field in `C` lanes that
/// hold each field's domain, offsets in `O`. It owns the snapshot (runs,
/// descriptors, resident count) and field descriptors; republication over
/// unchanged runs is this layout, otherwise it shares the field descriptors.
/// No host synchronization constructs it; key reads take no item pointers.
pub struct RadixLayout<K: ScalarTuple, V: ScalarTuple, W: Word, O: OffsetWord, C: DeviceItemColumns>
{
    publication: Arc<RadixPublication<W, O, C>>,
    marker: PhantomData<fn() -> (K, V)>,
}

impl<K: ScalarTuple, V: ScalarTuple, W: Word, O: OffsetWord, C: DeviceItemColumns> Clone
    for RadixLayout<K, V, W, O, C>
{
    fn clone(&self) -> Self {
        Self {
            publication: self.publication.clone(),
            marker: PhantomData,
        }
    }
}

impl<K: ScalarTuple, V: ScalarTuple, W: Word + 'static, O: OffsetWord, C: DeviceItemColumns>
    RadixLayout<K, V, W, O, C>
{
    /// The key packing of `K`'s columns in recorded order.
    pub fn derive_packing() -> Result<Packing> {
        Packing::build(
            K::COLUMNS
                .iter()
                .map(|column| (column.bits(), column.is_signed())),
        )
    }

    fn check_schema(packing: &Packing) -> Result<()> {
        if packing.len() != K::COLUMNS.len()
            || !packing
                .packed_fields()
                .zip(K::COLUMNS)
                .all(|(field, column)| {
                    field.bits == column.bits() && (field.signed != 0) == column.is_signed()
                })
        {
            return Err("radix layout packing differs from its key tuple schema".into());
        }
        if packing.bits() > W::BITS {
            return Err("radix layout key packing exceeds the key word".into());
        }
        let mut lanes = Vec::with_capacity(C::COLUMNS);
        C::lanes(&mut lanes);
        if lanes.len() != V::COLUMNS.len()
            || !lanes
                .iter()
                .zip(V::COLUMNS)
                .all(|(lane, &column)| lane.holds(column))
        {
            return Err("radix layout item lanes do not hold its value tuple schema".into());
        }
        Ok(())
    }

    /// Publish `run`'s current snapshot as a layout with key `packing`.
    pub fn publish(run: &DeviceRadixRun<W, O, C>, packing: Packing) -> Result<Self> {
        Self::check_schema(&packing)?;
        let fields = packing.packed_fields().collect::<Vec<_>>();
        Ok(Self {
            publication: Arc::new(RadixPublication {
                snapshot: run.snapshot()?,
                fields: Arc::new(Buffer::from_host_async(run.pool().clone(), &fields)?),
                packing,
            }),
            marker: PhantomData,
        })
    }

    /// This layout's successor over `run`'s current publication. Unchanged
    /// runs (the same cached snapshot) return this layout itself with no
    /// upload; otherwise the successor shares this layout's field descriptors.
    pub fn republish(&self, run: &DeviceRadixRun<W, O, C>) -> Result<Self> {
        check_execution(&self.publication.snapshot.pool, run.pool())?;
        let snapshot = run.snapshot()?;
        if Arc::ptr_eq(&snapshot, &self.publication.snapshot) {
            return Ok(self.clone());
        }
        Ok(Self {
            publication: Arc::new(RadixPublication {
                snapshot,
                fields: self.publication.fields.clone(),
                packing: self.publication.packing.clone(),
            }),
            marker: PhantomData,
        })
    }

    pub fn snapshot(&self) -> &Arc<RadixSnapshot<W, O, C>> {
        &self.publication.snapshot
    }
    pub fn snapshot_id(&self) -> u64 {
        self.publication.snapshot.id
    }
    pub fn pool(&self) -> &Arc<Pool> {
        &self.publication.snapshot.pool
    }
    pub fn packing(&self) -> &Packing {
        &self.publication.packing
    }
    pub fn run_count(&self) -> usize {
        self.publication.snapshot.runs.len()
    }
    /// Occurrences, known on the host from the build's exact counts.
    pub fn len(&self) -> usize {
        self.publication.snapshot.item_count()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Resident logical occurrence count, without a host download.
    pub fn count_ptr(&self) -> *const u64 {
        self.publication.snapshot.count_ptr()
    }
    /// Explicitly synchronize and download the resident count.
    pub fn logical_count(&self) -> Result<usize> {
        Ok(usize::try_from(
            self.publication.snapshot.count.to_host()?[0],
        )?)
    }
    /// Field-descriptor buffer address, shared by republished successors.
    pub fn fields_ptr(&self) -> *const PackedField {
        self.publication.fields.ptr()
    }
    /// Retain runs, descriptors and fields through every stream user.
    pub fn owner(&self) -> Arc<dyn Any + Send + Sync> {
        self.publication.clone()
    }

    pub fn view(&self) -> RadixLayoutView<W, O> {
        let publication = &self.publication;
        RadixLayoutView {
            keys: publication.snapshot.keys_view(),
            fields: publication.fields.ptr(),
            field_count: publication.fields.len() as u32,
            bits: publication.packing.bits(),
            count: publication.snapshot.count_ptr(),
        }
    }

    /// Narrow access to value field `column`, whose lane type must be `T`.
    pub fn item_column<T: ItemLane>(&self, column: usize) -> Result<ItemColumn<T>> {
        let mut lanes = Vec::with_capacity(C::COLUMNS);
        C::lanes(&mut lanes);
        match lanes.get(column) {
            Some(&kind) if kind == T::KIND => {}
            Some(_) => {
                return Err("item column lane differs from the requested element type".into())
            }
            None => return Err("item column outside the value tuple".into()),
        }
        let snapshot = &self.publication.snapshot;
        Ok(ItemColumn {
            view: RadixItemColumn {
                columns: snapshot.columns.ptr(),
                snapshot: snapshot.id,
                column: column as u32,
                column_count: C::COLUMNS as u32,
                run_count: snapshot.runs.len() as u32,
                reserved: 0,
            },
            marker: PhantomData,
        })
    }

    /// Inclusive packed-key interval of a key-prefix (all keys when empty).
    pub fn prefix_interval(&self, prefix: &[i128]) -> Result<RangeInclusive<W>> {
        self.publication.packing.prefix_range(prefix)
    }

    pub fn probe_keys(
        &self,
        pool: &Arc<Pool>,
        probes: &Buffer<W>,
        count: usize,
    ) -> Result<Buffer<KeyProbe>> {
        self.publication.snapshot.probe_keys(pool, probes, count)
    }
    pub fn probe_matches(
        &self,
        pool: &Arc<Pool>,
        probes: &Buffer<W>,
        count: usize,
    ) -> Result<RadixMatches<W, O, C>> {
        self.publication.snapshot.probe_matches(pool, probes, count)
    }
    pub fn probe_intervals(
        &self,
        pool: &Arc<Pool>,
        low: &Buffer<W>,
        high: &Buffer<W>,
        count: usize,
    ) -> Result<Buffer<IntervalProbe>> {
        self.publication
            .snapshot
            .probe_intervals(pool, low, high, count)
    }

    /// Every item column's address table (position reads by run).
    pub fn items_view(&self) -> RadixItemsView {
        self.publication.snapshot.items_view()
    }
}

impl<K: ScalarTuple, V: ScalarTuple, W: Word + 'static, O: OffsetWord, C: SourceItemColumns>
    RadixLayout<K, V, W, O, C>
{
    /// Formal rule GPU-RadixSourceBuild: one publication of `rows` occurrences
    /// read from resident source columns (through `positions` unless null):
    /// key fields packed by `K`'s packing into whole `W` keys, one item lane
    /// per value source. Grouping is the ordinary GPU-RadixPublish build; the
    /// only host synchronization is its group-count download.
    ///
    /// # Safety
    /// Every source must describe initialized values of its schema column at
    /// every read row, ready on `pool`'s stream and immutable during the
    /// build; `positions`, unless null, addresses `rows` such device rows.
    pub unsafe fn from_sources(
        pool: Arc<Pool>,
        params: RadixRunParams,
        keys: &[SourceView],
        values: &[SourceView],
        positions: *const u32,
        rows: usize,
    ) -> Result<Self> {
        let packing = Self::derive_packing()?;
        Self::check_schema(&packing)?;
        if keys.len() != K::COLUMNS.len() || values.len() != C::COLUMNS {
            return Err("radix sources differ from the layout's key and value schema".into());
        }
        let mut run = DeviceRadixRun::<W, O, C>::new(pool.clone(), params)?;
        if rows != 0 {
            u32::try_from(rows).map_err(|_| "radix source build exceeds the u32 row domain")?;
            let sources = Buffer::from_host_async(pool.clone(), keys)?;
            let fields = Buffer::from_host_async(
                pool.clone(),
                &packing.packed_fields().collect::<Vec<_>>(),
            )?;
            // SAFETY: the pack kernel initializes every key before grouping.
            let words = unsafe { Buffer::<W>::uninitialized(pool.clone(), rows)? };
            let module = &run.kernels.module;
            let initial = module.prepare_pack_sources::<W>(run.kernels.shape())?;
            let config = run.kernels.config(initial.function(), rows)?;
            module.pack_sources::<W>(
                pool.stream(),
                &module.prepare_pack_sources::<W>(config)?,
                sources.ptr(),
                fields.ptr(),
                keys.len() as u32,
                positions,
                rows as u64,
                words.ptr(),
            )?;
            let items = C::from_sources(&run.kernels, values, positions, rows)?;
            run.publish(&words, &items, rows)?;
        }
        Self::publish(&run, packing)
    }
}

/// Pointer-only reads usable inside generated device kernels.
pub mod device {
    use super::{
        OffsetWord, RadixItemColumn, RadixItemsView, RadixKeysView, RadixLayoutView, RadixMatch,
        RadixRunView,
    };
    use crate::Word;

    /// `a - b` for `a >= b`, lane-wise with borrow.
    #[inline(always)]
    pub fn difference<W: Word>(a: W, b: W) -> W {
        let mut output = W::zero();
        let mut borrow = 0_u64;
        let mut lane = 0;
        while lane < W::BITS / 64 {
            let (partial, first) = a.lane(lane).overflowing_sub(b.lane(lane));
            let (value, second) = partial.overflowing_sub(borrow);
            output.set_lane(lane, value);
            borrow = (first | second) as u64;
            lane += 1;
        }
        output
    }

    /// Number of significant bits, zero for the zero word.
    #[inline(always)]
    pub fn significant_bits<W: Word>(word: W) -> u32 {
        let mut lane = W::BITS / 64;
        while lane != 0 {
            lane -= 1;
            let value = word.lane(lane);
            if value != 0 {
                return lane * 64 + 64 - value.leading_zeros();
            }
        }
        0
    }

    /// The directory bucket of `key` in `[min, max]`: the top `radix_bits`
    /// bits of `key - min` at the run span's resolution. Monotone in `key`.
    #[inline(always)]
    pub fn prefix<W: Word>(key: W, min: W, max: W, radix_bits: u32) -> u64 {
        let shift = significant_bits(difference(max, min)).saturating_sub(radix_bits);
        difference(key, min).extract(shift, radix_bits)
    }

    /// Lower-bound slot of `key` in one run and whether it is present.
    /// # Safety
    /// `run` must describe a live run readable on this device.
    #[inline(always)]
    pub unsafe fn search<W: Word, O: OffsetWord>(run: RadixRunView<W, O>, key: W) -> (u64, bool) {
        let len = run.keys_len;
        if len == 0 {
            return (0, false);
        }
        let min = unsafe { *run.keys };
        if key.compare(&min).is_lt() {
            return (0, false);
        }
        let max = unsafe { *run.keys.add(len as usize - 1) };
        let last = key.compare(&max);
        if last.is_gt() {
            return (len, false);
        }
        if last.is_eq() {
            return (len - 1, true);
        }
        let (mut low, mut high) = if run.radix_bits == 0 {
            (0, len)
        } else {
            let bucket = prefix(key, min, max, run.radix_bits) as usize;
            unsafe {
                (
                    (*run.directory.add(bucket)).to_u64(),
                    (*run.directory.add(bucket + 1)).to_u64(),
                )
            }
        };
        while low < high {
            let middle = low + (high - low) / 2;
            if unsafe { &*run.keys.add(middle as usize) }
                .compare(&key)
                .is_lt()
            {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        (
            low,
            low < len
                && unsafe { &*run.keys.add(low as usize) }
                    .compare(&key)
                    .is_eq(),
        )
    }

    /// Item position of a slot's group start (`slot <= keys_len`).
    /// # Safety
    /// `run` must be live and `slot` at most its key count.
    #[inline(always)]
    pub unsafe fn offset<W: Word, O: OffsetWord>(run: RadixRunView<W, O>, slot: u64) -> u64 {
        unsafe { (*run.offsets.add(slot as usize)).to_u64() }
    }

    /// The run's half-open item group for `key`, empty at `(items, items)` if absent.
    /// # Safety
    /// `run` must describe a live run readable on this device.
    #[inline(always)]
    pub unsafe fn group<W: Word, O: OffsetWord>(run: RadixRunView<W, O>, key: W) -> (u64, u64) {
        let (slot, found) = unsafe { search(run, key) };
        if found {
            unsafe { (offset(run, slot), offset(run, slot + 1)) }
        } else {
            (run.items, run.items)
        }
    }

    /// # Safety
    /// `view` must describe live runs readable on this device.
    #[inline(always)]
    pub unsafe fn run<W: Word, O: OffsetWord>(
        view: RadixKeysView<W, O>,
        run: u32,
    ) -> RadixRunView<W, O> {
        unsafe { *view.runs.add(run as usize) }
    }

    /// # Safety
    /// `view` must describe live runs readable on this device.
    #[inline(always)]
    pub unsafe fn contains<W: Word, O: OffsetWord>(view: RadixKeysView<W, O>, key: W) -> bool {
        let mut index = 0;
        while index < view.run_count {
            if unsafe { search(run(view, index), key) }.1 {
                return true;
            }
            index += 1;
        }
        false
    }

    /// Occurrences of `key` across all runs, from offsets only.
    /// # Safety
    /// `view` must describe live runs readable on this device.
    #[inline(always)]
    pub unsafe fn count<W: Word, O: OffsetWord>(view: RadixKeysView<W, O>, key: W) -> u64 {
        let mut total = 0;
        let mut index = 0;
        while index < view.run_count {
            let (begin, end) = unsafe { group(run(view, index), key) };
            total += end - begin;
            index += 1;
        }
        total
    }

    /// The exact range of `key` in the stable concatenation of all runs'
    /// merged order (the CPU `RadixRun::range`), `(items, items)` if absent.
    /// # Safety
    /// `view` must describe live runs readable on this device.
    #[inline(always)]
    pub unsafe fn range<W: Word, O: OffsetWord>(view: RadixKeysView<W, O>, key: W) -> (u64, u64) {
        let mut begin = 0;
        let mut count = 0;
        let mut index = 0;
        while index < view.run_count {
            let current = unsafe { run(view, index) };
            let (slot, found) = unsafe { search(current, key) };
            let start = unsafe { offset(current, slot) };
            begin += start;
            if found {
                count += unsafe { offset(current, slot + 1) } - start;
            }
            index += 1;
        }
        if count == 0 {
            (view.items, view.items)
        } else {
            (begin, begin + count)
        }
    }

    /// Slot interval `[first, last)` of one run's keys within inclusive `[low, high]`.
    /// # Safety
    /// `run` must describe a live run readable on this device.
    #[inline(always)]
    pub unsafe fn interval<W: Word, O: OffsetWord>(
        run: RadixRunView<W, O>,
        low: W,
        high: W,
    ) -> (u64, u64) {
        let (first, _) = unsafe { search(run, low) };
        let (last, found) = unsafe { search(run, high) };
        let last = if found { last + 1 } else { last };
        (first, if last < first { first } else { last })
    }

    /// Read one item of column `column` at a run position.
    /// # Safety
    /// `T` must be the column's element type and the position live.
    #[inline(always)]
    pub unsafe fn item<T: Copy>(items: RadixItemsView, run: u32, column: u32, position: u64) -> T {
        let address = unsafe {
            *items
                .columns
                .add((run * items.column_count + column) as usize)
        };
        unsafe { *(address as *const T).add(position as usize) }
    }

    /// The match cursor of `key` in run `index`: snapshot identity, run index
    /// and the run-local group `[begin, end)` (`(items, items)` if absent).
    /// # Safety
    /// `view` must describe live runs readable on this device; `index < run_count`.
    #[inline(always)]
    pub unsafe fn cursor<W: Word, O: OffsetWord>(
        view: RadixKeysView<W, O>,
        index: u32,
        key: W,
    ) -> RadixMatch {
        let (begin, end) = unsafe { group(run(view, index), key) };
        RadixMatch {
            snapshot: view.snapshot,
            run: index,
            reserved: 0,
            begin,
            end,
        }
    }

    /// Whether `cursor` addresses the snapshot `column` belongs to.
    #[inline(always)]
    pub fn same_snapshot(column: RadixItemColumn, cursor: RadixMatch) -> bool {
        column.snapshot == cursor.snapshot
    }

    /// Item `offset` of a cursor's group in one demanded column.
    /// # Safety
    /// `T` must be the column's lane, `same_snapshot(column, cursor)` must hold,
    /// the snapshot must be live and `offset < cursor.end - cursor.begin`.
    #[inline(always)]
    pub unsafe fn cursor_item<T: Copy>(
        column: RadixItemColumn,
        cursor: RadixMatch,
        offset: u64,
    ) -> T {
        let address = unsafe {
            *column
                .columns
                .add((cursor.run * column.column_count + column.column) as usize)
        };
        unsafe { *(address as *const T).add((cursor.begin + offset) as usize) }
    }

    /// Formal rule GPU-RadixEncodedProbe: the inclusive key interval of the
    /// leading `count` key fields bound to `values` (each as its i64 bit
    /// pattern), encoded once into whole words: the bound fields' sign-bit
    /// encodings, then all-zero (low) or all-one (high) remaining key bits.
    /// No `high + 1` is formed. `false` when a value lies outside its
    /// field's domain (no key can match).
    /// # Safety
    /// `view.fields` must address `count <= view.field_count` live descriptors
    /// and `values` `count` initialized values readable on this device.
    #[inline(always)]
    pub unsafe fn encode_prefix<W: Word, O: OffsetWord>(
        view: RadixLayoutView<W, O>,
        values: *const i64,
        count: u32,
    ) -> (W, W, bool) {
        let mut low = W::zero();
        let mut remaining = view.bits;
        let mut index = 0;
        while index < count {
            let field = unsafe { *view.fields.add(index as usize) };
            let value = unsafe { *values.add(index as usize) };
            let fits = if field.bits >= 64 {
                true
            } else if field.signed != 0 {
                let half = 1_i64 << (field.bits - 1);
                value >= -half && value < half
            } else {
                value >= 0 && value < (1_i64 << field.bits)
            };
            if !fits {
                return (low, low, false);
            }
            let mask = u64::MAX >> (64 - field.bits);
            let sign = if field.signed != 0 {
                1_u64 << (field.bits - 1)
            } else {
                0
            };
            low.insert(field.shift, field.bits, ((value as u64) & mask) ^ sign);
            remaining = field.shift;
            index += 1;
        }
        let mut high = low;
        let mut lane = 0;
        while lane * 64 < remaining {
            let bits = (remaining - lane * 64).min(64);
            high.set_lane(lane, high.lane(lane) | (u64::MAX >> (64 - bits)));
            lane += 1;
        }
        (low, high, true)
    }

    /// Run-local item interval `[begin, end)` of the keys in inclusive `[low, high]`.
    /// # Safety
    /// `run` must describe a live run readable on this device.
    #[inline(always)]
    pub unsafe fn interval_items<W: Word, O: OffsetWord>(
        run: RadixRunView<W, O>,
        low: W,
        high: W,
    ) -> (u64, u64) {
        let (first, last) = unsafe { interval(run, low, high) };
        unsafe { (offset(run, first), offset(run, last)) }
    }

    /// Whether a run older than `before` holds `key`.
    #[inline(always)]
    unsafe fn older<W: Word, O: OffsetWord>(
        view: RadixKeysView<W, O>,
        before: u32,
        key: W,
    ) -> bool {
        let mut index = 0;
        while index < before {
            if unsafe { search(run(view, index), key) }.1 {
                return true;
            }
            index += 1;
        }
        false
    }

    /// Formal rule GPU-RadixCandidates: the exact number of distinct keys in
    /// inclusive `[low, high]` over the union of all runs. Keys are distinct
    /// within a run, so one run's count is its slot interval; across runs a
    /// key counts once, in its oldest run (never a sum of per-run counts).
    /// # Safety
    /// `view` must describe live runs readable on this device.
    #[inline(always)]
    pub unsafe fn distinct<W: Word, O: OffsetWord>(
        view: RadixKeysView<W, O>,
        low: W,
        high: W,
    ) -> u64 {
        let mut total = 0;
        let mut index = 0;
        while index < view.run_count {
            let current = unsafe { run(view, index) };
            let (first, last) = unsafe { interval(current, low, high) };
            if index == 0 {
                total += last - first;
            } else {
                let mut slot = first;
                while slot < last {
                    let key = unsafe { *current.keys.add(slot as usize) };
                    total += !unsafe { older(view, index, key) } as u64;
                    slot += 1;
                }
            }
            index += 1;
        }
        total
    }

    /// The `ordinal`-th distinct key of [`distinct`]'s enumeration (runs
    /// oldest first, keys ascending within a run, each key in its oldest
    /// run) as `(run, slot)`. Requires `ordinal < distinct(view, low, high)`.
    /// # Safety
    /// `view` must describe live runs readable on this device.
    #[inline(always)]
    pub unsafe fn distinct_at<W: Word, O: OffsetWord>(
        view: RadixKeysView<W, O>,
        low: W,
        high: W,
        mut ordinal: u64,
    ) -> (u32, u64) {
        let mut index = 0;
        while index < view.run_count {
            let current = unsafe { run(view, index) };
            let (first, last) = unsafe { interval(current, low, high) };
            if index == 0 {
                if ordinal < last - first {
                    return (0, first + ordinal);
                }
                ordinal -= last - first;
            } else {
                let mut slot = first;
                while slot < last {
                    let key = unsafe { *current.keys.add(slot as usize) };
                    if !unsafe { older(view, index, key) } {
                        if ordinal == 0 {
                            return (index, slot);
                        }
                        ordinal -= 1;
                    }
                    slot += 1;
                }
            }
            index += 1;
        }
        (view.run_count, 0)
    }
}

#[cuda_host::cuda_module]
mod radix_kernels {
    use crate::{
        layout::PackedField,
        radix_run::{
            device, IntervalProbe, ItemLane, KeyProbe, OffsetWord, RadixItemColumn, RadixKeysView,
            RadixMatch, RadixRunView, SourceLane, SIDE,
        },
        Word,
    };
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    use gpu_device_columns::{decode_source, SourceView};

    /// Pack each read row's key fields (most significant first, sign bit
    /// flipped for signed fields) into one whole key word.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn pack_sources<W: Word>(
        sources: *const SourceView,
        fields: *const PackedField,
        field_count: u32,
        positions: *const u32,
        rows: u64,
        keys: *mut W,
    ) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * thread::blockDim_x() as u64;
        while row < rows {
            let source = if positions.is_null() {
                row
            } else {
                unsafe { *positions.add(row as usize) as u64 }
            };
            let mut key = W::zero();
            let mut index = 0;
            while index < field_count {
                let field = unsafe { *fields.add(index as usize) };
                // DECODE V: radix source build reconstructs resident FOR values before key packing; P4 pack encoded order keys directly.
                let value = unsafe { decode_source(*sources.add(index as usize), source) } as u64;
                let mask = u64::MAX >> (64 - field.bits);
                let sign = if field.signed != 0 {
                    1_u64 << (field.bits - 1)
                } else {
                    0
                };
                key.insert(field.shift, field.bits, (value & mask) ^ sign);
                index += 1;
            }
            unsafe {
                *keys.add(row as usize) = key;
            }
            row += stride;
        }
    }

    /// Fill one item lane with each read row's source value.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn source_lane<T: SourceLane>(
        input: SourceView,
        positions: *const u32,
        rows: u64,
        output: *mut T,
    ) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * thread::blockDim_x() as u64;
        while row < rows {
            let source = if positions.is_null() {
                row
            } else {
                unsafe { *positions.add(row as usize) as u64 }
            };
            unsafe {
                // DECODE V: radix source build reconstructs resident FOR payloads into native item lanes; P4 retain encoded lanes.
                *output.add(row as usize) = T::from_source(decode_source(input, source));
            }
            row += stride;
        }
    }

    /// Mark group starts of the permuted keys; entry `rows` is the terminal.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn group_flags<W: Word>(input: *const W, order: *const u32, rows: u64, flags: *mut u32) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * thread::blockDim_x() as u64;
        while row <= rows {
            let start = row == 0
                || row == rows
                || unsafe {
                    !(*input.add(*order.add(row as usize) as usize))
                        .compare(&*input.add(*order.add(row as usize - 1) as usize))
                        .is_eq()
                };
            unsafe {
                *flags.add(row as usize) = start as u32;
            }
            row += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn gather_items<T: ItemLane>(
        input: *const T,
        order: *const u32,
        rows: u64,
        output: *mut T,
    ) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * thread::blockDim_x() as u64;
        while row < rows {
            unsafe {
                *output.add(row as usize) = *input.add(*order.add(row as usize) as usize);
            }
            row += stride;
        }
    }

    /// Merge two runs' key directories by stable merge-path tiles. Each slot
    /// records its source (`SIDE` marks the newer run), the items preceding it
    /// and whether it starts a new distinct key. Entry `slots` is the terminal.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn merge_slots<W: Word, O: OffsetWord>(
        left: RadixRunView<W, O>,
        right: RadixRunView<W, O>,
        tile_rows: u64,
        tags: *mut u64,
        positions: *mut O,
        flags: *mut u32,
    ) {
        let (left_len, right_len) = (left.keys_len as usize, right.keys_len as usize);
        let slots = left_len + right_len;
        let mut tile = thread::index_1d().get() as usize;
        let stride = thread::gridDim_x() as usize * thread::blockDim_x() as usize;
        if tile == 0 {
            unsafe {
                *positions.add(slots) = O::from_u64(left.items + right.items);
                *flags.add(slots) = 1;
            }
        }
        let tiles = slots.div_ceil(tile_rows as usize);
        while tile < tiles {
            let start = tile * tile_rows as usize;
            let end = start + (slots - start).min(tile_rows as usize);
            let mut a =
                unsafe { crate::runs::corank(left.keys, left_len, right.keys, right_len, start) };
            let mut b = start - a;
            let mut slot = start;
            while slot < end {
                let take_left = a < left_len
                    && (b == right_len
                        || !unsafe { &*left.keys.add(a) }
                            .compare(unsafe { &*right.keys.add(b) })
                            .is_gt());
                let position =
                    unsafe { device::offset(left, a as u64) + device::offset(right, b as u64) };
                // Runs hold distinct keys, so only the other run's predecessor can repeat this key.
                let (tag, repeat) = if take_left {
                    let key = unsafe { &*left.keys.add(a) };
                    (
                        a as u64,
                        b > 0 && unsafe { &*right.keys.add(b - 1) }.compare(key).is_eq(),
                    )
                } else {
                    let key = unsafe { &*right.keys.add(b) };
                    (
                        b as u64 | SIDE,
                        a > 0 && unsafe { &*left.keys.add(a - 1) }.compare(key).is_eq(),
                    )
                };
                unsafe {
                    *tags.add(slot) = tag;
                    *positions.add(slot) = O::from_u64(position);
                    *flags.add(slot) = !repeat as u32;
                }
                if take_left {
                    a += 1;
                } else {
                    b += 1;
                }
                slot += 1;
            }
            tile += stride;
        }
    }

    /// Map each merged item position to its source in `left ++ right` items.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn item_sources<W: Word, O: OffsetWord>(
        left: RadixRunView<W, O>,
        right: RadixRunView<W, O>,
        tags: *const u64,
        positions: *const O,
        slots: u64,
        total: u64,
        sources: *mut O,
    ) {
        let mut item = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * thread::blockDim_x() as u64;
        while item < total {
            // Slot positions strictly increase: every group is nonempty.
            let (mut low, mut high) = (0_u64, slots);
            while low < high {
                let middle = low + (high - low) / 2;
                if unsafe { *positions.add(middle as usize) }.to_u64() <= item {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            let slot = low - 1;
            let tag = unsafe { *tags.add(slot as usize) };
            let base = if tag & SIDE == 0 {
                unsafe { device::offset(left, tag) }
            } else {
                left.items + unsafe { device::offset(right, tag & !SIDE) }
            };
            let start = unsafe { *positions.add(slot as usize) }.to_u64();
            unsafe {
                *sources.add(item as usize) = O::from_u64(base + item - start);
            }
            item += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn merge_items<T: ItemLane, O: OffsetWord>(
        left: *const T,
        right: *const T,
        left_items: u64,
        sources: *const O,
        rows: u64,
        output: *mut T,
    ) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * thread::blockDim_x() as u64;
        while row < rows {
            let source = unsafe { *sources.add(row as usize) }.to_u64();
            unsafe {
                *output.add(row as usize) = if source < left_items {
                    *left.add(source as usize)
                } else {
                    *right.add((source - left_items) as usize)
                };
            }
            row += stride;
        }
    }

    /// `directory[bucket]` is the first slot whose key prefix is at least `bucket`.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn build_directory<W: Word, O: OffsetWord>(
        keys: *const W,
        len: u64,
        radix_bits: u32,
        directory: *mut O,
    ) {
        let entries = (1_u64 << radix_bits) + 1;
        let mut bucket = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * thread::blockDim_x() as u64;
        let min = unsafe { *keys };
        let max = unsafe { *keys.add(len as usize - 1) };
        while bucket < entries {
            let (mut low, mut high) = (0_u64, len);
            while low < high {
                let middle = low + (high - low) / 2;
                if device::prefix(unsafe { *keys.add(middle as usize) }, min, max, radix_bits)
                    < bucket
                {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            unsafe {
                *directory.add(bucket as usize) = O::from_u64(low);
            }
            bucket += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn probe_keys<W: Word, O: OffsetWord>(
        view: RadixKeysView<W, O>,
        probes: *const W,
        count: u64,
        output: *mut KeyProbe,
    ) {
        let mut probe = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * thread::blockDim_x() as u64;
        while probe < count {
            let key = unsafe { *probes.add(probe as usize) };
            let (begin, end) = unsafe { device::range(view, key) };
            let contains = unsafe { device::contains(view, key) };
            let total = unsafe { device::count(view, key) };
            unsafe {
                *output.add(probe as usize) = KeyProbe {
                    contains: contains as u64,
                    count: total,
                    begin,
                    end,
                };
            }
            probe += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn probe_matches<W: Word, O: OffsetWord>(
        view: RadixKeysView<W, O>,
        probes: *const W,
        count: u64,
        output: *mut RadixMatch,
    ) {
        let mut probe = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * thread::blockDim_x() as u64;
        while probe < count {
            let key = unsafe { *probes.add(probe as usize) };
            let mut index = 0;
            while index < view.run_count {
                let cursor = unsafe { device::cursor(view, index, key) };
                unsafe {
                    *output.add((probe * view.run_count as u64 + index as u64) as usize) = cursor;
                }
                index += 1;
            }
            probe += stride;
        }
    }

    /// Copy each cursor's group of one column to `output[bases[i]..]`.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn gather_matches<T: ItemLane>(
        column: RadixItemColumn,
        cursors: *const RadixMatch,
        count: u64,
        bases: *const u64,
        total: u64,
        output: *mut T,
    ) {
        let mut index = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * thread::blockDim_x() as u64;
        while index < count {
            let cursor = unsafe { *cursors.add(index as usize) };
            let base = unsafe { *bases.add(index as usize) };
            if device::same_snapshot(column, cursor) {
                let mut offset = 0;
                while offset < cursor.end - cursor.begin && base + offset < total {
                    unsafe {
                        *output.add((base + offset) as usize) =
                            device::cursor_item::<T>(column, cursor, offset);
                    }
                    offset += 1;
                }
            }
            index += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn probe_intervals<W: Word, O: OffsetWord>(
        view: RadixKeysView<W, O>,
        low: *const W,
        high: *const W,
        count: u64,
        output: *mut IntervalProbe,
    ) {
        let mut probe = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * thread::blockDim_x() as u64;
        while probe < count {
            let (lo, hi) = unsafe { (*low.add(probe as usize), *high.add(probe as usize)) };
            let mut result = IntervalProbe {
                groups: 0,
                items: 0,
            };
            let mut index = 0;
            while index < view.run_count {
                let run = unsafe { device::run(view, index) };
                let (first, last) = unsafe { device::interval(run, lo, hi) };
                result.groups += last - first;
                result.items += unsafe { device::offset(run, last) - device::offset(run, first) };
                index += 1;
            }
            unsafe {
                *output.add(probe as usize) = result;
            }
            probe += stride;
        }
    }
}
