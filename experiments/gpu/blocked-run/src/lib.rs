//! GPU-built sorted, clustered runs with a dense rank directory.
//!
//! Construction scans only per-bitmap-word row totals and presence popcounts.
//! Bins order original row IDs, or full prefix keys with row-ID tie-breaking.
//! Bounds are supplied from input metadata; active row counts and all stage
//! totals stay on device. `read_stats` is the single build-completion readback.
//! Inputs and consumers must be ordered on the supplied stream.
//! Kernel code is retained in a thread-local, context-identity-checked cache.
//! The cache owns no inputs, bounds, counts, runs or algorithm scratch.
//!
//! Buffer lengths are capacities: stats.rows, stats.unique and their fence
//! count delimit initialized logical results. Unused output tails are zero.
//! `prefix[w]` counts set bits strictly before bitmap word `w`.
//! Empty domains have min_key > max_key and offsets `[0]`.
//! Dense directories accept at most 2^32 key positions (including missing
//! keys), and row counts must fit in u32. Sparse wider domains are rejected,
//! not silently converted to a different lookup representation.
//! Prefix runs bin the first key and heap-order each bin by the remaining keys,
//! then original row ID. A global lexicographic check skips that work for already
//! ordered inputs; only the first key owns the dense rank directory.

use cuda_core::{CudaContext, CudaEvent, CudaStream, DeviceCopy, LaunchConfig1D, PinnedHostBuffer};
use rccl_memory::{
    ContiguousDeviceRange, CudaMemoryResource, DeviceBuffer, DeviceUninitBuffer,
    PoolMemoryResource, StatisticsMemoryResource,
};
use std::{cell::RefCell, error::Error, io, rc::Rc, sync::Arc};

pub type MemoryResource = StatisticsMemoryResource<PoolMemoryResource<CudaMemoryResource>>;
pub type Buffer<'a, T> = DeviceBuffer<'a, T, &'a MemoryResource>;

struct Input<'a>(&'a dyn ContiguousDeviceRange<i64>);
// SAFETY: the borrowed range retains ownership and supplies its initialized extent.
unsafe impl cuda_host::KernelSliceArg for Input<'_> {
    type Elem = i64;
    fn cu_deviceptr(&self) -> cuda_core::sys::CUdeviceptr {
        self.0.cu_deviceptr()
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}

pub const BLOCK_ROWS: usize = 2048;
pub const MAX_DOMAIN_KEYS: u64 = 1u64 << 32;
const THREADS: u32 = 256;

type BuildResult<T> = Result<T, Box<dyn Error>>;

thread_local! {
    static CACHED_KERNELS: RefCell<Option<(Arc<CudaContext>, Rc<kernels::LoadedModule>)>> =
        const { RefCell::new(None) };
}

fn cached_kernels(context: &Arc<CudaContext>) -> BuildResult<Rc<kernels::LoadedModule>> {
    CACHED_KERNELS.with(|cache| {
        if let Some((owner, module)) = cache.borrow().as_ref() {
            if Arc::ptr_eq(owner, context) {
                return Ok(Rc::clone(module));
            }
        }
        let module = Rc::new(unsafe { kernels::load(context)? });
        // Retain only the latest context on this thread. Replacing it cannot
        // invalidate in-flight host launchers, which retain their own Rc.
        *cache.borrow_mut() = Some((Arc::clone(context), Rc::clone(&module)));
        Ok(module)
    })
}

/// Load code before a timed build interval without retaining any query data.
/// Run construction and scans also initialize this cache lazily when needed.
pub fn prepare_kernels(context: &Arc<CudaContext>) -> BuildResult<()> {
    cached_kernels(context).map(|_| ())
}

/// Cardinalities of a completed run; counts and row offsets use the same width.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct RunStats {
    pub rows: u32,
    pub unique: u32,
    pub max_count: u32,
    pub invalid: u32,
    /// One when the input already has the required key order (lexicographic
    /// across every recorded level for a prefix run), including empty inputs.
    pub sorted: u32,
}
// SAFETY: this C-layout record consists only of u32 scalars.
unsafe impl DeviceCopy for RunStats {}

#[derive(Clone, Copy)]
#[repr(C)]
struct PrefixColumn {
    keys: *const i64,
    min_key: i64,
    domain: u64,
}
// SAFETY: a C-layout device pointer and scalar bounds; inputs retain ownership.
unsafe impl DeviceCopy for PrefixColumn {}

// The common two-level trie needs no metadata allocation or transfer: its
// only secondary column travels in the kernel parameter block.
#[derive(Clone, Copy)]
#[repr(C)]
struct PrefixKeys {
    first: PrefixColumn,
    more: *const PrefixColumn,
    len: usize,
}
unsafe impl DeviceCopy for PrefixKeys {}

// The metadata copy is enqueued, never waited on by the successful build.
// Retaining pinned ownership avoids both pageable-copy stalls and a dangling
// async source. An early drop waits only for this upload, not the later build.
struct PrefixUpload<'a> {
    host: PinnedHostBuffer<PrefixColumn>,
    ready: Option<CudaEvent>,
    stream: &'a CudaStream,
}
impl Drop for PrefixUpload<'_> {
    fn drop(&mut self) {
        let completed = if let Some(ready) = &self.ready {
            ready.synchronize()
        } else {
            // Only an error before recording the upload event reaches here.
            self.stream.synchronize()
        };
        self.stream.context().record_err(completed);
    }
}

/// Owning, immutable-after-build columns. Do not mutate directory buffers while
/// a `RankView` is in use, or retain a view after dropping/replacing its buffers.
pub struct Run<'a> {
    pub keys: Buffer<'a, i64>,
    pub payloads: Vec<Buffer<'a, i64>>,
    pub offsets: Buffer<'a, u32>,
    pub unique_keys: Buffer<'a, i64>,
    pub fences: Buffer<'a, i64>,
    pub bits: Buffer<'a, u64>,
    pub prefix: Buffer<'a, u32>,
    pub min_key: i64,
    pub max_key: i64,
    pub stats: Buffer<'a, RunStats>,
    _prefix_upload: Option<PrefixUpload<'a>>,
}

/// Non-owning device ABI. Pointers address GPU memory, never host memory.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct RankView {
    pub bits: *const u64,
    pub prefix: *const u32,
    pub offsets: *const u32,
    pub min_key: i64,
    pub max_key: i64,
}

// SAFETY: this C-layout value contains only copyable scalars and raw device
// pointers. Ownership remains with Run, not with copies of the view.
unsafe impl DeviceCopy for RankView {}

/// Return the half-open sorted-row range `(begin, end)` for a key.
///
/// Missing keys, including those outside the domain, return `None`. An empty
/// view never dereferences its pointers. This is a device-only operation.
///
/// # Safety
/// The view must describe a completed, unmodified run in the executing GPU's
/// context, and its allocations must outlive every kernel using this view.
#[cuda_device::device]
#[inline(always)]
pub unsafe fn point(view: RankView, key: i64) -> Option<(u32, u32)> {
    if key < view.min_key || key > view.max_key {
        return None;
    }
    // Unsigned subtraction also handles domains adjacent to i64::MIN/MAX
    // without ever overflowing signed arithmetic.
    let position = (key as u64).wrapping_sub(view.min_key as u64);
    let word_index = (position >> 6) as usize;
    let mask = 1u64 << (position & 63);
    let word = unsafe { view.bits.add(word_index).read() };
    if word & mask == 0 {
        return None;
    }
    let rank =
        unsafe { view.prefix.add(word_index).read() } + (word & mask.wrapping_sub(1)).count_ones();
    let begin = unsafe { view.offsets.add(rank as usize).read() };
    let end = unsafe { view.offsets.add(rank as usize + 1).read() };
    Some((begin, end))
}

/// In-place exclusive scan. Returned scratch must live until stream completion;
/// its last buffer contains the total, including for an empty input.
pub fn exclusive_scan<'a>(
    stream: &'a CudaStream,
    memory: &'a MemoryResource,
    values: &Buffer<'a, u32>,
) -> BuildResult<Vec<Buffer<'a, u32>>> {
    let module = cached_kernels(stream.context())?;
    let mut levels = Vec::new();
    let mut current = ptr(values);
    let mut length = values.len();
    loop {
        let blocks = length.div_ceil(THREADS as usize).max(1);
        let totals = allocate::<u32>(memory, stream, blocks)?;
        let launch = module.prepare_scan_blocks(LaunchConfig1D::new(blocks as u32, THREADS, 32))?;
        unsafe {
            module.scan_blocks(stream, &launch, current, length, ptr(&totals))?;
        }
        current = ptr(&totals);
        length = blocks;
        levels.push(totals);
        if blocks == 1 {
            break;
        }
    }
    for level in (0..levels.len() - 1).rev() {
        let target = if level == 0 {
            values
        } else {
            &levels[level - 1]
        };
        let launch = module.prepare_add_prefix(config(target.len()))?;
        unsafe {
            module.add_prefix(stream, &launch, ptr(target), target.len(), &levels[level])?;
        }
    }
    Ok(levels)
}

impl<'a> Run<'a> {
    /// Enqueue a stable counting-sort build without any host readback.
    ///
    /// `domain` is an inclusive bound supplied by input metadata, not inferred
    /// by downloading keys. `row_count`, when present, supplies the active
    /// prefix length on device; otherwise all input rows are active.
    /// Call `read_stats` before using results to reject invalid device counts
    /// or keys outside the supplied domain. All operations use `stream`.
    pub fn build(
        ctx: &CudaContext,
        stream: &'a CudaStream,
        memory: &'a MemoryResource,
        keys: &dyn ContiguousDeviceRange<i64>,
        payloads: &[&dyn ContiguousDeviceRange<i64>],
        domain: (i64, i64),
        row_count: Option<&Buffer<'_, u32>>,
    ) -> BuildResult<Self> {
        Self::build_inner(ctx, stream, memory, keys, payloads, domain, row_count, None)
    }

    // Keep the single-key launch sequence identical to the original builder.
    // A trie substitutes only its global order check and in-bin comparator;
    // sorted input reaches the same directory/gather path with no LSD passes.
    fn build_inner(
        ctx: &CudaContext,
        stream: &'a CudaStream,
        memory: &'a MemoryResource,
        keys: &dyn ContiguousDeviceRange<i64>,
        payloads: &[&dyn ContiguousDeviceRange<i64>],
        domain: (i64, i64),
        row_count: Option<&Buffer<'_, u32>>,
        secondary: Option<PrefixKeys>,
    ) -> BuildResult<Self> {
        if ctx != stream.context().as_ref() || ctx != keys.context().as_ref() {
            return Err(invalid(
                "context, stream and keys must share a CUDA context",
            ));
        }
        let capacity =
            u32::try_from(keys.len()).map_err(|_| invalid("run row count exceeds u32::MAX"))?;
        keys.ensure_usable_on(stream)?;
        validate_payloads(stream, keys.len(), payloads)?;
        if let Some(count) = row_count {
            count.ensure_usable_on(stream)?;
            if count.is_empty() {
                return Err(invalid("empty device row count"));
            }
        }
        let (min_key, max_key) = domain;
        let span = (max_key as i128 - min_key as i128 + 1).max(0);
        if span > MAX_DOMAIN_KEYS as i128 {
            return Err(invalid("dense rank domain exceeds 2^32 key positions"));
        }
        let domain = usize::try_from(span)
            .map_err(|_| invalid("dense domain exceeds host address space"))?;
        let words = domain.div_ceil(64);
        let n = keys.len();
        let unique_capacity = n.min(domain);
        let module = cached_kernels(stream.context())
            .map_err(|e| invalid(format!("run kernels load: {e}")))?;
        let stats = DeviceBuffer::zeroed_in(memory, stream, 1)?;
        let initialize = module.prepare_initialize(config(1))?;
        unsafe {
            module.initialize(
                stream,
                &initialize,
                capacity,
                row_count.map_or(core::ptr::null(), |c| ptr(c) as *const u32),
                ptr(&stats),
            )?;
        }
        let exclusive_scan = |values: &Buffer<'a, u32>| exclusive_scan(stream, memory, values);
        if let Some(secondary) = secondary {
            let check = module.prepare_prefix_sorted_check(config(n))?;
            unsafe {
                module.prefix_sorted_check(
                    stream,
                    &check,
                    &Input(keys),
                    min_key,
                    domain,
                    secondary,
                    ptr(&stats),
                )?;
            }
        } else {
            let check = module.prepare_sorted_check(config(n))?;
            unsafe {
                module.sorted_check(stream, &check, &Input(keys), min_key, domain, ptr(&stats))?;
            }
        }
        // Scratch is only partially initialized on the sorted path.
        let bins = DeviceUninitBuffer::<u32, _>::new_in(memory, stream, domain)?;
        let cursors = DeviceUninitBuffer::<u32, _>::new_in(memory, stream, domain)?;
        let bins_ptr = bins.cu_deviceptr() as *mut u32;
        let cursors_ptr = cursors.cu_deviceptr() as *mut u32;
        let clear = module.prepare_clear_histogram(LaunchConfig1D::new(
            domain.div_ceil(256).clamp(1, 568) as u32,
            THREADS,
            0,
        ))?;
        unsafe {
            module.clear_histogram(stream, &clear, domain, ptr(&stats), bins_ptr, cursors_ptr)?;
        }
        let bits = DeviceBuffer::zeroed_in(memory, stream, words)?;
        let adjacent = module.prepare_sorted_starts(bounded_config(n))?;
        unsafe {
            module.sorted_starts(
                stream,
                &adjacent,
                &Input(keys),
                min_key,
                ptr(&stats),
                bins_ptr,
                ptr(&bits),
            )?;
        }
        let histogram = module.prepare_histogram(config(n))?;
        unsafe {
            module.histogram(
                stream,
                &histogram,
                &Input(keys),
                min_key,
                ptr(&stats),
                bins_ptr,
                ptr(&bits),
            )?;
        }
        let prefix = allocate::<u32>(memory, stream, words)?;
        let word_rows = allocate::<u32>(memory, stream, words)?;
        let word_offsets = module.prepare_word_offsets(config(words * 32))?;
        unsafe {
            module.word_offsets(
                stream,
                &word_offsets,
                bins_ptr,
                domain,
                ptr(&stats),
                &bits,
                ptr(&prefix),
                ptr(&word_rows),
            )?;
        }
        let _row_scan = exclusive_scan(&word_rows)?;
        let prefix_scan = exclusive_scan(&prefix)?;
        let offsets = DeviceBuffer::zeroed_in(memory, stream, unique_capacity + 1)?;
        let unique_keys = DeviceBuffer::zeroed_in(memory, stream, unique_capacity)?;
        let complete = module.prepare_complete_stats(config(1))?;
        unsafe {
            module.complete_stats(
                stream,
                &complete,
                ptr(prefix_scan.last().unwrap()),
                ptr(&stats),
                ptr(&offsets),
            )?;
        }
        let permutation = DeviceUninitBuffer::<u32, _>::new_in(memory, stream, n)?;
        let permutation_ptr = permutation.cu_deviceptr() as *mut u32;
        let scatter = module.prepare_scatter(config(n))?;
        unsafe {
            module.scatter(
                stream,
                &scatter,
                &Input(keys),
                min_key,
                bins_ptr,
                &word_rows,
                ptr(&stats),
                cursors_ptr,
                permutation_ptr,
            )?;
        }
        if let Some(secondary) = secondary {
            let order = module.prepare_order_prefix_bins(bounded_config(domain))?;
            unsafe {
                module.order_prefix_bins(
                    stream,
                    &order,
                    bins_ptr,
                    domain,
                    &word_rows,
                    ptr(&stats),
                    min_key,
                    &bits,
                    &prefix,
                    permutation_ptr,
                    ptr(&offsets),
                    ptr(&unique_keys),
                    secondary,
                )?;
            }
        } else {
            let order = module.prepare_order_bins(bounded_config(domain))?;
            unsafe {
                module.order_bins(
                    stream,
                    &order,
                    bins_ptr,
                    domain,
                    &word_rows,
                    ptr(&stats),
                    min_key,
                    &bits,
                    &prefix,
                    permutation_ptr,
                    ptr(&offsets),
                    ptr(&unique_keys),
                )?;
            }
        }
        let sorted_directory = module.prepare_sorted_directory(bounded_config(n))?;
        unsafe {
            module.sorted_directory(
                stream,
                &sorted_directory,
                &Input(keys),
                min_key,
                ptr(&stats),
                bins_ptr,
                &bits,
                &prefix,
                ptr(&offsets),
                ptr(&unique_keys),
            )?;
        }
        let sorted = allocate::<i64>(memory, stream, n)?;
        let gather = module.prepare_gather(config(n))?;
        unsafe {
            module.gather(
                stream,
                &gather,
                &Input(keys),
                permutation_ptr,
                ptr(&stats),
                ptr(&sorted),
            )?;
        }
        let fences = allocate::<i64>(memory, stream, n.div_ceil(BLOCK_ROWS))?;
        let fence_launch = module.prepare_fences(config(fences.len()))?;
        unsafe {
            module.fences(stream, &fence_launch, &sorted, ptr(&stats), ptr(&fences))?;
        }
        let mut clustered = Vec::with_capacity(payloads.len());
        for payload in payloads {
            let output = allocate::<i64>(memory, stream, n)?;
            unsafe {
                module.gather(
                    stream,
                    &gather,
                    &Input(*payload),
                    permutation_ptr,
                    ptr(&stats),
                    ptr(&output),
                )?;
            }
            clustered.push(output);
        }
        Ok(Self {
            keys: sorted,
            payloads: clustered,
            offsets,
            unique_keys,
            fences,
            bits,
            prefix,
            min_key,
            max_key,
            stats,
            _prefix_upload: None,
        })
    }

    /// Build a stable lexicographic prefix run entirely on device.
    ///
    /// One check validates all domains and compares adjacent complete keys.
    /// Already ordered input uses the single-key sorted directory/copy path.
    /// Otherwise the first key supplies counting-sort bins; each bin orders
    /// deeper keys lexicographically, with original row ID breaking ties.
    /// `keys` and the rank directory describe `keys[0]`; payload order is unchanged.
    pub fn build_prefix(
        ctx: &CudaContext,
        stream: &'a CudaStream,
        memory: &'a MemoryResource,
        keys: &[&dyn ContiguousDeviceRange<i64>],
        payloads: &[&dyn ContiguousDeviceRange<i64>],
        domains: &[(i64, i64)],
        row_count: Option<&Buffer<'_, u32>>,
    ) -> BuildResult<Self> {
        if keys.is_empty() || keys.len() != domains.len() {
            return Err(invalid(
                "prefix runs require nonempty keys with matching domains",
            ));
        }
        let n = keys[0].len();
        if keys.len() == 1 {
            return Self::build(
                ctx, stream, memory, keys[0], payloads, domains[0], row_count,
            );
        }
        validate_payloads(stream, n, &keys[1..])?;
        let mut secondary = PrefixKeys {
            first: prefix_column(keys[1], domains[1])?,
            more: core::ptr::null(),
            len: keys.len() - 1,
        };
        if keys.len() == 2 {
            return Self::build_inner(
                ctx,
                stream,
                memory,
                keys[0],
                payloads,
                domains[0],
                row_count,
                Some(secondary),
            );
        }
        let mut host = PinnedHostBuffer::<PrefixColumn>::zeroed(stream.context(), keys.len() - 2)?;
        for ((column, &key), &domain) in host
            .as_mut_slice()
            .iter_mut()
            .zip(&keys[2..])
            .zip(&domains[2..])
        {
            *column = prefix_column(key, domain)?;
        }
        let metadata = allocate::<PrefixColumn>(memory, stream, host.len())?;
        let mut upload = PrefixUpload {
            host,
            ready: None,
            stream,
        };
        // SAFETY: the pinned source remains owned by the returned Run until its
        // completion event has finished; the destination is stream-ordered.
        unsafe {
            cuda_core::memory::memcpy_htod_async(
                metadata.cu_deviceptr(),
                upload.host.as_ptr(),
                upload.host.num_bytes(),
                stream.cu_stream(),
            )?;
        }
        upload.ready = Some(stream.record_event(None)?);
        secondary.more = metadata.cu_deviceptr() as *const PrefixColumn;
        let mut run = Self::build_inner(
            ctx,
            stream,
            memory,
            keys[0],
            payloads,
            domains[0],
            row_count,
            Some(secondary),
        )?;
        run._prefix_upload = Some(upload);
        Ok(run)
    }

    /// Borrow the run's rank directory as a copyable device ABI value.
    /// The caller must keep this run and its buffers alive and unchanged while
    /// any kernel uses the returned view; no ownership is transferred.
    pub fn view(&self) -> RankView {
        RankView {
            bits: self.bits.cu_deviceptr() as *const u64,
            prefix: self.prefix.cu_deviceptr() as *const u32,
            offsets: self.offsets.cu_deviceptr() as *const u32,
            min_key: self.min_key,
            max_key: self.max_key,
        }
    }
}

fn prefix_column(
    key: &dyn ContiguousDeviceRange<i64>,
    (min_key, max_key): (i64, i64),
) -> BuildResult<PrefixColumn> {
    let span = (max_key as i128 - min_key as i128 + 1).max(0);
    if span > MAX_DOMAIN_KEYS as i128 {
        return Err(invalid("dense rank domain exceeds 2^32 key positions"));
    }
    Ok(PrefixColumn {
        keys: key.cu_deviceptr() as *const i64,
        min_key,
        domain: span as u64,
    })
}

fn validate_payloads(
    stream: &CudaStream,
    rows: usize,
    payloads: &[&dyn ContiguousDeviceRange<i64>],
) -> BuildResult<()> {
    for payload in payloads {
        if payload.len() != rows {
            return Err(invalid("payload length differs from key length"));
        }
        payload.ensure_usable_on(stream)?;
    }
    Ok(())
}

/// Complete any number of stream-ordered builds with one metadata readback.
pub fn read_stats(
    memory: &MemoryResource,
    stream: &CudaStream,
    runs: &[&Run<'_>],
) -> BuildResult<Vec<RunStats>> {
    let combined = allocate::<RunStats>(memory, stream, runs.len())?;
    for (index, run) in runs.iter().enumerate() {
        run.stats.ensure_usable_on(stream)?;
        unsafe {
            cuda_core::memory::memcpy_dtod_async(
                combined.cu_deviceptr() + (index * core::mem::size_of::<RunStats>()) as u64,
                run.stats.cu_deviceptr(),
                core::mem::size_of::<RunStats>(),
                stream.cu_stream(),
            )?;
        }
    }
    let result = combined.to_host_vec()?;
    if result.iter().any(|s| s.invalid != 0) {
        return Err(invalid(
            "device row count exceeds capacity or key outside supplied domain",
        ));
    }
    Ok(result)
}

fn invalid(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(io::Error::new(io::ErrorKind::InvalidInput, message.into()))
}

fn config(items: usize) -> LaunchConfig1D {
    LaunchConfig1D::new(items.div_ceil(THREADS as usize).max(1) as u32, THREADS, 0)
}
fn bounded_config(items: usize) -> LaunchConfig1D {
    LaunchConfig1D::new(
        items.div_ceil(THREADS as usize).clamp(1, 568) as u32,
        THREADS,
        0,
    )
}

fn ptr<T: DeviceCopy>(buffer: &Buffer<'_, T>) -> *mut T {
    buffer.cu_deviceptr() as *mut T
}

// Every callsite initializes the complete allocation on its stream before
// reading it. Avoid a redundant memset for fully overwritten output columns.
fn allocate<'a, T: DeviceCopy>(
    memory: &'a MemoryResource,
    stream: &'a CudaStream,
    len: usize,
) -> BuildResult<Buffer<'a, T>> {
    // SAFETY: each callsite completely initializes its output on this stream
    // before exposing it to any reader.
    Ok(unsafe { DeviceUninitBuffer::new_in(memory, stream, len)?.assume_init() })
}

#[cuda_device::cuda_module]
mod kernels {
    use super::{PrefixColumn, PrefixKeys, RunStats};
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU32, DeviceAtomicU64};
    use cuda_device::{
        device, kernel, launch_bounds, launch_contract, thread, warp, DynamicSharedArray,
    };

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn initialize(capacity: u32, count: *const u32, stats: *mut RunStats) {
        if thread::index_1d().get() == 0 {
            let rows = if count.is_null() {
                capacity
            } else {
                unsafe { count.read() }
            };
            unsafe {
                (*stats).rows = rows.min(capacity);
                (*stats).invalid = (rows > capacity) as u32;
                (*stats).sorted = 1;
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn complete_stats(total: *const u32, stats: *mut RunStats, offsets: *mut u32) {
        if thread::index_1d().get() == 0 {
            unsafe {
                let unique = total.read();
                (*stats).unique = unique;
                offsets.add(unique as usize).write((*stats).rows);
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn sorted_check(keys: &[i64], min_key: i64, domain: usize, stats: *mut RunStats) {
        let i = thread::index_1d().get();
        let active = i < unsafe { (*stats).rows } as usize;
        let invalid = active && (keys[i] as u64).wrapping_sub(min_key as u64) >= domain as u64;
        let inversion = active && i != 0 && keys[i - 1] > keys[i];
        let invalid_lanes = warp::ballot_sync(u32::MAX, invalid);
        let inverted_lanes = warp::ballot_sync(u32::MAX, inversion);
        if warp::lane_id() == 0 {
            unsafe {
                if invalid_lanes != 0 {
                    DeviceAtomicU32::from_ptr(core::ptr::addr_of_mut!((*stats).invalid))
                        .store(1, AtomicOrdering::Relaxed);
                }
                if inverted_lanes != 0 {
                    DeviceAtomicU32::from_ptr(core::ptr::addr_of_mut!((*stats).sorted))
                        .store(0, AtomicOrdering::Relaxed);
                }
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn prefix_sorted_check(
        keys: &[i64],
        min_key: i64,
        domain: usize,
        secondary: PrefixKeys,
        stats: *mut RunStats,
    ) {
        let i = thread::index_1d().get();
        let active = i < unsafe { (*stats).rows } as usize;
        let mut invalid = active && (keys[i] as u64).wrapping_sub(min_key as u64) >= domain as u64;
        let mut inversion = active && i != 0 && keys[i - 1] > keys[i];
        let mut equal = active && i != 0 && keys[i - 1] == keys[i];
        let mut level = 0;
        while level < secondary.len {
            if active {
                let column = unsafe { prefix_level(secondary, level) };
                let value = unsafe { column.keys.add(i).read() };
                invalid |= (value as u64).wrapping_sub(column.min_key as u64) >= column.domain;
                if equal {
                    let previous = unsafe { column.keys.add(i - 1).read() };
                    inversion = previous > value;
                    equal = previous == value;
                }
            }
            level += 1;
        }
        let invalid_lanes = warp::ballot_sync(u32::MAX, invalid);
        let inverted_lanes = warp::ballot_sync(u32::MAX, inversion);
        if warp::lane_id() == 0 {
            unsafe {
                if invalid_lanes != 0 {
                    DeviceAtomicU32::from_ptr(core::ptr::addr_of_mut!((*stats).invalid))
                        .store(1, AtomicOrdering::Relaxed);
                }
                if inverted_lanes != 0 {
                    DeviceAtomicU32::from_ptr(core::ptr::addr_of_mut!((*stats).sorted))
                        .store(0, AtomicOrdering::Relaxed);
                }
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn clear_histogram(
        domain: usize,
        stats: *const RunStats,
        bins: *mut u32,
        cursors: *mut u32,
    ) {
        if unsafe { (*stats).sorted != 0 || (*stats).invalid != 0 } {
            return;
        }
        let mut bin = thread::index_1d().get();
        let stride = thread::gridDim_x() as usize * 256;
        while bin < domain {
            unsafe {
                bins.add(bin).write(0);
                cursors.add(bin).write(0);
            }
            bin += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn sorted_starts(
        keys: &[i64],
        min_key: i64,
        stats: *const RunStats,
        bins: *mut u32,
        bits: *mut u64,
    ) {
        if unsafe { (*stats).sorted == 0 || (*stats).invalid != 0 } {
            return;
        }
        let mut i = thread::index_1d().get();
        let stride = thread::gridDim_x() as usize * 256;
        let rows = unsafe { (*stats).rows } as usize;
        while i < rows {
            if i == 0 || keys[i - 1] != keys[i] {
                let bin = (keys[i] as u64).wrapping_sub(min_key as u64) as usize;
                unsafe {
                    bins.add(bin).write(i as u32);
                    DeviceAtomicU64::from_ptr(bits.add(bin >> 6))
                        .fetch_or(1u64 << (bin & 63), AtomicOrdering::Relaxed);
                }
            }
            i += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn sorted_directory(
        keys: &[i64],
        min_key: i64,
        stats: *mut RunStats,
        bins: *const u32,
        bits: &[u64],
        prefix: &[u32],
        offsets: *mut u32,
        unique: *mut i64,
    ) {
        if unsafe { (*stats).sorted == 0 || (*stats).invalid != 0 } {
            return;
        }
        let mut i = thread::index_1d().get();
        let stride = thread::gridDim_x() as usize * 256;
        let rows = unsafe { (*stats).rows } as usize;
        let mut maximum = 0;
        while i < rows {
            let bin = (keys[i] as u64).wrapping_sub(min_key as u64) as usize;
            if i == 0 || keys[i - 1] != keys[i] {
                let mask = 1u64 << (bin & 63);
                let rank = prefix[bin >> 6] + (bits[bin >> 6] & (mask - 1)).count_ones();
                unsafe {
                    offsets.add(rank as usize).write(i as u32);
                    unique.add(rank as usize).write(keys[i]);
                }
            }
            if i + 1 == rows || keys[i + 1] != keys[i] {
                maximum = maximum.max(i as u32 + 1 - unsafe { bins.add(bin).read() });
            }
            i += stride;
        }
        let lane = warp::lane_id();
        let mut distance = 1;
        while distance < 32 {
            let left = warp::shuffle_up(maximum, distance);
            if lane >= distance {
                maximum = maximum.max(left);
            }
            distance <<= 1;
        }
        if lane == 31 && maximum != 0 {
            unsafe {
                DeviceAtomicU32::from_ptr(core::ptr::addr_of_mut!((*stats).max_count))
                    .fetch_max(maximum, AtomicOrdering::Relaxed);
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn histogram(
        keys: &[i64],
        min_key: i64,
        stats: *const RunStats,
        counts: *mut u32,
        bits: *mut u64,
    ) {
        if unsafe { (*stats).sorted != 0 || (*stats).invalid != 0 } {
            return;
        }
        let i = thread::index_1d().get();
        if i < unsafe { (*stats).rows } as usize {
            let bin = (keys[i] as u64).wrapping_sub(min_key as u64) as usize;
            unsafe {
                let previous = DeviceAtomicU32::from_ptr(counts.add(bin))
                    .fetch_add(1, AtomicOrdering::Relaxed);
                if previous == 0 {
                    DeviceAtomicU64::from_ptr(bits.add(bin >> 6))
                        .fetch_or(1u64 << (bin & 63), AtomicOrdering::Relaxed);
                }
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (32, 32))]
    pub unsafe fn scan_blocks(values: *mut u32, length: usize, totals: *mut u32) {
        let i = thread::index_1d().get();
        let value = if i < length {
            unsafe { values.add(i).read() }
        } else {
            0
        };
        let scratch = DynamicSharedArray::<u8>::get_raw() as *mut u32;
        let (prefix, total) = gpu_tiles::scan(value, scratch);
        if i < length {
            unsafe {
                values.add(i).write(prefix);
            }
        }
        if thread::threadIdx_x() == 0 {
            unsafe {
                totals.add(i / 256).write(total);
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn add_prefix(values: *mut u32, length: usize, prefixes: &[u32]) {
        let i = thread::index_1d().get();
        if i < length {
            unsafe {
                let value = values.add(i).read();
                values.add(i).write(value + prefixes[i / 256]);
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn word_offsets(
        bins: *mut u32,
        domain: usize,
        stats: *mut RunStats,
        bits: &[u64],
        prefix: *mut u32,
        word_rows: *mut u32,
    ) {
        // One converged warp handles a bitmap word. Both 32-bin halves are
        // coalesced; only the 64x smaller word totals need a global scan.
        let word = thread::index_1d().get() / 32;
        let lane = warp::lane_id();
        if unsafe { (*stats).sorted != 0 || (*stats).invalid != 0 } {
            if lane == 31 && word < bits.len() {
                unsafe {
                    word_rows.add(word).write(0);
                    prefix.add(word).write(bits[word].count_ones());
                }
            }
            return;
        }
        let first = word * 64 + lane as usize;
        let second = first + 32;
        let a = if first < domain {
            unsafe { bins.add(first).read() }
        } else {
            0
        };
        let b = if second < domain {
            unsafe { bins.add(second).read() }
        } else {
            0
        };
        let mut sum_a = a;
        let mut sum_b = b;
        let mut maximum = a.max(b);
        let mut distance = 1;
        while distance < 32 {
            let left_a = warp::shuffle_up(sum_a, distance);
            let left_b = warp::shuffle_up(sum_b, distance);
            let left_max = warp::shuffle_up(maximum, distance);
            if lane >= distance {
                sum_a += left_a;
                sum_b += left_b;
                maximum = maximum.max(left_max);
            }
            distance <<= 1;
        }
        let first_total = warp::shuffle_down_sync(u32::MAX, sum_a, 31 - lane);
        unsafe {
            if first < domain {
                bins.add(first).write(sum_a - a);
            }
            if second < domain {
                bins.add(second).write(first_total + sum_b - b);
            }
            if lane == 31 && word < bits.len() {
                word_rows.add(word).write(first_total + sum_b);
                prefix.add(word).write(bits[word].count_ones());
                DeviceAtomicU32::from_ptr(core::ptr::addr_of_mut!((*stats).max_count))
                    .fetch_max(maximum, AtomicOrdering::Relaxed);
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn scatter(
        keys: &[i64],
        min_key: i64,
        bins: *const u32,
        word_rows: &[u32],
        stats: *const RunStats,
        cursors: *mut u32,
        permutation: *mut u32,
    ) {
        if unsafe { (*stats).sorted != 0 || (*stats).invalid != 0 } {
            return;
        }
        let i = thread::index_1d().get();
        if i < unsafe { (*stats).rows } as usize && unsafe { (*stats).invalid } == 0 {
            let bin = (keys[i] as u64).wrapping_sub(min_key as u64) as usize;
            unsafe {
                let slot = DeviceAtomicU32::from_ptr(cursors.add(bin))
                    .fetch_add(1, AtomicOrdering::Relaxed);
                permutation
                    .add((word_rows[bin >> 6] + bins.add(bin).read() + slot) as usize)
                    .write(i as u32);
            }
        }
    }

    // One thread owns each bin. A max heap gives ascending provenance in
    // O(k log k), including bins larger than a block or the entire input.
    #[device]
    #[inline(always)]
    unsafe fn sift_down(values: *mut u32, mut root: usize, length: usize) {
        let value = unsafe { values.add(root).read() };
        while root < length / 2 {
            let mut child = root * 2 + 1;
            let mut child_value = unsafe { values.add(child).read() };
            if child + 1 < length {
                let right = unsafe { values.add(child + 1).read() };
                if right > child_value {
                    child += 1;
                    child_value = right;
                }
            }
            if value >= child_value {
                break;
            }
            unsafe {
                values.add(root).write(child_value);
            }
            root = child;
        }
        unsafe {
            values.add(root).write(value);
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn order_bins(
        bins: *const u32,
        domain: usize,
        word_rows: &[u32],
        stats: *const RunStats,
        min_key: i64,
        bits: &[u64],
        prefix: &[u32],
        permutation: *mut u32,
        offsets: *mut u32,
        unique: *mut i64,
    ) {
        if unsafe { (*stats).sorted != 0 || (*stats).invalid != 0 } {
            return;
        }
        let mut bin = thread::index_1d().get();
        let stride = thread::gridDim_x() as usize * 256;
        while bin < domain {
            let begin = word_rows[bin >> 6] + unsafe { bins.add(bin).read() };
            let end = if bin + 1 < domain {
                word_rows[(bin + 1) >> 6] + unsafe { bins.add(bin + 1).read() }
            } else {
                unsafe { (*stats).rows }
            };
            if begin != end {
                let mask = 1u64 << (bin & 63);
                let rank = prefix[bin >> 6] + (bits[bin >> 6] & (mask - 1)).count_ones();
                unsafe {
                    offsets.add(rank as usize).write(begin);
                    unique
                        .add(rank as usize)
                        .write((min_key as u64).wrapping_add(bin as u64) as i64);
                    let values = permutation.add(begin as usize);
                    let length = (end - begin) as usize;
                    let mut parent = length / 2;
                    while parent != 0 {
                        parent -= 1;
                        sift_down(values, parent, length);
                    }
                    let mut remaining = length;
                    while remaining > 1 {
                        remaining -= 1;
                        let largest = values.read();
                        values.write(values.add(remaining).read());
                        values.add(remaining).write(largest);
                        sift_down(values, 0, remaining);
                    }
                }
            }
            bin += stride;
        }
    }

    #[device]
    #[inline(always)]
    unsafe fn prefix_level(secondary: PrefixKeys, level: usize) -> PrefixColumn {
        if level == 0 {
            secondary.first
        } else {
            unsafe { secondary.more.add(level - 1).read() }
        }
    }

    #[device]
    #[inline(always)]
    unsafe fn prefix_greater(secondary: PrefixKeys, a: u32, b: u32) -> bool {
        let mut level = 0;
        while level < secondary.len {
            let keys = unsafe { prefix_level(secondary, level) }.keys;
            let left = unsafe { keys.add(a as usize).read() };
            let right = unsafe { keys.add(b as usize).read() };
            if left != right {
                return left > right;
            }
            level += 1;
        }
        // Original row IDs make equal complete keys stable, even though the
        // histogram's atomic scatter itself is unordered.
        a > b
    }

    #[device]
    #[inline(always)]
    unsafe fn sift_prefix_down(
        values: *mut u32,
        mut root: usize,
        length: usize,
        secondary: PrefixKeys,
    ) {
        let value = unsafe { values.add(root).read() };
        while root < length / 2 {
            let mut child = root * 2 + 1;
            let mut child_value = unsafe { values.add(child).read() };
            if child + 1 < length {
                let right = unsafe { values.add(child + 1).read() };
                if unsafe { prefix_greater(secondary, right, child_value) } {
                    child += 1;
                    child_value = right;
                }
            }
            if !unsafe { prefix_greater(secondary, child_value, value) } {
                break;
            }
            unsafe {
                values.add(root).write(child_value);
            }
            root = child;
        }
        unsafe {
            values.add(root).write(value);
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn order_prefix_bins(
        bins: *const u32,
        domain: usize,
        word_rows: &[u32],
        stats: *const RunStats,
        min_key: i64,
        bits: &[u64],
        prefix: &[u32],
        permutation: *mut u32,
        offsets: *mut u32,
        unique: *mut i64,
        secondary: PrefixKeys,
    ) {
        if unsafe { (*stats).sorted != 0 || (*stats).invalid != 0 } {
            return;
        }
        let mut bin = thread::index_1d().get();
        let stride = thread::gridDim_x() as usize * 256;
        while bin < domain {
            let begin = word_rows[bin >> 6] + unsafe { bins.add(bin).read() };
            let end = if bin + 1 < domain {
                word_rows[(bin + 1) >> 6] + unsafe { bins.add(bin + 1).read() }
            } else {
                unsafe { (*stats).rows }
            };
            if begin != end {
                let mask = 1u64 << (bin & 63);
                let rank = prefix[bin >> 6] + (bits[bin >> 6] & (mask - 1)).count_ones();
                unsafe {
                    offsets.add(rank as usize).write(begin);
                    unique
                        .add(rank as usize)
                        .write((min_key as u64).wrapping_add(bin as u64) as i64);
                    let values = permutation.add(begin as usize);
                    let length = (end - begin) as usize;
                    let mut parent = length / 2;
                    while parent != 0 {
                        parent -= 1;
                        sift_prefix_down(values, parent, length, secondary);
                    }
                    let mut remaining = length;
                    while remaining > 1 {
                        remaining -= 1;
                        let largest = values.read();
                        values.write(values.add(remaining).read());
                        values.add(remaining).write(largest);
                        sift_prefix_down(values, 0, remaining, secondary);
                    }
                }
            }
            bin += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn gather(
        input: &[i64],
        permutation: *const u32,
        stats: *const RunStats,
        output: *mut i64,
    ) {
        let i = thread::index_1d().get();
        if i < input.len() {
            let value = if i < unsafe { (*stats).rows } as usize && unsafe { (*stats).invalid } == 0
            {
                if unsafe { (*stats).sorted } != 0 {
                    input[i]
                } else {
                    input[unsafe { permutation.add(i).read() } as usize]
                }
            } else {
                0
            };
            unsafe {
                output.add(i).write(value);
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn fences(keys: &[i64], stats: *const RunStats, output: *mut i64) {
        let i = thread::index_1d().get();
        if i < (keys.len() + super::BLOCK_ROWS - 1) / super::BLOCK_ROWS {
            let row = i * super::BLOCK_ROWS;
            unsafe {
                output.add(i).write(if row < (*stats).rows as usize {
                    keys[row]
                } else {
                    0
                });
            }
        }
    }
}
