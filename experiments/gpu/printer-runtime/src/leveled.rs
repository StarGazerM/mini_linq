//! Maintained immutable sorted runs. Admission guarantees disjoint deltas; this
//! module preserves their full multiset and never deduplicates during compaction.
//! All producers, readers, and retirement use the runtime pool's single stream.
use crate::{
    Bounds, Buffer, DeviceColumns, Dictionary, Execution, HostColumns, Plain, Pool, Result,
    RowCodec, RunView, Runtime, TableId, failure,
};
use cuda_core::{CudaContext, CudaStream, DeviceCopy, LaunchConfig1D};
use gpu_blocked_run::{Buffer as Scratch, RankView, Run};
use rccl_memory::ContiguousDeviceRange;
use std::{marker::PhantomData, sync::Arc};

pub const MAX_LEVELS: usize = 16;

// Parameter-space packing covers the built-in tuple codecs. Wider custom rows
// retain the exact lexicographic comparator rather than truncating their keys.
const PACK_COLUMNS: usize = 12;
#[repr(C)]
#[derive(Clone, Copy)]
struct SmallPacking {
    bases: [i64; PACK_COLUMNS],
    shifts: [u32; PACK_COLUMNS],
    row_bits: u32,
    end_bits: u32,
    enabled: u32,
}
unsafe impl DeviceCopy for SmallPacking {}
impl SmallPacking {
    fn new(n: usize, order: &[usize], bounds: &[Bounds]) -> Self {
        let row_bits = usize::BITS - (n - 1).leading_zeros();
        let mut value = Self {
            bases: [0; PACK_COLUMNS],
            shifts: [0; PACK_COLUMNS],
            row_bits,
            end_bits: row_bits,
            enabled: (order.len() <= PACK_COLUMNS && n > 1) as u32,
        };
        if value.enabled != 0 {
            for &slot in order.iter().rev() {
                let (min, max) = domain(bounds[slot]);
                let span = (max as i128 - min as i128 + 1).max(1) as u128;
                let bits = u128::BITS - (span - 1).leading_zeros();
                value.bases[slot] = min;
                value.shifts[slot] = value.end_bits;
                value.end_bits += bits;
            }
            value.enabled = (value.end_bits <= 64) as u32;
        }
        value
    }
}

const EMPTY_RUN: RunView = RunView {
    rank: RankView {
        bits: core::ptr::null(),
        prefix: core::ptr::null(),
        offsets: core::ptr::null(),
        min_key: 0,
        max_key: -1,
    },
    columns: core::ptr::null(),
    len: 0,
    count: core::ptr::null(),
    bucket_mask: 0,
};

/// Nonowning round snapshot. Enqueue every reader before the next insertion or
/// index drop, on this index's runtime stream. It is not a cross-stream lease.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LeveledRunsView {
    pub runs: [RunView; MAX_LEVELS],
    pub count: u32,
}
unsafe impl DeviceCopy for LeveledRunsView {}

/// Read the count without materializing the fixed-size snapshot array.
///
/// # Safety
/// `snapshot` must name a live, stream-published immutable device snapshot.
#[cuda_device::device]
pub unsafe fn leveled_count(snapshot: *const LeveledRunsView) -> u32 {
    unsafe { cuda_device::read_only::load(&*core::ptr::addr_of!((*snapshot).count)) }
}

/// Read one run directly from immutable global snapshot storage.
///
/// # Safety
/// The snapshot and its buffers must be live; `index` must be below its count.
#[cuda_device::device]
pub unsafe fn leveled_run(snapshot: *const LeveledRunsView, index: usize) -> RunView {
    let run = unsafe {
        core::ptr::addr_of!((*snapshot).runs)
            .cast::<RunView>()
            .add(index)
    };
    unsafe {
        macro_rules! word {
            ($($field:tt)*) => {
                cuda_device::read_only::load(&*core::ptr::addr_of!((*run).$($field)*).cast::<u64>())
            };
        }
        RunView {
            rank: RankView {
                bits: word!(rank.bits) as *const u64,
                prefix: word!(rank.prefix) as *const u32,
                offsets: word!(rank.offsets) as *const u32,
                min_key: word!(rank.min_key) as i64,
                max_key: word!(rank.max_key) as i64,
            },
            columns: word!(columns) as *const *const i64,
            len: word!(len),
            count: word!(count) as *const u32,
            bucket_mask: word!(bucket_mask),
        }
    }
}

fn runs_view(runs: &[Option<Arc<ResidentRun<'_>>>]) -> LeveledRunsView {
    let mut view = LeveledRunsView {
        runs: [EMPTY_RUN; MAX_LEVELS],
        count: 0,
    };
    for run in runs.iter().flatten() {
        view.runs[view.count as usize] = run.view;
        view.count += 1;
    }
    view
}

/// Block-wide predicate OR using one barrier, for skipping empty run work.
///
/// # Safety
/// Every thread in the block must reach this call convergently. Barrier zero
/// must not be in use by another split-phase collective.
#[cuda_device::device]
#[cuda_device::convergent]
pub unsafe fn leveled_any(predicate: bool) -> bool {
    let result: u32;
    unsafe {
        cuda_device::ptx_asm!(
            "{ .reg .pred p, q; setp.ne.u32 p, %1, 0; bar.red.or.pred q, 0, p; selp.u32 %0, 1, 0, q; }",
            out("=r") result,
            in("r") predicate as u32,
            clobber("memory"),
        );
    }
    result != 0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeveledMergeReason {
    /// The highest representable level cannot accept another promoted run.
    MaxLevels,
}

/// Host-known work counters; observing them neither launches nor synchronizes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LeveledStatus {
    pub inserts: u64,
    pub sorted_rows: u64,
    pub merges: u64,
    pub merged_rows: u64,
    pub forced_merges: u64,
    pub last_forced_merge: Option<LeveledMergeReason>,
}

struct Chain<'runtime> {
    levels: Vec<Vec<usize>>,
    order: Vec<usize>,
    device_order: Buffer<u64>,
    runs: Vec<Option<Arc<ResidentRun<'runtime>>>>,
    snapshot: Buffer<LeveledRunsView>,
}

// The view owns nothing. These owners keep every address in it live until their
// stream-ordered deallocation, including on a partially submitted error path.
#[allow(dead_code)]
enum Storage<'runtime> {
    Built(Run<'runtime>),
    Packed {
        slab: Buffer<u64>,
        bits: Option<Scratch<'runtime, u64>>,
    },
}
struct ResidentRun<'runtime> {
    view: RunView,
    bounds: Vec<Bounds>,
    _pointers: Option<Buffer<u64>>,
    _storage: Storage<'runtime>,
}

/// A disjoint union of full-row-ordered runs for each recorded key chain.
/// `Layout` is the compiler's certificate; this carrier makes no planner choices.
pub struct LeveledDeviceIndex<'runtime, 'data, Row, C = Plain, Layout = ()> {
    runtime: &'runtime Runtime,
    kernels: leveled_kernels::LoadedModule,
    chains: Vec<Chain<'runtime>>,
    ratio: u32,
    base_rows: u64,
    rows: u64,
    dictionary: Dictionary<'data>,
    status: LeveledStatus,
    marker: PhantomData<(Row, C, Layout)>,
}

impl<'runtime, 'data, Row, C, Layout> LeveledDeviceIndex<'runtime, 'data, Row, C, Layout> {
    /// Use one row as the level-zero bound. Call `with_base_rows` when policy
    /// advice records a different base. Ratio is always explicit.
    pub fn new(runtime: &'runtime Runtime, chains: &[&[&[usize]]], ratio: u32) -> Result<Self>
    where
        Row: RowCodec<'data>,
    {
        Self::with_base_rows(runtime, chains, ratio, 1)
    }

    pub fn with_base_rows(
        runtime: &'runtime Runtime,
        chains: &[&[&[usize]]],
        ratio: u32,
        base_rows: u64,
    ) -> Result<Self>
    where
        Row: RowCodec<'data>,
    {
        if ratio < 2 || base_rows == 0 || base_rows > u32::MAX as u64 || chains.is_empty() {
            return Err(failure(
                "CQ_GPU_LEVELED_POLICY",
                "nonempty chains, ratio >= 2 and base in 1..=u32::MAX required",
            ));
        }
        for levels in chains {
            crate::validate_frozen_layout(levels, Row::WIDTH)?;
        }
        let kernels = unsafe { leveled_kernels::load(runtime.context())? };
        let mut recorded = Vec::<Chain<'runtime>>::new();
        for levels in chains {
            let levels: Vec<Vec<usize>> = levels.iter().map(|v| v.to_vec()).collect();
            if recorded.iter().any(|c| c.levels == levels) {
                continue;
            }
            let mut order: Vec<usize> = levels.iter().flatten().copied().collect();
            for slot in 0..Row::WIDTH {
                if !order.contains(&slot) {
                    order.push(slot);
                }
            }
            let device_order = runtime.upload_descriptors(order.iter().map(|&slot| slot as u64))?;
            let snapshot = runtime.upload_descriptors(std::iter::once(LeveledRunsView {
                runs: [EMPTY_RUN; MAX_LEVELS],
                count: 0,
            }))?;
            recorded.push(Chain {
                levels,
                order,
                device_order,
                runs: vec![None; MAX_LEVELS],
                snapshot,
            });
        }
        Ok(Self {
            runtime,
            kernels,
            chains: recorded,
            ratio,
            base_rows,
            rows: 0,
            dictionary: Dictionary::default(),
            status: LeveledStatus::default(),
            marker: PhantomData,
        })
    }

    /// Append exactly the host-known admitted prefix of `delta`. Admission must
    /// already have checked its status/count and dictionary compatibility. No
    /// count, row, or extrema are downloaded here. On failure published runs and
    /// host counters are unchanged; submitted scratch is retired on the stream.
    pub fn insert(&mut self, delta: &DeviceColumns<'data, Row, C>, delta_rows: u64) -> Result<()> {
        let _nvtx = crate::nvtx::range(c"index/leveled/insert");
        if !Arc::ptr_eq(self.runtime.pool(), delta.pool()) {
            return Err(failure(
                "CQ_GPU_CONTEXT",
                "leveled delta must share the runtime pool and stream",
            ));
        }
        let width = self.chains[0].order.len();
        if delta.storage().columns.len() != width || delta_rows > delta.capacity() as u64 {
            return Err(failure(
                "CQ_GPU_LEVELED_DELTA",
                "delta schema or host-known count exceeds storage",
            ));
        }
        let total = self
            .rows
            .checked_add(delta_rows)
            .filter(|&n| n <= u32::MAX as u64)
            .ok_or_else(|| {
                failure(
                    "CQ_GPU_LEVELED_ROWS",
                    "leveled cardinality exceeds u32::MAX",
                )
            })?;
        self.dictionary.merge(delta.dictionary())?;
        if delta_rows == 0 {
            return Ok(());
        }
        let n = delta_rows as usize;
        let table = delta.storage();
        // Plain columns are borrowed at the exact admitted length, not copied
        // and not sorted at their potentially much larger output capacity.
        let mut decoded = Vec::with_capacity(if n > SMALL_ROWS { width } else { 0 });
        for column in table.columns.iter().filter(|_| n > SMALL_ROWS) {
            if unsafe { column.plain_buffer() }.is_some() {
                decoded.push(None);
            } else {
                let buffer = unsafe { Buffer::uninitialized(self.runtime.pool().clone(), n)? };
                let launch = self.kernels.prepare_leveled_decode_column(config(n))?;
                unsafe {
                    self.kernels.leveled_decode_column(
                        self.runtime.stream(),
                        &launch,
                        column.view(),
                        n,
                        buffer.ptr(),
                    )?;
                }
                decoded.push(Some(buffer));
            }
        }
        let inputs: Vec<_> = table
            .columns
            .iter()
            .zip(&decoded)
            .map(|(column, decoded)| ExactPrefix {
                buffer: unsafe { column.plain_buffer() }
                    .or(decoded.as_ref())
                    .unwrap(),
                len: n,
            })
            .collect();
        let mut staged = Vec::with_capacity(self.chains.len());
        let mut status = self.status;
        for chain in &self.chains {
            let mut slots = chain.runs.clone();
            let mut carry = Arc::new(if n <= SMALL_ROWS {
                self.build_small(chain, table, n)?
            } else {
                self.build(chain, &inputs, &table.bounds)?
            });
            status.sorted_rows += delta_rows;
            let mut level = 0;
            loop {
                if let Some(existing) = slots[level].take() {
                    let merged_rows = existing.view.len + carry.view.len;
                    let forced = level == MAX_LEVELS - 1
                        && merged_rows > level_bound(self.base_rows, self.ratio, level);
                    carry = Arc::new(self.merge(chain, &existing, &carry)?);
                    status.merges += 1;
                    status.merged_rows += merged_rows;
                    if forced {
                        status.forced_merges += 1;
                        status.last_forced_merge = Some(LeveledMergeReason::MaxLevels);
                    }
                }
                if carry.view.len <= level_bound(self.base_rows, self.ratio, level)
                    || level == MAX_LEVELS - 1
                {
                    slots[level] = Some(carry);
                    break;
                }
                level += 1;
            }
            let snapshot = self
                .runtime
                .upload_descriptors(std::iter::once(runs_view(&slots)))?;
            staged.push((slots, snapshot));
        }
        // Publication occurs only after every chain successfully enqueued. Any
        // earlier snapshot reader precedes both the merge and these frees.
        for (chain, (slots, snapshot)) in self.chains.iter_mut().zip(staged) {
            chain.runs = slots;
            chain.snapshot = snapshot;
        }
        self.rows = total;
        status.inserts += 1;
        self.status = status;
        Ok(())
    }

    fn chain(&self, levels: &[&[usize]]) -> Result<&Chain<'runtime>> {
        self.chains
            .iter()
            .find(|chain| {
                chain.levels.len() == levels.len()
                    && chain
                        .levels
                        .iter()
                        .zip(levels)
                        .all(|(a, b)| a.as_slice() == *b)
            })
            .ok_or_else(|| {
                failure(
                    "CQ_GPU_LEVELED_LAYOUT_MISSING",
                    format!("no exact key-level layout {levels:?}"),
                )
            })
    }
    pub fn snapshot(&self, levels: &[&[usize]]) -> Result<LeveledRunsView> {
        Ok(runs_view(&self.chain(levels)?.runs))
    }
    /// Borrow the immutable device descriptor published at the last insert.
    /// Enqueue every reader on the runtime stream before the next insertion.
    pub fn snapshot_ptr(&self, levels: &[&[usize]]) -> Result<*const LeveledRunsView> {
        Ok(self.chain(levels)?.snapshot.ptr())
    }
    pub fn rows(&self) -> u64 {
        self.rows
    }
    /// Each chain follows the same host-only occupancy policy.
    pub fn run_count(&self) -> usize {
        self.chains[0].runs.iter().flatten().count()
    }
    pub fn status(&self) -> LeveledStatus {
        self.status
    }

    fn build_small(
        &self,
        chain: &Chain<'runtime>,
        table: &gpu_device_columns::Table,
        n: usize,
    ) -> Result<ResidentRun<'runtime>> {
        let (run, data, temporary) =
            self.allocate_packed(n, chain.order.len(), chain.order[0], &table.bounds, true)?;
        let packing = SmallPacking::new(n, &chain.order, &table.bounds);
        let size = n.min(SMALL_TILE).next_power_of_two();
        let shared = if packing.enabled != 0 {
            (size * 8 + (size * 4).max(32)) as u32
        } else {
            (size * 4).max(32) as u32
        };
        if n <= SMALL_TILE {
            let launch = self
                .kernels
                .prepare_leveled_small_build(LaunchConfig1D::new(1, 256, shared))?;
            unsafe {
                self.kernels.leveled_small_build(
                    self.runtime.stream(),
                    &launch,
                    table.views.ptr(),
                    chain.device_order.ptr(),
                    chain.order.len(),
                    n,
                    run.view,
                    data,
                    temporary,
                    packing,
                )?;
            }
        } else {
            let sorted_keys = unsafe { temporary.add(chain.order.len() * n) as *mut u64 };
            let sorted_rows = unsafe { sorted_keys.add(n) as *mut u32 };
            let tiles = self
                .kernels
                .prepare_leveled_sort_tiles(LaunchConfig1D::new(
                    n.div_ceil(SMALL_TILE) as u32,
                    256,
                    shared,
                ))?;
            unsafe {
                self.kernels.leveled_sort_tiles(
                    self.runtime.stream(),
                    &tiles,
                    table.views.ptr(),
                    chain.device_order.ptr(),
                    chain.order.len(),
                    n,
                    run.view,
                    data,
                    temporary,
                    packing,
                    sorted_keys,
                    sorted_rows,
                )?;
            }
            let merge = self.kernels.prepare_leveled_merge_tiles(config(n))?;
            unsafe {
                self.kernels.leveled_merge_tiles(
                    self.runtime.stream(),
                    &merge,
                    temporary,
                    sorted_keys,
                    sorted_rows,
                    data,
                    n,
                    chain.order.len(),
                    chain.device_order.ptr(),
                    packing.enabled != 0,
                )?;
            }
            let finish = self
                .kernels
                .prepare_leveled_finish_small(LaunchConfig1D::new(1, 256, 32))?;
            unsafe {
                self.kernels.leveled_finish_small(
                    self.runtime.stream(),
                    &finish,
                    run.view,
                    data,
                    chain.order[0],
                )?;
            }
        }
        Ok(run)
    }

    /// One allocation holds columns and descriptor/directory metadata. A small
    /// build also reserves decoded input scratch in this same stream-owned slab.
    fn allocate_packed(
        &self,
        n: usize,
        width: usize,
        first: usize,
        bounds: &[Bounds],
        small: bool,
    ) -> Result<(ResidentRun<'runtime>, *mut i64, *mut i64)> {
        let (min_key, max_key) = domain(bounds[first]);
        let span = (max_key as i128 - min_key as i128 + 1).max(0);
        if span == 0 || span > gpu_blocked_run::MAX_DOMAIN_KEYS as i128 {
            return Err(failure(
                "CQ_GPU_RUN_DOMAIN",
                "nonempty run requires a dense domain within 2^32 key positions",
            ));
        }
        let words = (span as usize).div_ceil(64);
        let columns_at = width + 1;
        let temporary_at = columns_at + width * n;
        let bits_at = temporary_at
            + if small {
                width * n + if n > SMALL_TILE { n + n.div_ceil(2) } else { 0 }
            } else {
                0
            };
        let prefix_at = bits_at + if small { words } else { 0 };
        let offsets_at = prefix_at + words.div_ceil(2);
        let total = offsets_at + (n.min(span as usize) + 1).div_ceil(2);
        let slab = unsafe { Buffer::<u64>::uninitialized(self.runtime.pool().clone(), total)? };
        // Merges set bitmap bits while producing rows. Zero only this directory,
        // never the output columns. Small builds initialize it in their block.
        let bits = if small {
            None
        } else {
            Some(Scratch::<u64>::zeroed_in(
                self.runtime.pool().resource(),
                self.runtime.stream(),
                words,
            )?)
        };
        let base = slab.ptr();
        let view = RunView {
            rank: RankView {
                bits: bits.as_ref().map_or_else(
                    || unsafe { base.add(bits_at) as *const u64 },
                    |b| b.cu_deviceptr() as _,
                ),
                prefix: unsafe { base.add(prefix_at) as _ },
                offsets: unsafe { base.add(offsets_at) as _ },
                min_key,
                max_key,
            },
            columns: base as _,
            len: n as u64,
            count: unsafe { base.add(width) as _ },
            bucket_mask: 0,
        };
        let data = unsafe { base.add(columns_at) as *mut i64 };
        let temporary = unsafe { base.add(temporary_at) as *mut i64 };
        Ok((
            ResidentRun {
                view,
                bounds: bounds.to_vec(),
                _pointers: None,
                _storage: Storage::Packed { slab, bits },
            },
            data,
            temporary,
        ))
    }

    fn build(
        &self,
        chain: &Chain<'runtime>,
        inputs: &[ExactPrefix<'_>],
        bounds: &[Bounds],
    ) -> Result<ResidentRun<'runtime>> {
        let keys: Vec<&dyn ContiguousDeviceRange<i64>> =
            chain.order.iter().map(|&i| &inputs[i] as _).collect();
        let domains: Vec<_> = chain.order.iter().map(|&i| domain(bounds[i])).collect();
        let first = chain.order[0];
        let payloads: Vec<&dyn ContiguousDeviceRange<i64>> = inputs
            .iter()
            .enumerate()
            .filter_map(|(i, c)| (i != first).then_some(c as _))
            .collect();
        let run = Run::build_prefix_stream_ordered(
            self.runtime.context(),
            self.runtime.stream(),
            self.runtime.pool().resource(),
            &keys,
            &payloads,
            &domains,
            None,
        )
        .map_err(|e| failure("CQ_GPU_RUN_BUILD", e.to_string()))?;
        let mut payload = 0;
        let pointers = self.runtime.upload_descriptors((0..inputs.len()).map(|i| {
            if i == first {
                run.keys.cu_deviceptr()
            } else {
                let pointer = run.payloads[payload].cu_deviceptr();
                payload += 1;
                pointer
            }
        }))?;
        let view = RunView {
            rank: run.view(),
            columns: pointers.ptr() as _,
            len: inputs[0].len as u64,
            count: run.stats.cu_deviceptr() as _,
            bucket_mask: 0,
        };
        Ok(ResidentRun {
            view,
            bounds: bounds.to_vec(),
            _pointers: Some(pointers),
            _storage: Storage::Built(run),
        })
    }

    fn merge(
        &self,
        chain: &Chain<'runtime>,
        left: &ResidentRun<'runtime>,
        right: &ResidentRun<'runtime>,
    ) -> Result<ResidentRun<'runtime>> {
        let _nvtx = crate::nvtx::range(c"index/leveled/merge");
        let n = (left.view.len + right.view.len) as usize;
        let bounds: Vec<_> = left
            .bounds
            .iter()
            .zip(&right.bounds)
            .map(|(&a, &b)| union_bounds(a, b))
            .collect();
        let (run, data, _) =
            self.allocate_packed(n, chain.order.len(), chain.order[0], &bounds, false)?;
        let merge = self
            .kernels
            .prepare_merge_path(config(n.div_ceil(MERGE_TILE)))?;
        unsafe {
            self.kernels.merge_path(
                self.runtime.stream(),
                &merge,
                left.view,
                right.view,
                chain.device_order.ptr(),
                chain.order.len(),
                run.view,
                data,
            )?;
        }
        let prefix = self
            .kernels
            .prepare_leveled_directory_prefix(LaunchConfig1D::new(1, 256, 32))?;
        unsafe {
            self.kernels
                .leveled_directory_prefix(self.runtime.stream(), &prefix, run.view.rank)?;
        }
        let directory = self.kernels.prepare_directory_offsets(config(n))?;
        unsafe {
            self.kernels.directory_offsets(
                self.runtime.stream(),
                &directory,
                data.add(chain.order[0] * n),
                n,
                run.view.rank,
                run.view.rank.offsets as _,
            )?;
        }
        Ok(run)
    }
}

impl<'data, Row, C, Layout> LeveledDeviceIndex<'_, 'data, Row, C, Layout> {
    pub fn pool(&self) -> &Arc<Pool> {
        self.runtime.pool()
    }
    pub fn seed_dictionary<'short>(&self, dictionary: &mut Dictionary<'short>) -> Result<()>
    where
        'data: 'short,
    {
        dictionary.merge(&self.dictionary)
    }
    pub fn column_bounds(&self, slot: usize) -> Result<Bounds> {
        if slot >= self.chains[0].order.len() {
            return Err(failure(
                "CQ_GPU_KEY_COLUMN",
                "leveled column outside schema",
            ));
        }
        Ok(self.chains[0]
            .runs
            .iter()
            .flatten()
            .fold(Bounds::Empty, |bounds, run| {
                union_bounds(bounds, run.bounds[slot])
            }))
    }
}

impl<'data, 'source: 'data, Row: RowCodec<'data>, C, Layout> HostColumns<'data>
    for &LeveledDeviceIndex<'_, 'source, Row, C, Layout>
{
    type Row = Row;
    fn pool(self) -> Option<Arc<Pool>> {
        Some(self.runtime.pool().clone())
    }
    fn seed_dictionary(self, dictionary: &mut Dictionary<'data>) -> Result<()> {
        dictionary.merge(&self.dictionary)
    }
    fn bind(self, _: &mut Execution<'_, 'data>, _: &'static str, _: usize) -> Result<TableId> {
        Err(failure(
            "CQ_GPU_LEVELED_CONTIGUOUS_BIND",
            "maintained leveled hosts require snapshot reads, not a contiguous table binding",
        ))
    }
}

fn domain(bounds: Bounds) -> (i64, i64) {
    match bounds {
        Bounds::Empty => (0, -1),
        Bounds::Inclusive { min, max } => (min, max),
    }
}
fn union_bounds(a: Bounds, b: Bounds) -> Bounds {
    match (a, b) {
        (Bounds::Empty, b) => b,
        (a, Bounds::Empty) => a,
        (Bounds::Inclusive { min: a, max: b }, Bounds::Inclusive { min: c, max: d }) => {
            Bounds::Inclusive {
                min: a.min(c),
                max: b.max(d),
            }
        }
    }
}
fn level_bound(base: u64, ratio: u32, level: usize) -> u64 {
    (0..level).fold(base, |n, _| n.saturating_mul(ratio as u64))
}
fn config(items: usize) -> LaunchConfig1D {
    LaunchConfig1D::new(items.div_ceil(256).max(1) as u32, 256, 0)
}
struct ExactPrefix<'a> {
    buffer: &'a Buffer<i64>,
    len: usize,
}
unsafe impl ContiguousDeviceRange<i64> for ExactPrefix<'_> {
    fn len(&self) -> usize {
        self.len
    }
    fn cu_deviceptr(&self) -> cuda_core::sys::CUdeviceptr {
        self.buffer.cu_deviceptr()
    }
    fn context(&self) -> &Arc<CudaContext> {
        self.buffer.pool().context()
    }
    fn ensure_usable_on(
        &self,
        stream: &CudaStream,
    ) -> std::result::Result<(), rccl_memory::MemoryError> {
        self.buffer.ensure_usable_on(stream)
    }
}

// Each thread locates one diagonal cut, then serially emits a short contiguous
// tile. Left wins equal full tuples, so merge preserves a stable total order.
const MERGE_TILE: usize = 8;
const SMALL_ROWS: usize = 4096;
const SMALL_TILE: usize = 512;
#[cuda_device::cuda_module]
mod leveled_kernels {
    use super::{MERGE_TILE, SMALL_TILE};
    use crate::RunView;
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU64};
    use cuda_device::{DynamicSharedArray, device, kernel, launch_bounds, launch_contract, thread};
    use gpu_blocked_run::RankView;

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn leveled_decode_column(
        column: gpu_device_columns::ColumnView,
        n: usize,
        out: *mut i64,
    ) {
        let i = thread::index_1d().get();
        if i < n {
            unsafe {
                out.add(i)
                    .write(gpu_device_columns::decode(column, i as u64));
            }
        }
    }

    #[device]
    unsafe fn leveled_scan_bitmap(rank: RankView, scratch: *mut u32) {
        let words =
            ((rank.max_key as u64).wrapping_sub(rank.min_key as u64) as usize + 1).div_ceil(64);
        let lane = thread::threadIdx_x() as usize;
        let mut base = 0;
        let mut total = 0;
        while base < words {
            let word = base + lane;
            let count = if word < words {
                unsafe { rank.bits.add(word).read().count_ones() }
            } else {
                0
            };
            let (prefix, block_total) = gpu_tiles::scan(count, scratch);
            if word < words {
                unsafe {
                    (rank.prefix as *mut u32).add(word).write(total + prefix);
                }
            }
            total += block_total;
            base += 256;
        }
        thread::sync_threads();
    }

    #[device]
    unsafe fn leveled_row_greater(
        data: *const i64,
        n: usize,
        order: *const u64,
        width: usize,
        a: u32,
        b: u32,
    ) -> bool {
        if a == u32::MAX || b == u32::MAX {
            return a > b;
        }
        let mut k = 0;
        while k < width {
            let slot = unsafe { order.add(k).read() } as usize;
            let x = unsafe { data.add(slot * n + a as usize).read() };
            let y = unsafe { data.add(slot * n + b as usize).read() };
            if x != y {
                return x > y;
            }
            k += 1;
        }
        a > b
    }

    #[device]
    unsafe fn leveled_pack_row(
        decoded: *const i64,
        n: usize,
        row: usize,
        width: usize,
        packing: super::SmallPacking,
    ) -> u64 {
        let mut packed = row as u64;
        // Constant-index accesses are intentional: dynamic indexing would copy
        // the complete parameter array into every thread's local memory.
        macro_rules! column {
            ($slot:literal) => {
                if width > $slot && packing.shifts[$slot] < 64 {
                    let value = unsafe { decoded.add($slot * n + row).read() };
                    packed |= (value as u64).wrapping_sub(packing.bases[$slot] as u64)
                        << packing.shifts[$slot];
                }
            };
        }
        column!(0);
        column!(1);
        column!(2);
        column!(3);
        column!(4);
        column!(5);
        column!(6);
        column!(7);
        column!(8);
        column!(9);
        column!(10);
        column!(11);
        packed
    }

    /// Cooperative tile sort. Packed keys include global input row IDs, giving
    /// every tile the same stable total order used by the merge-path rank pass.
    #[device]
    unsafe fn leveled_sort_tile(
        source: *const gpu_device_columns::ColumnView,
        order: *const u64,
        width: usize,
        n: usize,
        base: usize,
        rows: usize,
        decoded: *mut i64,
        packing: super::SmallPacking,
        keys: *mut u64,
        permutation: *mut u32,
    ) {
        let lane = thread::threadIdx_x() as usize;
        let size = rows.next_power_of_two();
        let mut slot = 0;
        while slot < width {
            let column = unsafe { source.add(slot).read() };
            let mut i = lane;
            while i < rows {
                unsafe {
                    decoded
                        .add(slot * n + base + i)
                        .write(gpu_device_columns::decode(column, (base + i) as u64));
                }
                i += 256;
            }
            slot += 1;
        }
        let mut i = lane;
        while i < size {
            unsafe {
                permutation.add(i).write(if i < rows {
                    (base + i) as u32
                } else {
                    u32::MAX
                });
                if packing.enabled != 0 {
                    keys.add(i).write(if i < rows {
                        leveled_pack_row(decoded, n, base + i, width, packing)
                    } else {
                        u64::MAX
                    });
                }
            }
            i += 256;
        }
        thread::sync_threads();
        let mut group = 2;
        while group <= size {
            let mut distance = group / 2;
            while distance != 0 {
                let mut i = lane;
                while i < size {
                    let other = i ^ distance;
                    if i < other {
                        if packing.enabled != 0 {
                            let a = unsafe { keys.add(i).read() };
                            let b = unsafe { keys.add(other).read() };
                            if (a > b) == (i & group == 0) {
                                unsafe {
                                    keys.add(i).write(b);
                                    keys.add(other).write(a);
                                }
                            }
                        } else {
                            let a = unsafe { permutation.add(i).read() };
                            let b = unsafe { permutation.add(other).read() };
                            let greater =
                                unsafe { leveled_row_greater(decoded, n, order, width, a, b) };
                            if greater == (i & group == 0) {
                                unsafe {
                                    permutation.add(i).write(b);
                                    permutation.add(other).write(a);
                                }
                            }
                        }
                    }
                    i += 256;
                }
                thread::sync_threads();
                distance /= 2;
            }
            group *= 2;
        }
        if packing.enabled != 0 {
            let mask = (1u64 << packing.row_bits) - 1;
            let mut i = lane;
            while i < rows {
                unsafe {
                    permutation.add(i).write((keys.add(i).read() & mask) as u32);
                }
                i += 256;
            }
        }
        thread::sync_threads();
    }

    #[device]
    unsafe fn leveled_publish_run(run: RunView, data: *mut i64, width: usize) {
        let lane = thread::threadIdx_x() as usize;
        let mut slot = lane;
        while slot < width {
            unsafe {
                (run.columns as *mut *const i64)
                    .add(slot)
                    .write(data.add(slot * run.len as usize));
            }
            slot += 256;
        }
        if lane == 0 {
            unsafe {
                (run.count as *mut u32).write(run.len as u32);
            }
        }
    }

    #[device]
    unsafe fn leveled_finish_directory(
        run: RunView,
        data: *const i64,
        first: usize,
        scratch: *mut u32,
    ) {
        let lane = thread::threadIdx_x() as usize;
        let n = run.len as usize;
        let words = ((run.rank.max_key as u64).wrapping_sub(run.rank.min_key as u64) as usize + 1)
            .div_ceil(64);
        let mut word = lane;
        while word < words {
            unsafe {
                (run.rank.bits as *mut u64).add(word).write(0);
            }
            word += 256;
        }
        thread::sync_threads();
        let keys = unsafe { data.add(first * n) };
        let mut i = lane;
        while i < n {
            let key = unsafe { keys.add(i).read() };
            if i == 0 || unsafe { keys.add(i - 1).read() } != key {
                let bin = (key as u64).wrapping_sub(run.rank.min_key as u64) as usize;
                unsafe {
                    DeviceAtomicU64::from_ptr((run.rank.bits as *mut u64).add(bin >> 6))
                        .fetch_or(1u64 << (bin & 63), AtomicOrdering::Relaxed);
                }
            }
            i += 256;
        }
        thread::sync_threads();
        unsafe {
            leveled_scan_bitmap(run.rank, scratch);
        }
        let mut i = lane;
        while i < n {
            let key = unsafe { keys.add(i).read() };
            let bin = (key as u64).wrapping_sub(run.rank.min_key as u64) as usize;
            let mask = 1u64 << (bin & 63);
            let position = unsafe { run.rank.prefix.add(bin >> 6).read() }
                + (unsafe { run.rank.bits.add(bin >> 6).read() } & (mask - 1)).count_ones();
            if i == 0 || unsafe { keys.add(i - 1).read() } != key {
                unsafe {
                    (run.rank.offsets as *mut u32)
                        .add(position as usize)
                        .write(i as u32);
                }
            }
            if i + 1 == n {
                unsafe {
                    (run.rank.offsets as *mut u32)
                        .add(position as usize + 1)
                        .write(n as u32);
                }
            }
            i += 256;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (32, 6144))]
    pub unsafe fn leveled_small_build(
        source: *const gpu_device_columns::ColumnView,
        order: *const u64,
        width: usize,
        n: usize,
        run: RunView,
        data: *mut i64,
        decoded: *mut i64,
        packing: super::SmallPacking,
    ) {
        let lane = thread::threadIdx_x() as usize;
        let first = unsafe { order.read() } as usize;
        unsafe {
            leveled_publish_run(run, data, width);
        }
        if n == 1 {
            let key = unsafe { gpu_device_columns::decode(source.add(first).read(), 0) };
            let bin = (key as u64).wrapping_sub(run.rank.min_key as u64) as usize;
            let words = ((run.rank.max_key as u64).wrapping_sub(run.rank.min_key as u64) as usize
                + 1)
            .div_ceil(64);
            let mut word = lane;
            while word < words {
                unsafe {
                    (run.rank.bits as *mut u64)
                        .add(word)
                        .write(if word == bin >> 6 {
                            1u64 << (bin & 63)
                        } else {
                            0
                        });
                    (run.rank.prefix as *mut u32)
                        .add(word)
                        .write((word > bin >> 6) as u32);
                }
                word += 256;
            }
            let mut slot = lane;
            while slot < width {
                unsafe {
                    data.add(slot)
                        .write(gpu_device_columns::decode(source.add(slot).read(), 0));
                }
                slot += 256;
            }
            if lane == 0 {
                unsafe {
                    (run.rank.offsets as *mut u32).write(0);
                    (run.rank.offsets as *mut u32).add(1).write(1);
                }
            }
            return;
        }
        let shared_keys = DynamicSharedArray::<u64>::get_raw() as *mut u64;
        let permutation = if packing.enabled != 0 {
            unsafe { shared_keys.add(n.next_power_of_two()) as *mut u32 }
        } else {
            shared_keys as *mut u32
        };
        unsafe {
            leveled_sort_tile(
                source,
                order,
                width,
                n,
                0,
                n,
                decoded,
                packing,
                shared_keys,
                permutation,
            );
        }
        let mut i = lane;
        while i < n {
            let row = unsafe { permutation.add(i).read() } as usize;
            let mut slot = 0;
            while slot < width {
                unsafe {
                    data.add(slot * n + i)
                        .write(decoded.add(slot * n + row).read());
                }
                slot += 1;
            }
            i += 256;
        }
        thread::sync_threads();
        unsafe {
            leveled_finish_directory(run, data, first, permutation);
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (2048, 6144))]
    pub unsafe fn leveled_sort_tiles(
        source: *const gpu_device_columns::ColumnView,
        order: *const u64,
        width: usize,
        n: usize,
        run: RunView,
        data: *mut i64,
        decoded: *mut i64,
        packing: super::SmallPacking,
        sorted_keys: *mut u64,
        sorted_rows: *mut u32,
    ) {
        let base = thread::blockIdx_x() as usize * SMALL_TILE;
        let rows = (n - base).min(SMALL_TILE);
        let shared_keys = DynamicSharedArray::<u64>::get_raw() as *mut u64;
        let permutation = if packing.enabled != 0 {
            unsafe { shared_keys.add(SMALL_TILE) as *mut u32 }
        } else {
            shared_keys as *mut u32
        };
        if thread::blockIdx_x() == 0 {
            unsafe {
                leveled_publish_run(run, data, width);
            }
        }
        unsafe {
            leveled_sort_tile(
                source,
                order,
                width,
                n,
                base,
                rows,
                decoded,
                packing,
                shared_keys,
                permutation,
            );
        }
        let mut i = thread::threadIdx_x() as usize;
        while i < rows {
            unsafe {
                sorted_rows.add(base + i).write(permutation.add(i).read());
                if packing.enabled != 0 {
                    sorted_keys.add(base + i).write(shared_keys.add(i).read());
                }
            }
            i += 256;
        }
    }

    /// One multiway merge-path rank pass: a row's global rank is its local rank
    /// plus its lower-bound cut in every other sorted tile. Stable row IDs make
    /// all ranks distinct, so scatter needs neither atomics nor a second pass.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn leveled_merge_tiles(
        decoded: *const i64,
        keys: *const u64,
        rows: *const u32,
        data: *mut i64,
        n: usize,
        width: usize,
        order: *const u64,
        packed: bool,
    ) {
        let i = thread::index_1d().get();
        if i >= n {
            return;
        }
        let own = i / SMALL_TILE;
        let row = unsafe { rows.add(i).read() };
        let key = if packed {
            unsafe { keys.add(i).read() }
        } else {
            0
        };
        let mut rank = i % SMALL_TILE;
        let mut tile = 0;
        while tile < n.div_ceil(SMALL_TILE) {
            if tile != own {
                let base = tile * SMALL_TILE;
                let mut lo = base;
                let mut hi = (base + SMALL_TILE).min(n);
                while lo < hi {
                    let mid = (lo + hi) / 2;
                    let less = if packed {
                        (unsafe { keys.add(mid).read() }) < key
                    } else {
                        unsafe {
                            leveled_row_greater(decoded, n, order, width, row, rows.add(mid).read())
                        }
                    };
                    if less {
                        lo = mid + 1;
                    } else {
                        hi = mid;
                    }
                }
                rank += lo - base;
            }
            tile += 1;
        }
        let mut slot = 0;
        while slot < width {
            unsafe {
                data.add(slot * n + rank)
                    .write(decoded.add(slot * n + row as usize).read());
            }
            slot += 1;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (32, 32))]
    pub unsafe fn leveled_finish_small(run: RunView, data: *const i64, first: usize) {
        let scratch = DynamicSharedArray::<u32>::get_raw() as *mut u32;
        unsafe {
            leveled_finish_directory(run, data, first, scratch);
        }
    }
    #[device]
    #[inline(always)]
    unsafe fn less(
        a: RunView,
        i: usize,
        b: RunView,
        j: usize,
        order: *const u64,
        width: usize,
    ) -> bool {
        let mut k = 0;
        while k < width {
            let slot = unsafe { order.add(k).read() } as usize;
            let x = unsafe { a.columns.add(slot).read().add(i).read() };
            let y = unsafe { b.columns.add(slot).read().add(j).read() };
            if x != y {
                return x < y;
            }
            k += 1;
        }
        false
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn merge_path(
        a: RunView,
        b: RunView,
        order: *const u64,
        width: usize,
        out: RunView,
        data: *mut i64,
    ) {
        let diagonal = thread::index_1d().get() * MERGE_TILE;
        let an = a.len as usize;
        let bn = b.len as usize;
        let n = an + bn;
        let tid = thread::index_1d().get();
        let mut slot = tid;
        while slot < width {
            unsafe {
                (out.columns as *mut *const i64)
                    .add(slot)
                    .write(data.add(slot * n));
            }
            slot += thread::gridDim_x() as usize * 256;
        }
        if tid == 0 {
            unsafe {
                (out.count as *mut u32).write(n as u32);
            }
        }
        if diagonal >= n {
            return;
        }
        let mut lo = diagonal.saturating_sub(bn);
        let mut hi = diagonal.min(an);
        while lo < hi {
            let i = (lo + hi) / 2;
            let j = diagonal - i;
            // B[j-1] >= A[i] means this cut consumed too few A rows.
            if i < an && j > 0 && !unsafe { less(b, j - 1, a, i, order, width) } {
                lo = i + 1;
            } else {
                hi = i;
            }
        }
        let mut i = lo;
        let mut j = diagonal - i;
        let first = unsafe { order.read() } as usize;
        let mut previous = i64::MIN;
        if i != 0 {
            previous = unsafe { a.columns.add(first).read().add(i - 1).read() };
        }
        if j != 0 {
            previous = previous.max(unsafe { b.columns.add(first).read().add(j - 1).read() });
        }
        let end = (diagonal + MERGE_TILE).min(n);
        let mut dst = diagonal;
        while dst < end {
            let take_a = i < an && (j == bn || !unsafe { less(b, j, a, i, order, width) });
            let (source, row) = if take_a { (a, i) } else { (b, j) };
            let mut slot = 0;
            while slot < width {
                unsafe {
                    data.add(slot * n + dst)
                        .write(source.columns.add(slot).read().add(row).read());
                }
                slot += 1;
            }
            let key = unsafe { source.columns.add(first).read().add(row).read() };
            if dst == 0 || key != previous {
                let bin = (key as u64).wrapping_sub(out.rank.min_key as u64) as usize;
                unsafe {
                    DeviceAtomicU64::from_ptr((out.rank.bits as *mut u64).add(bin >> 6))
                        .fetch_or(1u64 << (bin & 63), AtomicOrdering::Relaxed);
                }
            }
            previous = key;
            if take_a {
                i += 1;
            } else {
                j += 1;
            }
            dst += 1;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (32, 32))]
    pub unsafe fn leveled_directory_prefix(rank: RankView) {
        let scratch = DynamicSharedArray::<u32>::get_raw() as *mut u32;
        unsafe {
            leveled_scan_bitmap(rank, scratch);
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn directory_offsets(keys: *const i64, n: usize, rank: RankView, offsets: *mut u32) {
        let i = thread::index_1d().get();
        if i < n {
            let key = unsafe { keys.add(i).read() };
            let bin = (key as u64).wrapping_sub(rank.min_key as u64) as usize;
            let mask = 1u64 << (bin & 63);
            let position = unsafe { rank.prefix.add(bin >> 6).read() }
                + (unsafe { rank.bits.add(bin >> 6).read() } & (mask - 1)).count_ones();
            if i == 0 || unsafe { keys.add(i - 1).read() } != key {
                unsafe {
                    offsets.add(position as usize).write(i as u32);
                }
            }
            if i + 1 == n {
                unsafe {
                    offsets.add(position as usize + 1).write(n as u32);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_saturate_without_wrapping() {
        assert_eq!(level_bound(1, 4, 0), 1);
        assert_eq!(level_bound(1, 4, 10), 1_048_576);
        assert_eq!(level_bound(u32::MAX as u64, u32::MAX, 15), u64::MAX);
    }
    #[test]
    fn million_row_base_and_single_delta_occupy_different_levels() {
        let level = |n| {
            (0..MAX_LEVELS)
                .find(|&i| n <= level_bound(1, 4, i))
                .unwrap()
        };
        assert_ne!(level(1_000_000), level(1));
    }
    #[test]
    fn packing_preserves_layout_and_never_truncates() {
        let bounds = [
            Bounds::Inclusive { min: -8, max: 7 },
            Bounds::Inclusive { min: 100, max: 131 },
            Bounds::Inclusive { min: 0, max: 4095 },
        ];
        let packed = SmallPacking::new(4096, &[1, 0, 2], &bounds);
        assert_eq!(packed.enabled, 1);
        assert_eq!(packed.row_bits, 12);
        assert_eq!(&packed.shifts[..3], &[24, 28, 12]);
        assert_eq!(packed.end_bits, 33);
        let wide = [Bounds::Inclusive { min: i64::MIN, max: i64::MAX }];
        assert_eq!(SmallPacking::new(2, &[0], &wide).enabled, 0);
        assert_eq!(SmallPacking::new(1, &[0], &wide).enabled, 0);
    }
}
