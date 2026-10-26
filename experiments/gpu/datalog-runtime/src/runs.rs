use crate::{Result, Word};
use cuda_core::{CudaEvent, DeviceBuffer, DeviceCopy, LaunchConfig1D};
use gpu_device_columns::{Buffer, Pool};
use rccl_cub::{DeviceSliceRead, DeviceSliceWrite};
use std::{cmp::Reverse, collections::BinaryHeap, sync::Arc};

/// A borrowed kernel ABI. Its owner must outlive all stream-ordered reads.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct RunView<W: Word> {
    pub ptr: *const W,
    pub len: usize,
}
// SAFETY: this C-layout descriptor contains only a device pointer and length.
unsafe impl<W: Word> DeviceCopy for RunView<W> {}

/// Find the half-open index interval for the inclusive value interval [lo, hi].
/// This also handles the maximum word without overflowing `hi + 1`.
/// # Safety
/// `view` must describe initialized, sorted words readable on this device.
#[inline(always)]
pub unsafe fn range<W: Word>(view: RunView<W>, lo: W, hi: W) -> (usize, usize) {
    let mut begin = 0;
    let mut end = view.len;
    while begin < end {
        let mid = begin + (end - begin) / 2;
        if unsafe { &*view.ptr.add(mid) }.compare(&lo).is_lt() { begin = mid + 1; } else { end = mid; }
    }
    let first = begin;
    end = view.len;
    while begin < end {
        let mid = begin + (end - begin) / 2;
        if !unsafe { &*view.ptr.add(mid) }.compare(&hi).is_gt() { begin = mid + 1; } else { end = mid; }
    }
    (first, begin)
}

/// Cheap shared ownership of an immutable sorted, unique initialized prefix.
#[derive(Clone)]
pub struct Run<W: Word> {
    buffer: Arc<Buffer<W>>,
    rows: usize,
}
impl<W: Word> Run<W> {
    /// # Safety
    /// The prefix must be initialized, sorted and unique on its pool's stream.
    /// No alias may mutate its words for the lifetime of this run or its clones.
    pub unsafe fn from_sorted(buffer: Buffer<W>, rows: usize) -> Result<Self> {
        if rows > buffer.len() { return Err("run length exceeds buffer capacity".into()); }
        // An empty immutable prefix need not keep its entire rejected candidate
        // allocation alive. Releasing it is stream ordered and copies no rows.
        let buffer = if rows == 0 && !buffer.is_empty() {
            let pool = buffer.pool().clone();
            drop(buffer);
            unsafe { Buffer::uninitialized(pool, 0)? }
        } else { buffer };
        let buffer = Arc::new(buffer);
        // SAFETY: the caller publishes an immutable initialized prefix.
        let buffer = if rows == buffer.len() { buffer } else {
            Arc::new(unsafe { Buffer::shared_slice(buffer, 0, rows)? })
        };
        Ok(Self { buffer, rows })
    }
    pub fn len(&self) -> usize { self.rows }
    pub fn is_empty(&self) -> bool { self.rows == 0 }
    /// Initialized packed-row bytes, excluding retained spare capacity.
    pub fn live_bytes(&self) -> usize { self.rows * std::mem::size_of::<W>() }
    /// Full backing allocation retained by this run, including a sliced suffix.
    /// Clones and layouts may retain the same allocation; do not sum aliases.
    pub fn allocated_bytes(&self) -> usize { self.buffer.allocated_bytes() }
    pub fn reserved_capacity(&self) -> usize {
        self.allocated_bytes() / std::mem::size_of::<W>()
    }
    pub fn buffer(&self) -> &Buffer<W> { &self.buffer }
    pub fn view(&self) -> RunView<W> { RunView { ptr: self.buffer.ptr(), len: self.rows } }
}
// SAFETY: the retained immutable buffer owns the reported initialized prefix.
unsafe impl<W: Word> cuda_host::KernelSliceArg for Run<W> {
    type Elem = W;
    fn cu_deviceptr(&self) -> cuda_core::sys::CUdeviceptr { self.buffer.cu_deviceptr() }
    fn len(&self) -> usize { self.rows }
}

// Only unpublished output storage implements the writable kernel ABI.
struct Output<W: Word>(Buffer<W>);
// SAFETY: this affine wrapper owns the entire allocation.
unsafe impl<W: Word> cuda_host::KernelSliceArg for Output<W> {
    type Elem = W;
    fn cu_deviceptr(&self) -> cuda_core::sys::CUdeviceptr { self.0.cu_deviceptr() }
    fn len(&self) -> usize { self.0.len() }
}
// SAFETY: mutable access is exclusive and output is unpublished until launch.
unsafe impl<W: Word> cuda_host::KernelSliceArgMut for Output<W> {}

#[derive(Clone, Copy, Debug, Default)]
pub struct AdmissionTimings {
    pub sort_unique_ms: f32,
    pub anti_join_ms: f32,
    pub merge_ms: f32,
}
struct Timing { events: [CudaEvent; 4] }
impl Timing {
    fn new(pool: &Pool) -> Result<Self> {
        let event = || pool.context().new_event(Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT));
        Ok(Self { events: [event()?, event()?, event()?, event()?] })
    }
    fn record(&self, phase: usize, pool: &Pool) -> Result<()> { Ok(self.events[phase].record(pool.stream())?) }
    fn elapsed(&self) -> Result<AdmissionTimings> {
        Ok(AdmissionTimings {
            sort_unique_ms: self.events[0].elapsed_ms(&self.events[1])?,
            anti_join_ms: self.events[1].elapsed_ms(&self.events[2])?,
            merge_ms: self.events[2].elapsed_ms(&self.events[3])?,
        })
    }
}

/// Formal rule DL-DeviceRuns: admission subtracts all accepted immutable runs;
/// each appended delta is unique and disjoint, and merging preserves their union.
///
/// Each nonempty admission downloads only the candidate unique count and, when
/// accepted state is nonempty, the fresh count. There are no row readbacks.
/// Empty batches need neither download. `to_host` and opt-in `last_timings` are
/// explicit diagnostic synchronization paths, never used by admission.
pub struct DeviceRuns<W: Word, S = ()> {
    pool: Arc<Pool>,
    accepted: Vec<Run<W>>,
    rows: usize,
    module: run_kernels::LoadedModule,
    timing_enabled: bool,
    timing: Option<Timing>,
    schema: std::marker::PhantomData<fn() -> S>,
}
impl<W: Word, S> DeviceRuns<W, S> {
    pub fn new(pool: Arc<Pool>) -> Result<Self> {
        // SAFETY: module is loaded in, and retained with, the pool's context.
        let module = unsafe { run_kernels::load(pool.context())? };
        Ok(Self { pool, accepted: Vec::new(), rows: 0, module, timing_enabled: false, timing: None,
            schema: std::marker::PhantomData })
    }
    pub fn len(&self) -> usize { self.rows }
    pub fn is_empty(&self) -> bool { self.rows == 0 }
    /// Initialized bytes in the accepted set, excluding retained spare capacity.
    pub fn live_bytes(&self) -> usize { self.rows * std::mem::size_of::<W>() }
    /// Backing bytes retained by current accepted runs. Published snapshots may
    /// alias these allocations or retain older runs not counted here.
    pub fn allocated_bytes(&self) -> usize {
        self.accepted.iter().map(Run::allocated_bytes).sum()
    }
    pub fn reserved_capacity(&self) -> usize {
        self.accepted.iter().map(Run::reserved_capacity).sum()
    }
    pub fn retained_buffers(&self) -> usize { self.accepted.len() }
    pub fn runs(&self) -> Vec<RunView<W>> { self.accepted.iter().map(Run::view).collect() }
    /// Retain a snapshot of accepted immutable runs, independent of later merges.
    pub fn owned_runs(&self) -> Vec<Run<W>> { self.accepted.clone() }
    pub fn enable_timing(&mut self, enabled: bool) { self.timing_enabled = enabled; }
    /// Explicitly synchronize the last profiled admission and read CUDA event times.
    pub fn last_timings(&self) -> Result<AdmissionTimings> {
        self.timing.as_ref().ok_or_else(|| "admission timing is not enabled".into()).and_then(Timing::elapsed)
    }
    pub fn admit(&mut self, candidates: &Buffer<W>, rows: usize) -> Result<Run<W>> {
        if !Arc::ptr_eq(&self.pool, candidates.pool()) { return Err("candidate pool differs from accepted state".into()); }
        if rows > candidates.len() || rows > u32::MAX as usize { return Err("candidate live length exceeds capacity or u32 domain".into()); }
        self.timing = if self.timing_enabled { Some(Timing::new(&self.pool)?) } else { None };
        self.mark(0)?;
        let unique = self.sort_unique(candidates, rows)?;
        self.mark(1)?;
        let fresh = if self.accepted.is_empty() || unique.is_empty() { unique } else {
            self.subtract(&unique)?
        };
        self.mark(2)?;
        // SAFETY: subtraction checked every accepted run; unique is sorted.
        unsafe { self.append_sorted(fresh.clone())? };
        self.mark(3)?;
        Ok(fresh)
    }
    fn mark(&self, phase: usize) -> Result<()> {
        if let Some(timing) = &self.timing { timing.record(phase, &self.pool)?; }
        Ok(())
    }
    /// Canonicalize a buffer without changing accepted state, for layout deltas.
    /// Only the unique count is downloaded. The input remains unchanged.
    pub fn sort_unique(&self, input: &Buffer<W>, rows: usize) -> Result<Run<W>> {
        if !Arc::ptr_eq(&self.pool, input.pool()) || rows > input.len() || rows > u32::MAX as usize {
            return Err("invalid sort input pool or live length".into());
        }
        // SAFETY: kernels initialize the returned prefix before publication.
        let output = unsafe { Buffer::<W>::uninitialized(self.pool.clone(), rows)? };
        if rows == 0 { return unsafe { Run::from_sorted(output, 0) }; }
        let cub = rccl_cub::cuda::Program::load(self.pool.context())?;
        let launch = cub.on(self.pool.stream())?;
        let mut count = Native::<u64>::new(&self.pool, 1)?;
        let mut compact = rccl_cub::thrust_integration::compaction_workspace_in(self.pool.resource(), &launch, rows)?;
        if W::BITS == 64 {
            let mut first = Native::<u64>::new(&self.pool, rows)?;
            let mut alternate = Native::<u64>::new(&self.pool, rows)?;
            // SAFETY: input is initialized; the destination is disjoint and same-width.
            unsafe { cuda_core::simt::memory::memcpy_dtod_async(first.get().cu_deviceptr(), input.cu_deviceptr(), rows * 8, self.pool.stream().cu_stream())?; }
            let mut controls = RadixScratch {
                workspace: rccl_cub::device_radix_sort_keys_buffer_workspace::<u64>(&launch, rows)?,
                pool: self.pool.clone(),
            };
            let mut buffers = rccl_cub::DoubleBuffer::new(first.get_mut(), alternate.get_mut()).ok_or("unequal sort buffers")?;
            rccl_cub::device_radix_sort_keys_buffer(&launch, &mut buffers, 0..64, &mut controls.workspace)?;
            rccl_cub::thrust_integration::device_unique_copy_by(
                &launch, DeviceSliceRead::new(buffers.current().cu_deviceptr() as *const u64),
                DeviceSliceWrite::new(output.ptr().cast::<u64>()), count.get_mut(),
                |a: u64, b: u64| a == b, rows, &mut compact,
            )?;
        } else {
            let mut first = Native::<u64>::new(&self.pool, rows)?;
            let mut alternate = Native::<u64>::new(&self.pool, rows)?;
            let mut indices = Native::<u32>::new(&self.pool, rows)?;
            let mut alternate_indices = Native::<u32>::new(&self.pool, rows)?;
            let mut controls = RadixScratch {
                workspace: rccl_cub::device_radix_sort_pairs_buffer_workspace::<u64, u32>(&launch, rows)?,
                pool: self.pool.clone(),
            };
            let mut buffers = rccl_cub::DoubleBuffer::paired(
                (first.get_mut(), indices.get_mut()),
                (alternate.get_mut(), alternate_indices.get_mut()),
            ).ok_or("unequal lane radix buffers")?;
            let config = LaunchConfig1D::new(rows.div_ceil(256).clamp(1, 2272) as u32, 256, 0);
            let prepared = self.module.prepare_gather_lane::<W>(config)?;
            // Stable least-significant-lane passes retain full-row order.
            for lane in 0..W::BITS / 64 {
                let (values, order) = buffers.current_mut();
                self.module.gather_lane::<W>(
                    self.pool.stream(), &prepared, input.ptr(), rows as u32, lane,
                    values.cu_deviceptr() as *mut u64,
                    order.cu_deviceptr() as *mut u32,
                )?;
                rccl_cub::device_radix_sort_pairs_buffer(
                    &launch, &mut buffers, 0..64, &mut controls.workspace,
                )?;
            }
            let source = input.ptr();
            let words = rccl_cub::DeviceMap::new(
                DeviceSliceRead::new(buffers.current().1.cu_deviceptr() as *const u32),
                move |row: u32| unsafe { *source.add(row as usize) },
            );
            rccl_cub::thrust_integration::device_unique_copy_by(
                &launch, words, DeviceSliceWrite::new(output.ptr()),
                count.get_mut(), |a: W, b: W| a.compare(&b).is_eq(), rows, &mut compact,
            )?;
        }
        let rows = read_count(&self.pool, count.get())?;
        // SAFETY: device unique wrote exactly this sorted unique prefix.
        unsafe { Run::from_sorted(output, rows) }
    }
    fn subtract(&self, unique: &Run<W>) -> Result<Run<W>> {
        let rows = unique.len();
        let descriptors = Buffer::from_host_async(self.pool.clone(), &self.runs())?;
        // SAFETY: the flag kernel and stable select initialize these outputs.
        let flags = unsafe { Buffer::<u32>::uninitialized(self.pool.clone(), rows)? };
        let output = unsafe { Buffer::<W>::uninitialized(self.pool.clone(), rows)? };
        let config = LaunchConfig1D::new(rows.div_ceil(256).clamp(1, 2272) as u32, 256, 0);
        self.module.absent::<W>(
            self.pool.stream(), &self.module.prepare_absent::<W>(config)?, unique.buffer.ptr(),
            rows as u64, descriptors.ptr(), self.accepted.len() as u32, flags.ptr(),
        )?;
        let cub = rccl_cub::cuda::Program::load(self.pool.context())?;
        let launch = cub.on(self.pool.stream())?;
        let mut count = Native::<u64>::new(&self.pool, 1)?;
        let mut workspace = rccl_cub::thrust_integration::compaction_workspace_in(self.pool.resource(), &launch, rows)?;
        rccl_cub::thrust_integration::device_select_flagged(
            &launch, DeviceSliceRead::new(unique.buffer.ptr()), DeviceSliceRead::new(flags.ptr()),
            DeviceSliceWrite::new(output.ptr()), count.get_mut(), rows, &mut workspace,
        )?;
        let fresh = read_count(&self.pool, count.get())?;
        // SAFETY: stable selection preserves sorted uniqueness of the input.
        unsafe { Run::from_sorted(output, fresh) }
    }
    /// Append a pre-canonicalized layout delta and apply ratio-four merging.
    /// Wide rows use tiled merge-path partitioning, without native RCCL's
    /// 16-byte key-layout limit or a full-size auxiliary row allocation.
    /// # Safety
    /// The run must be disjoint from every accepted run. In particular callers
    /// may sort a bijective permutation of an admitted delta for another layout.
    pub unsafe fn append_sorted(&mut self, run: Run<W>) -> Result<()> {
        if !Arc::ptr_eq(&self.pool, run.buffer.pool()) { return Err("layout run pool differs".into()); }
        if run.is_empty() { return Ok(()); }
        let rows = self.rows.checked_add(run.len()).ok_or("accepted row count overflow")?;
        // Prepare the complete merge before mutating state: failures leave it unchanged.
        let mut merged = run;
        let mut keep = self.accepted.len();
        while keep > 0 && self.accepted[keep - 1].len() <= merged.len().saturating_mul(4) {
            merged = self.merge(&self.accepted[keep - 1], &merged)?;
            keep -= 1;
        }
        self.accepted.truncate(keep);
        self.accepted.push(merged);
        self.rows = rows;
        Ok(())
    }
    fn merge(&self, first: &Run<W>, second: &Run<W>) -> Result<Run<W>> {
        let rows = first.len().checked_add(second.len()).ok_or("merged length overflow")?;
        // SAFETY: merge writes every output element before publication.
        let mut output = Output(unsafe { Buffer::uninitialized(self.pool.clone(), rows)? });
        if W::BITS > 128 {
            let tiles = rows.div_ceil(run_kernels::MERGE_TILE_ROWS);
            let config = LaunchConfig1D::new(tiles.div_ceil(256).clamp(1, 2272) as u32, 256, 0);
            self.module.merge_wide::<W>(
                self.pool.stream(), &self.module.prepare_merge_wide::<W>(config)?,
                first.view(), second.view(), output.0.ptr(),
            )?;
            // SAFETY: disjoint merge-path tiles initialize the sorted union.
            return unsafe { Run::from_sorted(output.0, rows) };
        }
        let cub = rccl_cub::cuda::Program::load(self.pool.context())?;
        let launch = cub.on(self.pool.stream())?;
        let partitions = rccl_cub::thrust_integration::device_merge_partition_count::<W>(first.len(), second.len())?;
        let mut partitions = Native::<usize>::new(&self.pool, partitions)?;
        let mut workspace = rccl_cub::DeviceMergeWorkspace::__from_partitions(partitions.get_mut());
        rccl_cub::device_merge_keys(&launch, first, second, &mut workspace, &mut output, |a: &W, b: &W| a.compare(b))?;
        // SAFETY: merging disjoint sorted unique runs produces a sorted unique run.
        unsafe { Run::from_sorted(output.0, rows) }
    }
    /// Explicit diagnostic row download, followed by a host k-way sorted merge.
    pub fn to_host(&self) -> Result<Vec<W>> {
        let runs = self.accepted.iter().map(|run| run.buffer.to_host_prefix(run.len())).collect::<Result<Vec<_>>>()?;
        let mut heap = BinaryHeap::new();
        for (run, values) in runs.iter().enumerate() {
            if let Some(&word) = values.first() { heap.push(Reverse((word, run, 0))); }
        }
        let mut output = Vec::with_capacity(self.rows);
        while let Some(Reverse((word, run, offset))) = heap.pop() {
            output.push(word);
            if let Some(&next) = runs[run].get(offset + 1) { heap.push(Reverse((next, run, offset + 1))); }
        }
        Ok(output)
    }
}

// Native RCCL count/control buffers need stream-ordered destruction too.
struct Native<T: DeviceCopy> { buffer: Option<DeviceBuffer<T>>, pool: Arc<Pool> }
impl<T: DeviceCopy> Native<T> {
    fn new(pool: &Arc<Pool>, rows: usize) -> Result<Self> {
        // SAFETY: every consumer initializes scratch before reading it.
        Ok(Self { buffer: Some(unsafe { DeviceBuffer::uninitialized_async(pool.stream(), rows)? }), pool: pool.clone() })
    }
    fn get(&self) -> &DeviceBuffer<T> { self.buffer.as_ref().unwrap() }
    fn get_mut(&mut self) -> &mut DeviceBuffer<T> { self.buffer.as_mut().unwrap() }
}
impl<T: DeviceCopy> Drop for Native<T> {
    fn drop(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            // SAFETY: all accesses were enqueued on this pool stream.
            if let Err(error) = unsafe { buffer.drop_async(self.pool.stream()) } { self.pool.context().record_err::<()>(Err(error)); }
        }
    }
}
struct RadixScratch { workspace: rccl_cub::DeviceRadixSortBufferWorkspace, pool: Arc<Pool> }
impl Drop for RadixScratch {
    fn drop(&mut self) {
        fn release<T: DeviceCopy>(pool: &Pool, buffer: &mut DeviceBuffer<T>) {
            // SAFETY: zero-length replacement owns no allocation; old uses are stream ordered.
            let empty = unsafe { DeviceBuffer::uninitialized_async(pool.stream(), 0) }.expect("empty allocation");
            let old = std::mem::replace(buffer, empty);
            if let Err(error) = unsafe { old.drop_async(pool.stream()) } { pool.context().record_err::<()>(Err(error)); }
        }
        let (bins, lookback, counters) = self.workspace.__parts_mut();
        release(&self.pool, bins); release(&self.pool, lookback); release(&self.pool, counters);
    }
}
fn read_count(pool: &Pool, buffer: &DeviceBuffer<u64>) -> Result<usize> {
    let mut count = 0_u64;
    // SAFETY: preceding compaction initialized one u64; stack storage survives synchronization.
    unsafe { cuda_core::simt::memory::memcpy_dtoh_async(&mut count as *mut u64, buffer.cu_deviceptr(), 8, pool.stream().cu_stream())?; }
    crate::record_readback();
    pool.stream().synchronize()?;
    Ok(usize::try_from(count)?)
}

#[cuda_host::cuda_module]
mod run_kernels {
    use crate::{Word, runs::RunView};
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};
    pub const MERGE_TILE_ROWS: usize = 32;

    /// Each thread finds one merge-path diagonal, then merges a bounded tile.
    /// Tiles partition the output, so rows need neither atomics nor scratch.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn merge_wide<W: Word>(first: RunView<W>, second: RunView<W>, output: *mut W) {
        let rows = first.len + second.len;
        let mut tile = thread::index_1d().get() as usize;
        let stride = thread::gridDim_x() as usize * 256;
        let tiles = rows.div_ceil(MERGE_TILE_ROWS);
        while tile < tiles {
            let start = tile * MERGE_TILE_ROWS;
            let end = start + (rows - start).min(MERGE_TILE_ROWS);
            let mut lo = start.saturating_sub(second.len);
            let mut hi = start.min(first.len);
            // Find how many first-run rows precede this output diagonal.
            // Equal keys prefer the first run, matching the merge below.
            while lo < hi {
                let mid = lo + (hi - lo) / 2;
                let other = start - mid;
                if other > 0 && mid < first.len
                    && !unsafe { &*first.ptr.add(mid) }
                        .compare(unsafe { &*second.ptr.add(other - 1) }).is_gt()
                {
                    lo = mid + 1;
                } else {
                    hi = mid;
                }
            }
            let mut a = lo;
            let mut b = start - lo;
            let mut row = start;
            while row < end {
                let take_first = a < first.len && (b == second.len
                    || !unsafe { &*first.ptr.add(a) }
                        .compare(unsafe { &*second.ptr.add(b) }).is_gt());
                unsafe {
                    if take_first {
                        *output.add(row) = *first.ptr.add(a);
                        a += 1;
                    } else {
                        *output.add(row) = *second.ptr.add(b);
                        b += 1;
                    }
                }
                row += 1;
            }
            tile += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn gather_lane<W: Word>(
        input: *const W, rows: u32, lane: u32, values: *mut u64, order: *mut u32,
    ) {
        let mut row = thread::index_1d().get() as usize;
        let stride = thread::gridDim_x() as usize * 256;
        let lanes = W::BITS / 64;
        let physical = if W::MS_FIRST { lanes - 1 - lane } else { lane };
        while row < rows as usize {
            let source = if lane == 0 { row as u32 } else { unsafe { *order.add(row) } };
            unsafe {
                *order.add(row) = source;
                *values.add(row) = *input.cast::<u64>().add(source as usize * lanes as usize + physical as usize);
            }
            row += stride;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn absent<W: Word>(input: *const W, rows: u64, runs: *const RunView<W>, count: u32, flags: *mut u32) {
        let mut row = thread::index_1d().get() as usize;
        let stride = thread::gridDim_x() as usize * 256;
        while row < rows as usize {
            let value = unsafe { *input.add(row) };
            let mut absent = true;
            let mut run = 0;
            while run < count as usize && absent {
                let view = unsafe { &*runs.add(run) };
                let mut begin = 0;
                let mut end = view.len;
                while begin < end {
                    let mid = begin + (end - begin) / 2;
                    if unsafe { &*view.ptr.add(mid) }.compare(&value).is_lt() { begin = mid + 1; } else { end = mid; }
                }
                absent = begin == view.len || !unsafe { &*view.ptr.add(begin) }.compare(&value).is_eq();
                run += 1;
            }
            unsafe { *flags.add(row) = absent as u32; }
            row += stride;
        }
    }
}
