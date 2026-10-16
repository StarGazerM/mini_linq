//! GPU-built sorted, clustered runs with a dense rank directory.
//!
//! Construction uses a dense counting sort and hierarchical GPU scans, with no
//! external sorting library. Atomic scatter is made stable by sorting original
//! row IDs within each bin with heapsort before gathering payload columns.
//! Only extrema, the unique count, and the maximum bin count are read back.
//! Input writes must be ordered before `Run::build` on its stream. Construction
//! completes before returning, so the result can subsequently be used on any
//! stream in the same context. Inputs are not modified.
//!
//! `offsets` has `stats.unique + 1` entries, including the final row count.
//! `fences[b]` is the minimum key of sorted rows `[2048*b, 2048*(b+1))`.
//! `prefix[w]` is the number of set bits strictly before bitmap word `w`.
//! Empty runs have offsets `[0]`, empty other columns, and `min_key > max_key`.
//! Dense directories accept at most 2^32 key positions (including missing
//! keys), and row counts must fit in u32. Sparse wider domains are rejected,
//! not silently converted to a different lookup representation.

use cuda_core::{CudaContext, CudaStream, DeviceBuffer, DeviceCopy, LaunchConfig1D};
use std::{error::Error, io};

pub const BLOCK_ROWS: usize = 2048;
pub const MAX_DOMAIN_KEYS: u64 = 1u64 << 32;
const THREADS: u32 = 256;

type BuildResult<T> = Result<T, Box<dyn Error>>;

/// Cardinalities of a completed run; counts and row offsets use the same width.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(C)]
pub struct RunStats {
    pub rows: u32,
    pub unique: u32,
    pub max_count: u32,
}

/// Owning, immutable-after-build columns. Do not mutate directory buffers while
/// a `RankView` is in use, or retain a view after dropping/replacing its buffers.
pub struct Run {
    pub keys: DeviceBuffer<i64>,
    pub payloads: Vec<DeviceBuffer<i64>>,
    pub offsets: DeviceBuffer<u32>,
    pub unique_keys: DeviceBuffer<i64>,
    pub fences: DeviceBuffer<i64>,
    pub bits: DeviceBuffer<u64>,
    pub prefix: DeviceBuffer<u32>,
    pub min_key: i64,
    pub max_key: i64,
    pub stats: RunStats,
    /// Absolute device bytes in use at build completion, including live scratch.
    /// This is a context/device memory sample, not this run's allocation size.
    pub build_peak_mem_used: usize,
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
    let rank = unsafe { view.prefix.add(word_index).read() }
        + (word & mask.wrapping_sub(1)).count_ones();
    let begin = unsafe { view.offsets.add(rank as usize).read() };
    let end = unsafe { view.offsets.add(rank as usize + 1).read() };
    Some((begin, end))
}

impl Run {
    /// Count signed i64 keys into a dense domain, scan bin offsets, restore
    /// stable source-row order per bin, and build CSR, fences and dense rank.
    ///
    /// The payload slice can be made from an existing `Vec<DeviceBuffer<i64>>`
    /// with `payloads.iter().collect::<Vec<_>>()`. Every column must match the
    /// key length and context. Equal keys preserve their input row order.
    pub fn build(
        ctx: &CudaContext,
        stream: &CudaStream,
        keys: &DeviceBuffer<i64>,
        payloads: &[&DeviceBuffer<i64>],
    ) -> BuildResult<Self> {
        if ctx != stream.context().as_ref() || ctx != keys.context().as_ref() {
            return Err(invalid("context, stream and keys must share a CUDA context"));
        }
        let rows = u32::try_from(keys.len())
            .map_err(|_| invalid("run row count exceeds u32::MAX"))?;
        for payload in payloads {
            if payload.len() != keys.len() {
                return Err(invalid("payload length differs from key length"));
            }
            if ctx != payload.context().as_ref() {
                return Err(invalid("payload and keys must share a CUDA context"));
            }
        }
        ctx.bind_to_thread()?;
        if rows == 0 {
            let mut run = Self {
                keys: DeviceBuffer::zeroed(stream, 0)?,
                payloads: payloads.iter()
                    .map(|_| DeviceBuffer::zeroed(stream, 0))
                    .collect::<Result<Vec<_>, _>>()?,
                offsets: DeviceBuffer::zeroed(stream, 1)?,
                unique_keys: DeviceBuffer::zeroed(stream, 0)?,
                fences: DeviceBuffer::zeroed(stream, 0)?,
                bits: DeviceBuffer::zeroed(stream, 0)?,
                prefix: DeviceBuffer::zeroed(stream, 0)?,
                min_key: 0,
                max_key: -1,
                stats: RunStats::default(),
                build_peak_mem_used: 0,
            };
            stream.synchronize()?;
            run.build_peak_mem_used = memory_used()?;
            return Ok(run);
        }

        // SAFETY: this crate owns these kernel declarations and their bundle.
        let module = unsafe { kernels::load(stream.context()) }
            .map_err(|e| invalid(format!("run kernels load: {e}")))?;
        let n = keys.len();
        // Bias signed keys into unsigned order, allowing unsigned atomic
        // extrema without signed overflow at either end of the i64 range.
        let extrema = DeviceBuffer::from_host(stream, &[u64::MAX, 0u64])?;
        let reduce = module.prepare_extrema(config(n))?;
        unsafe { module.extrema(stream, &reduce, keys, ptr(&extrema))?; }
        let mut host_extrema = [0u64; 2];
        extrema.copy_to_host(stream, &mut host_extrema)?;
        let min_key = (host_extrema[0] ^ (1u64 << 63)) as i64;
        let max_key = (host_extrema[1] ^ (1u64 << 63)) as i64;
        let span = max_key as i128 - min_key as i128 + 1;
        if span > MAX_DOMAIN_KEYS as i128 {
            return Err(invalid("dense rank domain exceeds 2^32 key positions"));
        }
        let domain = usize::try_from(span)
            .map_err(|_| invalid("dense domain exceeds host address space"))?;
        let words = domain.div_ceil(64);

        // Each level scans 256-item blocks in place and emits block totals.
        // Scan those totals recursively, then propagate their exclusive
        // prefixes downwards. Retain every level until submitted work finishes.
        let exclusive_scan = |values: &DeviceBuffer<u32>| -> BuildResult<Vec<DeviceBuffer<u32>>> {
            let mut levels = Vec::new();
            let mut current = ptr(values);
            let mut length = values.len();
            loop {
                let blocks = length.div_ceil(THREADS as usize);
                let totals = allocate::<u32>(stream, blocks)?;
                let launch = module.prepare_scan_blocks(
                    LaunchConfig1D::new(blocks as u32, THREADS, 32),
                )?;
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
                let target = if level == 0 { values } else { &levels[level - 1] };
                let launch = module.prepare_add_prefix(config(target.len()))?;
                unsafe {
                    module.add_prefix(stream, &launch, ptr(target), target.len(), &levels[level])?;
                }
            }
            Ok(levels)
        };

        let bins = DeviceBuffer::<u32>::zeroed(stream, domain)?;
        let histogram = module.prepare_histogram(config(n))?;
        unsafe { module.histogram(stream, &histogram, keys, min_key, ptr(&bins))?; }
        let _bin_scan = exclusive_scan(&bins)?;

        let bits = allocate::<u64>(stream, words)?;
        let prefix = allocate::<u32>(stream, words)?;
        let maximum = DeviceBuffer::<u32>::zeroed(stream, 1)?;
        let directory = module.prepare_directory(config(words))?;
        unsafe {
            module.directory(stream, &directory, &bins, rows,
                ptr(&bits), ptr(&prefix), ptr(&maximum))?;
        }
        let prefix_scan = exclusive_scan(&prefix)?;
        let mut host_unique = [0u32; 1];
        // The unscanned final one-element level is the total population.
        prefix_scan.last().unwrap().copy_to_host(stream, &mut host_unique)?;
        let unique = host_unique[0];
        let offset_items = (unique as usize).checked_add(1)
            .ok_or_else(|| invalid("CSR sentinel exceeds host address space"))?;
        let offsets = allocate::<u32>(stream, offset_items)?;
        let unique_keys = allocate::<i64>(stream, unique as usize)?;
        let permutation = allocate::<u32>(stream, n)?;
        let cursors = DeviceBuffer::<u32>::zeroed(stream, domain)?;
        let scatter = module.prepare_scatter(config(n))?;
        unsafe {
            module.scatter(stream, &scatter, keys, min_key, &bins,
                ptr(&cursors), ptr(&permutation))?;
        }
        let order = module.prepare_order_bins(config(domain))?;
        unsafe {
            module.order_bins(stream, &order, &bins, rows, min_key, &bits, &prefix,
                ptr(&permutation), ptr(&offsets), ptr(&unique_keys))?;
        }

        let sorted = allocate::<i64>(stream, n)?;
        let gather = module.prepare_gather(config(n))?;
        unsafe { module.gather(stream, &gather, keys, &permutation, ptr(&sorted))?; }
        let fences = allocate::<i64>(stream, n.div_ceil(BLOCK_ROWS))?;
        let fence_launch = module.prepare_fences(config(fences.len()))?;
        unsafe { module.fences(stream, &fence_launch, &sorted, ptr(&fences))?; }
        let mut clustered = Vec::with_capacity(payloads.len());
        for payload in payloads {
            let output = allocate::<i64>(stream, n)?;
            unsafe { module.gather(stream, &gather, *payload, &permutation, ptr(&output))?; }
            clustered.push(output);
        }
        let mut max_count = [0u32; 1];
        maximum.copy_to_host(stream, &mut max_count)?;
        stream.synchronize()?;
        let build_peak_mem_used = memory_used()?;
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
            stats: RunStats { rows, unique, max_count: max_count[0] },
            build_peak_mem_used,
        })
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

fn memory_used() -> BuildResult<usize> {
    let mut free = 0;
    let mut total = 0;
    let status = unsafe { cuda_core::sys::cuMemGetInfo_v2(&mut free, &mut total) };
    if status != 0 {
        return Err(format!("cuMemGetInfo: {status}").into());
    }
    Ok(total - free)
}

fn invalid(message: impl Into<String>) -> Box<dyn Error> {
    Box::new(io::Error::new(io::ErrorKind::InvalidInput, message.into()))
}

fn config(items: usize) -> LaunchConfig1D {
    LaunchConfig1D::new(items.div_ceil(THREADS as usize) as u32, THREADS, 0)
}

fn ptr<T: DeviceCopy>(buffer: &DeviceBuffer<T>) -> *mut T {
    buffer.cu_deviceptr() as *mut T
}

// Every callsite initializes the complete allocation on its stream before
// reading it. Avoid a redundant memset for fully overwritten output columns.
fn allocate<T: DeviceCopy>(stream: &CudaStream, len: usize) -> BuildResult<DeviceBuffer<T>> {
    let bytes = len.checked_mul(core::mem::size_of::<T>())
        .filter(|&bytes| bytes <= isize::MAX as usize)
        .ok_or_else(|| invalid("device allocation size overflow"))?;
    let address = if bytes == 0 { 0 } else {
        unsafe { cuda_core::memory::malloc_sync(bytes)? }
    };
    // SAFETY: the allocation belongs to the already-bound stream context and
    // transfers its sole ownership to this buffer, including on error paths.
    Ok(unsafe { DeviceBuffer::from_raw_parts(address, len, stream.context().clone()) })
}

#[cuda_device::cuda_module]
mod kernels {
    use cuda_device::{device, kernel, launch_bounds, launch_contract, thread, warp, DynamicSharedArray};
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU32, DeviceAtomicU64};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn extrema(keys: &[i64], output: *mut u64) {
        let i = thread::index_1d().get();
        let mut low = if i < keys.len() { (keys[i] as u64) ^ (1u64 << 63) } else { u64::MAX };
        let mut high = if i < keys.len() { low } else { 0 };
        let lane = warp::lane_id();
        let mut distance = 1;
        while distance < 32 {
            let previous_low = warp::shuffle_up(low as u32, distance) as u64
                | ((warp::shuffle_up((low >> 32) as u32, distance) as u64) << 32);
            let previous_high = warp::shuffle_up(high as u32, distance) as u64
                | ((warp::shuffle_up((high >> 32) as u32, distance) as u64) << 32);
            if lane >= distance {
                low = low.min(previous_low);
                high = high.max(previous_high);
            }
            distance <<= 1;
        }
        if lane == 31 {
            unsafe {
                DeviceAtomicU64::from_ptr(output).fetch_min(low, AtomicOrdering::Relaxed);
                DeviceAtomicU64::from_ptr(output.add(1)).fetch_max(high, AtomicOrdering::Relaxed);
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn histogram(keys: &[i64], min_key: i64, counts: *mut u32) {
        let i = thread::index_1d().get();
        if i < keys.len() {
            let bin = (keys[i] as u64).wrapping_sub(min_key as u64) as usize;
            unsafe {
                DeviceAtomicU32::from_ptr(counts.add(bin)).fetch_add(1, AtomicOrdering::Relaxed);
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1), dynamic_shared_range = (32, 32))]
    pub unsafe fn scan_blocks(values: *mut u32, length: usize, totals: *mut u32) {
        let i = thread::index_1d().get();
        let value = if i < length { unsafe { values.add(i).read() } } else { 0 };
        let scratch = DynamicSharedArray::<u8>::get_raw() as *mut u32;
        let (prefix, total) = gpu_tiles::scan(value, scratch);
        if i < length {
            unsafe { values.add(i).write(prefix); }
        }
        if thread::threadIdx_x() == 0 {
            unsafe { totals.add(i / 256).write(total); }
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
    pub unsafe fn directory(
        bins: &[u32], rows: u32, bits: *mut u64, prefix: *mut u32, maximum: *mut u32,
    ) {
        let word = thread::index_1d().get();
        if word < (bins.len() + 63) / 64 {
            let mut bits_in_word = 0u64;
            let mut max_count = 0;
            let mut bit = 0;
            while bit < 64 && word * 64 + bit < bins.len() {
                let bin = word * 64 + bit;
                let end = if bin + 1 < bins.len() { bins[bin + 1] } else { rows };
                let count = end - bins[bin];
                if count != 0 {
                    bits_in_word |= 1u64 << bit;
                    max_count = max_count.max(count);
                }
                bit += 1;
            }
            unsafe {
                bits.add(word).write(bits_in_word);
                prefix.add(word).write(bits_in_word.count_ones());
                DeviceAtomicU32::from_ptr(maximum).fetch_max(max_count, AtomicOrdering::Relaxed);
            }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn scatter(
        keys: &[i64], min_key: i64, bins: &[u32], cursors: *mut u32, permutation: *mut u32,
    ) {
        let i = thread::index_1d().get();
        if i < keys.len() {
            let bin = (keys[i] as u64).wrapping_sub(min_key as u64) as usize;
            unsafe {
                let slot = DeviceAtomicU32::from_ptr(cursors.add(bin))
                    .fetch_add(1, AtomicOrdering::Relaxed);
                permutation.add((bins[bin] + slot) as usize).write(i as u32);
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
            unsafe { values.add(root).write(child_value); }
            root = child;
        }
        unsafe { values.add(root).write(value); }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn order_bins(
        bins: &[u32], rows: u32, min_key: i64, bits: &[u64], prefix: &[u32],
        permutation: *mut u32, offsets: *mut u32, unique: *mut i64,
    ) {
        let bin = thread::index_1d().get();
        if bin < bins.len() {
            let begin = bins[bin];
            let end = if bin + 1 < bins.len() { bins[bin + 1] } else { rows };
            if begin != end {
                let mask = 1u64 << (bin & 63);
                let rank = prefix[bin >> 6] + (bits[bin >> 6] & (mask - 1)).count_ones();
                unsafe {
                    offsets.add(rank as usize).write(begin);
                    unique.add(rank as usize).write((min_key as u64).wrapping_add(bin as u64) as i64);
                    if bin + 1 == bins.len() {
                        offsets.add(rank as usize + 1).write(rows);
                    }
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
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn gather(input: &[i64], permutation: &[u32], output: *mut i64) {
        let i = thread::index_1d().get();
        if i < permutation.len() {
            unsafe { output.add(i).write(input[permutation[i] as usize]); }
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub unsafe fn fences(keys: &[i64], output: *mut i64) {
        let i = thread::index_1d().get();
        if i < (keys.len() + super::BLOCK_ROWS - 1) / super::BLOCK_ROWS {
            unsafe { output.add(i).write(keys[i * super::BLOCK_ROWS]); }
        }
    }
}
