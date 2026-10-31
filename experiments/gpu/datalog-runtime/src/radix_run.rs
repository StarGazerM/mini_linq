//! Occurrence-preserving grouped device runs indexed by an exact radix-prefix
//! directory: the device counterpart of `cq_storage::RadixRun`.
//!
//! A run stores each distinct packed key once, group offsets in the selected
//! carrier `O`, and the selected item columns `C` in stable occurrence order.
//! Keys are [`Word`]s: the DL-DevicePacking word is the one order-preserving
//! key embedding and is stored whole, never truncated into the directory.
//! Every policy is a caller parameter ([`RadixRunParams`]) or a type fact
//! (`W`, `O`, `C`, and the kernels' compiled [`BLOCK_THREADS`] contract).
//! Grid sizes follow the shared occupancy convention for the target device.
//!
//! Key reads ([`RadixKeysView`]) carry no item pointers; item columns are read
//! by position through the separate [`RadixItemsView`].
use crate::{runs::{self, Native}, Result, Word};
use cuda_core::{DeviceCopy, LaunchConfig1D};
use gpu_device_columns::{Buffer, Pool};
use rccl_cub::{DeviceArithmeticRange, DeviceMap, DeviceSliceRead, DeviceSliceWrite};
use std::sync::Arc;

/// Threads per block of every radix-run kernel: their compiled launch contract.
pub const BLOCK_THREADS: u32 = 256;
const SIDE: u64 = 1 << 63;

mod sealed {
    pub trait Offset {}
    pub trait Lane {}
    pub trait Columns {}
}

/// A group-offset carrier, selected upstream from item-count bound evidence.
/// Builds and merges whose terminal item count exceeds `MAX` are rejected.
pub trait OffsetWord:
    sealed::Offset + Copy + Ord + std::fmt::Debug + DeviceCopy + rccl_cub::DeviceArithmetic + 'static
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
    fn from_u64(value: u64) -> Self { value as u32 }
    #[inline(always)]
    fn to_u64(self) -> u64 { self as u64 }
}
impl sealed::Offset for u64 {}
impl OffsetWord for u64 {
    const MAX: u64 = u64::MAX;
    #[inline(always)]
    fn from_u64(value: u64) -> Self { value }
    #[inline(always)]
    fn to_u64(self) -> u64 { self }
}

/// Whether carrier `O` represents the terminal offset of `items` occurrences.
pub fn offset_fits<O: OffsetWord>(items: usize) -> bool {
    u64::try_from(items).is_ok_and(|items| items <= O::MAX)
}

/// A fixed-width, byte-addressable item column element.
pub trait ItemLane: sealed::Lane + Copy + DeviceCopy + 'static {}
macro_rules! lanes {
    ($($ty:ty),*) => {$( impl sealed::Lane for $ty {} impl ItemLane for $ty {} )*};
}
lanes!(u8, i8, u16, i16, u32, i32, u64, i64, u128);
impl<const L: usize> sealed::Lane for [u64; L] {}
impl<const L: usize> ItemLane for [u64; L] {}

/// Formal rule GPU-RadixItemColumns:
/// An explicit item layout: one `Buffer<T>` column, a tuple of layouts stored as
/// independent columns, or `()` for key-only runs. Position `i` names the same
/// occurrence in every column; grouping and merging gather whole columns by
/// position and never reconstruct rows. The layout is selected by the provider
/// type, never by inspecting a row type.
pub trait DeviceItemColumns: sealed::Columns + Sized + 'static {
    /// Downloaded diagnostic columns.
    type Host;
    const COLUMNS: usize;
    /// Every column belongs to `pool` and has at least `rows` initialized entries.
    fn check(&self, pool: &Arc<Pool>, rows: usize) -> Result<()>;
    #[doc(hidden)]
    fn gather(&self, kernels: &ItemKernels, order: *const u32, rows: usize) -> Result<Self>;
    #[doc(hidden)]
    fn merge<O: OffsetWord>(
        left: &Self, right: &Self, kernels: &ItemKernels, left_items: usize, sources: *const O, rows: usize,
    ) -> Result<Self>;
    /// Append each column's device address in layout order.
    fn column_pointers(&self, out: &mut Vec<u64>);
    fn to_host(&self, rows: usize) -> Result<Self::Host>;
}

/// Loaded column-gather kernels; constructed only by [`DeviceRadixRun`].
pub struct ItemKernels {
    pool: Arc<Pool>,
    module: radix_kernels::LoadedModule,
}

impl ItemKernels {
    fn config(&self, function: &cuda_core::CudaFunction, parts: usize) -> Result<LaunchConfig1D> {
        let grid = cq_gpu_runtime::launch_grid_for_kernel(
            function, parts.div_ceil(BLOCK_THREADS as usize), BLOCK_THREADS, 0)?;
        Ok(LaunchConfig1D::new(grid, BLOCK_THREADS, 0))
    }
    fn shape() -> LaunchConfig1D { LaunchConfig1D::new(1, BLOCK_THREADS, 0) }
}

impl<T: ItemLane> sealed::Columns for Buffer<T> {}
impl<T: ItemLane> DeviceItemColumns for Buffer<T> {
    type Host = Vec<T>;
    const COLUMNS: usize = 1;
    fn check(&self, pool: &Arc<Pool>, rows: usize) -> Result<()> {
        if !Arc::ptr_eq(pool, self.pool()) || rows > self.len() {
            return Err("item column pool or live length differs from its keys".into());
        }
        Ok(())
    }
    fn gather(&self, kernels: &ItemKernels, order: *const u32, rows: usize) -> Result<Self> {
        // SAFETY: the gather initializes every output position before publication.
        let output = unsafe { Buffer::<T>::uninitialized(kernels.pool.clone(), rows)? };
        if rows != 0 {
            let module = &kernels.module;
            let initial = module.prepare_gather_items::<T>(ItemKernels::shape())?;
            let config = kernels.config(initial.function(), rows)?;
            module.gather_items::<T>(
                kernels.pool.stream(), &module.prepare_gather_items::<T>(config)?,
                self.ptr(), order, rows as u64, output.ptr(),
            )?;
        }
        Ok(output)
    }
    fn merge<O: OffsetWord>(
        left: &Self, right: &Self, kernels: &ItemKernels, left_items: usize, sources: *const O, rows: usize,
    ) -> Result<Self> {
        // SAFETY: every output position has exactly one source occurrence.
        let output = unsafe { Buffer::<T>::uninitialized(kernels.pool.clone(), rows)? };
        if rows != 0 {
            let module = &kernels.module;
            let initial = module.prepare_merge_items::<T, O>(ItemKernels::shape())?;
            let config = kernels.config(initial.function(), rows)?;
            module.merge_items::<T, O>(
                kernels.pool.stream(), &module.prepare_merge_items::<T, O>(config)?,
                left.ptr(), right.ptr(), left_items as u64, sources, rows as u64, output.ptr(),
            )?;
        }
        Ok(output)
    }
    fn column_pointers(&self, out: &mut Vec<u64>) { out.push(self.cu_deviceptr()); }
    fn to_host(&self, rows: usize) -> Result<Vec<T>> { self.to_host_prefix(rows) }
}

impl sealed::Columns for () {}
impl DeviceItemColumns for () {
    type Host = ();
    const COLUMNS: usize = 0;
    fn check(&self, _: &Arc<Pool>, _: usize) -> Result<()> { Ok(()) }
    fn gather(&self, _: &ItemKernels, _: *const u32, _: usize) -> Result<()> { Ok(()) }
    fn merge<O: OffsetWord>(_: &(), _: &(), _: &ItemKernels, _: usize, _: *const O, _: usize) -> Result<()> {
        Ok(())
    }
    fn column_pointers(&self, _: &mut Vec<u64>) {}
    fn to_host(&self, _: usize) -> Result<()> { Ok(()) }
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
}

impl RadixRunParams {
    pub fn validate(&self) -> Result<()> {
        if self.ratio < 2 { return Err("radix run ratio must be at least two".into()); }
        if self.radix_bits > 32 { return Err("radix directory bits must be at most 32".into()); }
        if self.merge_tile_rows == 0 { return Err("merge tile must contain at least one slot".into()); }
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

/// All published runs, oldest first. Reads visit every run.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RadixKeysView<W: Word, O: OffsetWord> {
    pub runs: *const RadixRunView<W, O>,
    pub run_count: u32,
    pub reserved: u32,
    /// Total occurrences across all runs; the exact absent-key position.
    pub items: u64,
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
}
// SAFETY: this C-layout descriptor contains only a device pointer and integers.
unsafe impl DeviceCopy for RadixItemsView {}

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

/// Half-open item positions of one run's group (empty when absent).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ItemRange {
    pub begin: u64,
    pub end: u64,
}
// SAFETY: plain integers.
unsafe impl DeviceCopy for ItemRange {}

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
            directory: if self.radix_bits == 0 { std::ptr::null() } else { self.directory.ptr() },
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
///
/// Each nonempty build downloads one group count; key reads never touch items.
pub struct DeviceRadixRun<W: Word, O: OffsetWord, C: DeviceItemColumns> {
    pool: Arc<Pool>,
    params: RadixRunParams,
    runs: Vec<Arc<Segment<W, O, C>>>,
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
            kernels: ItemKernels { pool: pool.clone(), module },
            pool, params, runs: Vec::new(), order,
        })
    }

    /// Group `rows` occurrences: the first `rows` keys with the same positions
    /// of every item column. Equal keys keep their input occurrence order.
    pub fn build(pool: Arc<Pool>, params: RadixRunParams, keys: &Buffer<W>, items: &C, rows: usize) -> Result<Self> {
        let mut run = Self::new(pool, params)?;
        run.publish(keys, items, rows)?;
        Ok(run)
    }

    pub fn pool(&self) -> &Arc<Pool> { &self.pool }
    pub fn params(&self) -> RadixRunParams { self.params }
    pub fn run_count(&self) -> usize { self.runs.len() }
    pub fn is_empty(&self) -> bool { self.runs.is_empty() }
    /// Total occurrences across all runs.
    pub fn item_count(&self) -> usize { self.runs.iter().map(|run| run.item_count).sum() }
    /// `(distinct keys, occurrences)` per run, oldest first.
    pub fn run_lengths(&self) -> Vec<(usize, usize)> {
        self.runs.iter().map(|run| (run.key_count, run.item_count)).collect()
    }

    /// Formal rule GPU-RadixPublish:
    /// Publication groups only the delta, appends its run, and merges adjacent
    /// newest runs by stable merge-path until every older run has at least
    /// `ratio` times the next run's occurrences: the CPU RadixRun ratio law.
    /// Reads of every snapshot include all unmerged runs. Empty deltas are no-ops;
    /// a failed build or merge leaves the published runs unchanged.
    pub fn publish(&mut self, keys: &Buffer<W>, items: &C, rows: usize) -> Result<()> {
        let Some(segment) = self.group(keys, items, rows)? else { return Ok(()) };
        let mut merged = Arc::new(segment);
        let mut keep = self.runs.len();
        while keep > 0 && self.runs[keep - 1].item_count / self.params.ratio < merged.item_count {
            merged = Arc::new(self.merge(&self.runs[keep - 1], &merged)?);
            keep -= 1;
        }
        self.runs.truncate(keep);
        self.runs.push(merged);
        Ok(())
    }

    /// Upload run descriptors. The snapshot retains its runs across later
    /// publications; all reads must use [`Self::pool`]'s stream.
    pub fn snapshot(&self) -> Result<RadixSnapshot<W, O, C>> {
        let views = self.runs.iter().map(|run| run.view()).collect::<Vec<_>>();
        u32::try_from(views.len()).map_err(|_| "radix run count exceeds u32")?;
        let mut columns = Vec::with_capacity(views.len() * C::COLUMNS);
        for run in &self.runs { run.items.column_pointers(&mut columns); }
        Ok(RadixSnapshot {
            descriptors: Buffer::from_host_async(self.pool.clone(), &views)?,
            columns: Buffer::from_host_async(self.pool.clone(), &columns)?,
            items: self.item_count() as u64,
            runs: self.runs.clone(),
            module: self.kernels.module.clone(),
            pool: self.pool.clone(),
        })
    }

    /// Explicit diagnostic download of every run, oldest first.
    pub fn to_host(&self) -> Result<Vec<HostRadixRun<W, O, C::Host>>> {
        self.runs.iter().map(|run| Ok(HostRadixRun {
            keys: run.keys.to_host_prefix(run.key_count)?,
            offsets: run.offsets.to_host_prefix(run.key_count + 1)?,
            directory: if run.radix_bits == 0 { Vec::new() } else { run.directory.to_host()? },
            items: run.items.to_host(run.item_count)?,
        })).collect()
    }

    fn launch_config(&self, function: &cuda_core::CudaFunction, parts: usize) -> Result<LaunchConfig1D> {
        self.kernels.config(function, parts)
    }

    fn group(&self, keys: &Buffer<W>, items: &C, rows: usize) -> Result<Option<Segment<W, O, C>>> {
        if !Arc::ptr_eq(&self.pool, keys.pool()) || rows > keys.len() {
            return Err("radix run keys belong to another pool or exceed their capacity".into());
        }
        items.check(&self.pool, rows)?;
        if rows > u32::MAX as usize { return Err("radix run build exceeds the u32 row domain".into()); }
        if !offset_fits::<O>(rows) { return Err("radix run items exceed the offset carrier".into()); }
        if rows == 0 { return Ok(None); }
        let pool = &self.pool;
        let module = &self.kernels.module;
        let cub = rccl_cub::cuda::Program::load(pool.context())?;
        let launch = cub.on(pool.stream())?;
        let permutation = runs::stable_order(pool, &self.order, &launch, keys, rows)?;
        let order = permutation.get().cu_deviceptr() as *const u32;
        // SAFETY: the flag kernel initializes all rows + 1 entries.
        let flags = unsafe { Buffer::<u32>::uninitialized(pool.clone(), rows + 1)? };
        let initial = module.prepare_group_flags::<W>(ItemKernels::shape())?;
        let config = self.launch_config(initial.function(), rows + 1)?;
        module.group_flags::<W>(
            pool.stream(), &module.prepare_group_flags::<W>(config)?,
            keys.ptr(), order, rows as u64, flags.ptr(),
        )?;
        // SAFETY: stable selections initialize the prefixes read below.
        let offsets = unsafe { Buffer::<O>::uninitialized(pool.clone(), rows + 1)? };
        let distinct = unsafe { Buffer::<W>::uninitialized(pool.clone(), rows)? };
        let mut offset_count = Native::<u64>::new(pool, 1)?;
        let mut key_count = Native::<u64>::new(pool, 1)?;
        let mut workspace = rccl_cub::thrust_integration::compaction_workspace_in(pool.resource(), &launch, rows + 1)?;
        rccl_cub::thrust_integration::device_select_flagged(
            &launch, DeviceArithmeticRange::new(O::from_u64(0), O::from_u64(1)),
            DeviceSliceRead::new(flags.ptr() as *const u32), DeviceSliceWrite::new(offsets.ptr()),
            offset_count.get_mut(), rows + 1, &mut workspace,
        )?;
        let source = keys.ptr() as *const W;
        let sorted = DeviceMap::new(DeviceSliceRead::new(order), move |row: u32| unsafe { *source.add(row as usize) });
        let mut workspace = rccl_cub::thrust_integration::compaction_workspace_in(pool.resource(), &launch, rows)?;
        rccl_cub::thrust_integration::device_select_flagged(
            &launch, sorted, DeviceSliceRead::new(flags.ptr() as *const u32),
            DeviceSliceWrite::new(distinct.ptr()), key_count.get_mut(), rows, &mut workspace,
        )?;
        let items = items.gather(&self.kernels, order, rows)?;
        let key_count = runs::read_count(pool, offset_count.get())? - 1;
        let directory = self.directory(&distinct, key_count)?;
        Ok(Some(Segment {
            keys: distinct, offsets, directory, items, key_count, item_count: rows,
            radix_bits: self.params.radix_bits,
        }))
    }

    fn merge(&self, left: &Segment<W, O, C>, right: &Segment<W, O, C>) -> Result<Segment<W, O, C>> {
        let total = left.item_count.checked_add(right.item_count).ok_or("merged item count overflow")?;
        if !offset_fits::<O>(total) { return Err("merged radix run items exceed the offset carrier".into()); }
        let slots = left.key_count + right.key_count;
        let pool = &self.pool;
        let module = &self.kernels.module;
        // SAFETY: the merge-path kernel initializes all slots and the terminal entry.
        let tags = unsafe { Buffer::<u64>::uninitialized(pool.clone(), slots)? };
        let positions = unsafe { Buffer::<O>::uninitialized(pool.clone(), slots + 1)? };
        let flags = unsafe { Buffer::<u32>::uninitialized(pool.clone(), slots + 1)? };
        let tile = self.params.merge_tile_rows as usize;
        let initial = module.prepare_merge_slots::<W, O>(ItemKernels::shape())?;
        let config = self.launch_config(initial.function(), slots.div_ceil(tile))?;
        module.merge_slots::<W, O>(
            pool.stream(), &module.prepare_merge_slots::<W, O>(config)?,
            left.view(), right.view(), tile as u64, tags.ptr(), positions.ptr(), flags.ptr(),
        )?;
        let cub = rccl_cub::cuda::Program::load(pool.context())?;
        let launch = cub.on(pool.stream())?;
        // SAFETY: stable selections initialize the prefixes read below.
        let offsets = unsafe { Buffer::<O>::uninitialized(pool.clone(), slots + 1)? };
        let distinct = unsafe { Buffer::<W>::uninitialized(pool.clone(), slots)? };
        let mut offset_count = Native::<u64>::new(pool, 1)?;
        let mut key_count = Native::<u64>::new(pool, 1)?;
        let mut workspace = rccl_cub::thrust_integration::compaction_workspace_in(pool.resource(), &launch, slots + 1)?;
        rccl_cub::thrust_integration::device_select_flagged(
            &launch, DeviceSliceRead::new(positions.ptr() as *const O),
            DeviceSliceRead::new(flags.ptr() as *const u32), DeviceSliceWrite::new(offsets.ptr()),
            offset_count.get_mut(), slots + 1, &mut workspace,
        )?;
        let (left_keys, right_keys) = (left.keys.ptr() as *const W, right.keys.ptr() as *const W);
        let merged = DeviceMap::new(DeviceSliceRead::new(tags.ptr() as *const u64), move |tag: u64| unsafe {
            if tag & SIDE == 0 { *left_keys.add(tag as usize) } else { *right_keys.add((tag & !SIDE) as usize) }
        });
        let mut workspace = rccl_cub::thrust_integration::compaction_workspace_in(pool.resource(), &launch, slots)?;
        rccl_cub::thrust_integration::device_select_flagged(
            &launch, merged, DeviceSliceRead::new(flags.ptr() as *const u32),
            DeviceSliceWrite::new(distinct.ptr()), key_count.get_mut(), slots, &mut workspace,
        )?;
        // SAFETY: each merged item position has exactly one source occurrence.
        let sources = unsafe { Buffer::<O>::uninitialized(pool.clone(), total)? };
        let initial = module.prepare_item_sources::<W, O>(ItemKernels::shape())?;
        let config = self.launch_config(initial.function(), total)?;
        module.item_sources::<W, O>(
            pool.stream(), &module.prepare_item_sources::<W, O>(config)?,
            left.view(), right.view(), tags.ptr(), positions.ptr(), slots as u64, total as u64, sources.ptr(),
        )?;
        let items = C::merge(&left.items, &right.items, &self.kernels, left.item_count, sources.ptr(), total)?;
        let key_count = runs::read_count(pool, offset_count.get())? - 1;
        let directory = self.directory(&distinct, key_count)?;
        Ok(Segment {
            keys: distinct, offsets, directory, items, key_count, item_count: total,
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
        let initial = module.prepare_build_directory::<W, O>(ItemKernels::shape())?;
        let config = self.launch_config(initial.function(), entries)?;
        module.build_directory::<W, O>(
            self.pool.stream(), &module.prepare_build_directory::<W, O>(config)?,
            keys.ptr(), key_count as u64, bits, directory.ptr(),
        )?;
        Ok(directory)
    }
}

/// Immutable descriptors over a set of published runs, retained independently
/// of later publications. Batch reads launch on the pool stream.
pub struct RadixSnapshot<W: Word, O: OffsetWord, C> {
    descriptors: Buffer<RadixRunView<W, O>>,
    columns: Buffer<u64>,
    items: u64,
    runs: Vec<Arc<Segment<W, O, C>>>,
    module: radix_kernels::LoadedModule,
    pool: Arc<Pool>,
}

impl<W: Word, O: OffsetWord, C: DeviceItemColumns> RadixSnapshot<W, O, C> {
    pub fn pool(&self) -> &Arc<Pool> { &self.pool }
    pub fn run_count(&self) -> usize { self.runs.len() }
    pub fn item_count(&self) -> usize { self.items as usize }
    /// Key-read ABI: keys, offsets and directories only.
    pub fn keys_view(&self) -> RadixKeysView<W, O> {
        RadixKeysView {
            runs: self.descriptors.ptr(), run_count: self.runs.len() as u32, reserved: 0, items: self.items,
        }
    }
    /// Item-read ABI, separate from every key read.
    pub fn items_view(&self) -> RadixItemsView {
        RadixItemsView {
            columns: self.columns.ptr(), column_count: C::COLUMNS as u32, run_count: self.runs.len() as u32,
        }
    }

    fn config(&self, function: &cuda_core::CudaFunction, parts: usize) -> Result<LaunchConfig1D> {
        let grid = cq_gpu_runtime::launch_grid_for_kernel(
            function, parts.div_ceil(BLOCK_THREADS as usize), BLOCK_THREADS, 0)?;
        Ok(LaunchConfig1D::new(grid, BLOCK_THREADS, 0))
    }

    fn check_probes(&self, probes: &Buffer<W>, count: usize) -> Result<()> {
        if !Arc::ptr_eq(&self.pool, probes.pool()) || count > probes.len() {
            return Err("probe keys belong to another pool or exceed their capacity".into());
        }
        Ok(())
    }

    /// Membership, count and concatenated stable range for each probe key.
    pub fn probe_keys(&self, probes: &Buffer<W>, count: usize) -> Result<Buffer<KeyProbe>> {
        self.check_probes(probes, count)?;
        // SAFETY: one thread initializes each result.
        let output = unsafe { Buffer::<KeyProbe>::uninitialized(self.pool.clone(), count)? };
        if count != 0 {
            let initial = self.module.prepare_probe_keys::<W, O>(ItemKernels::shape())?;
            let config = self.config(initial.function(), count)?;
            self.module.probe_keys::<W, O>(
                self.pool.stream(), &self.module.prepare_probe_keys::<W, O>(config)?,
                self.keys_view(), probes.ptr(), count as u64, output.ptr(),
            )?;
        }
        Ok(output)
    }

    /// Per-run item ranges for each probe: `output[probe * run_count + run]`.
    /// These are the cursor groups an expansion reads through [`Self::items_view`].
    pub fn probe_groups(&self, probes: &Buffer<W>, count: usize) -> Result<Buffer<ItemRange>> {
        self.check_probes(probes, count)?;
        let len = count.checked_mul(self.runs.len()).ok_or("probe group count overflow")?;
        // SAFETY: one thread initializes every run range of its probe.
        let output = unsafe { Buffer::<ItemRange>::uninitialized(self.pool.clone(), len)? };
        if len != 0 {
            let initial = self.module.prepare_probe_groups::<W, O>(ItemKernels::shape())?;
            let config = self.config(initial.function(), count)?;
            self.module.probe_groups::<W, O>(
                self.pool.stream(), &self.module.prepare_probe_groups::<W, O>(config)?,
                self.keys_view(), probes.ptr(), count as u64, output.ptr(),
            )?;
        }
        Ok(output)
    }

    /// Groups and occurrences with keys in each inclusive interval `[low, high]`.
    pub fn probe_intervals(&self, low: &Buffer<W>, high: &Buffer<W>, count: usize) -> Result<Buffer<IntervalProbe>> {
        self.check_probes(low, count)?;
        self.check_probes(high, count)?;
        // SAFETY: one thread initializes each result.
        let output = unsafe { Buffer::<IntervalProbe>::uninitialized(self.pool.clone(), count)? };
        if count != 0 {
            let initial = self.module.prepare_probe_intervals::<W, O>(ItemKernels::shape())?;
            let config = self.config(initial.function(), count)?;
            self.module.probe_intervals::<W, O>(
                self.pool.stream(), &self.module.prepare_probe_intervals::<W, O>(config)?,
                self.keys_view(), low.ptr(), high.ptr(), count as u64, output.ptr(),
            )?;
        }
        Ok(output)
    }
}

/// Pointer-only reads usable inside generated device kernels.
pub mod device {
    use super::{OffsetWord, RadixItemsView, RadixKeysView, RadixRunView};
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
            if value != 0 { return lane * 64 + 64 - value.leading_zeros(); }
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
        if len == 0 { return (0, false); }
        let min = unsafe { *run.keys };
        if key.compare(&min).is_lt() { return (0, false); }
        let max = unsafe { *run.keys.add(len as usize - 1) };
        let last = key.compare(&max);
        if last.is_gt() { return (len, false); }
        if last.is_eq() { return (len - 1, true); }
        let (mut low, mut high) = if run.radix_bits == 0 { (0, len) } else {
            let bucket = prefix(key, min, max, run.radix_bits) as usize;
            unsafe { ((*run.directory.add(bucket)).to_u64(), (*run.directory.add(bucket + 1)).to_u64()) }
        };
        while low < high {
            let middle = low + (high - low) / 2;
            if unsafe { &*run.keys.add(middle as usize) }.compare(&key).is_lt() { low = middle + 1; } else { high = middle; }
        }
        (low, low < len && unsafe { &*run.keys.add(low as usize) }.compare(&key).is_eq())
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
        if found { unsafe { (offset(run, slot), offset(run, slot + 1)) } } else { (run.items, run.items) }
    }

    /// # Safety
    /// `view` must describe live runs readable on this device.
    #[inline(always)]
    pub unsafe fn run<W: Word, O: OffsetWord>(view: RadixKeysView<W, O>, run: u32) -> RadixRunView<W, O> {
        unsafe { *view.runs.add(run as usize) }
    }

    /// # Safety
    /// `view` must describe live runs readable on this device.
    #[inline(always)]
    pub unsafe fn contains<W: Word, O: OffsetWord>(view: RadixKeysView<W, O>, key: W) -> bool {
        let mut index = 0;
        while index < view.run_count {
            if unsafe { search(run(view, index), key) }.1 { return true; }
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
            if found { count += unsafe { offset(current, slot + 1) } - start; }
            index += 1;
        }
        if count == 0 { (view.items, view.items) } else { (begin, begin + count) }
    }

    /// Slot interval `[first, last)` of one run's keys within inclusive `[low, high]`.
    /// # Safety
    /// `run` must describe a live run readable on this device.
    #[inline(always)]
    pub unsafe fn interval<W: Word, O: OffsetWord>(run: RadixRunView<W, O>, low: W, high: W) -> (u64, u64) {
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
        let address = unsafe { *items.columns.add((run * items.column_count + column) as usize) };
        unsafe { *(address as *const T).add(position as usize) }
    }
}

#[cuda_host::cuda_module]
mod radix_kernels {
    use crate::{
        radix_run::{device, IntervalProbe, ItemLane, ItemRange, KeyProbe, OffsetWord, RadixKeysView, RadixRunView, SIDE},
        Word,
    };
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};

    /// Mark group starts of the permuted keys; entry `rows` is the terminal.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn group_flags<W: Word>(input: *const W, order: *const u32, rows: u64, flags: *mut u32) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while row <= rows {
            let start = row == 0 || row == rows || unsafe {
                !(*input.add(*order.add(row as usize) as usize))
                    .compare(&*input.add(*order.add(row as usize - 1) as usize)).is_eq()
            };
            unsafe { *flags.add(row as usize) = start as u32; }
            row += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn gather_items<T: ItemLane>(input: *const T, order: *const u32, rows: u64, output: *mut T) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while row < rows {
            unsafe { *output.add(row as usize) = *input.add(*order.add(row as usize) as usize); }
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
        left: RadixRunView<W, O>, right: RadixRunView<W, O>, tile_rows: u64,
        tags: *mut u64, positions: *mut O, flags: *mut u32,
    ) {
        let (left_len, right_len) = (left.keys_len as usize, right.keys_len as usize);
        let slots = left_len + right_len;
        let mut tile = thread::index_1d().get() as usize;
        let stride = thread::gridDim_x() as usize * 256;
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
            let mut a = unsafe { crate::runs::corank(left.keys, left_len, right.keys, right_len, start) };
            let mut b = start - a;
            let mut slot = start;
            while slot < end {
                let take_left = a < left_len && (b == right_len
                    || !unsafe { &*left.keys.add(a) }.compare(unsafe { &*right.keys.add(b) }).is_gt());
                let position = unsafe { device::offset(left, a as u64) + device::offset(right, b as u64) };
                // Runs hold distinct keys, so only the other run's predecessor can repeat this key.
                let (tag, repeat) = if take_left {
                    let key = unsafe { &*left.keys.add(a) };
                    (a as u64, b > 0 && unsafe { &*right.keys.add(b - 1) }.compare(key).is_eq())
                } else {
                    let key = unsafe { &*right.keys.add(b) };
                    (b as u64 | SIDE, a > 0 && unsafe { &*left.keys.add(a - 1) }.compare(key).is_eq())
                };
                unsafe {
                    *tags.add(slot) = tag;
                    *positions.add(slot) = O::from_u64(position);
                    *flags.add(slot) = !repeat as u32;
                }
                if take_left { a += 1; } else { b += 1; }
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
        left: RadixRunView<W, O>, right: RadixRunView<W, O>, tags: *const u64, positions: *const O,
        slots: u64, total: u64, sources: *mut O,
    ) {
        let mut item = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while item < total {
            // Slot positions strictly increase: every group is nonempty.
            let (mut low, mut high) = (0_u64, slots);
            while low < high {
                let middle = low + (high - low) / 2;
                if unsafe { *positions.add(middle as usize) }.to_u64() <= item { low = middle + 1; } else { high = middle; }
            }
            let slot = low - 1;
            let tag = unsafe { *tags.add(slot as usize) };
            let base = if tag & SIDE == 0 {
                unsafe { device::offset(left, tag) }
            } else {
                left.items + unsafe { device::offset(right, tag & !SIDE) }
            };
            let start = unsafe { *positions.add(slot as usize) }.to_u64();
            unsafe { *sources.add(item as usize) = O::from_u64(base + item - start); }
            item += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn merge_items<T: ItemLane, O: OffsetWord>(
        left: *const T, right: *const T, left_items: u64, sources: *const O, rows: u64, output: *mut T,
    ) {
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
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
    pub fn build_directory<W: Word, O: OffsetWord>(keys: *const W, len: u64, radix_bits: u32, directory: *mut O) {
        let entries = (1_u64 << radix_bits) + 1;
        let mut bucket = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        let min = unsafe { *keys };
        let max = unsafe { *keys.add(len as usize - 1) };
        while bucket < entries {
            let (mut low, mut high) = (0_u64, len);
            while low < high {
                let middle = low + (high - low) / 2;
                if device::prefix(unsafe { *keys.add(middle as usize) }, min, max, radix_bits) < bucket {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            unsafe { *directory.add(bucket as usize) = O::from_u64(low); }
            bucket += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn probe_keys<W: Word, O: OffsetWord>(view: RadixKeysView<W, O>, probes: *const W, count: u64, output: *mut KeyProbe) {
        let mut probe = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while probe < count {
            let key = unsafe { *probes.add(probe as usize) };
            let (begin, end) = unsafe { device::range(view, key) };
            let contains = unsafe { device::contains(view, key) };
            let total = unsafe { device::count(view, key) };
            unsafe { *output.add(probe as usize) = KeyProbe { contains: contains as u64, count: total, begin, end }; }
            probe += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn probe_groups<W: Word, O: OffsetWord>(view: RadixKeysView<W, O>, probes: *const W, count: u64, output: *mut ItemRange) {
        let mut probe = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while probe < count {
            let key = unsafe { *probes.add(probe as usize) };
            let mut index = 0;
            while index < view.run_count {
                let (begin, end) = unsafe { device::group(device::run(view, index), key) };
                unsafe { *output.add((probe * view.run_count as u64 + index as u64) as usize) = ItemRange { begin, end }; }
                index += 1;
            }
            probe += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn probe_intervals<W: Word, O: OffsetWord>(
        view: RadixKeysView<W, O>, low: *const W, high: *const W, count: u64, output: *mut IntervalProbe,
    ) {
        let mut probe = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while probe < count {
            let (lo, hi) = unsafe { (*low.add(probe as usize), *high.add(probe as usize)) };
            let mut result = IntervalProbe { groups: 0, items: 0 };
            let mut index = 0;
            while index < view.run_count {
                let run = unsafe { device::run(view, index) };
                let (first, last) = unsafe { device::interval(run, lo, hi) };
                result.groups += last - first;
                result.items += unsafe { device::offset(run, last) - device::offset(run, first) };
                index += 1;
            }
            unsafe { *output.add(probe as usize) = result; }
            probe += stride;
        }
    }
}
