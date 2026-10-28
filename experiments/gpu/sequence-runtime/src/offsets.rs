//! Device-wide checked exclusive offsets for resident range lengths.
use crate::{release, temporary, Buffer, MemoryResource, Pool, Result};
use cq_gpu_runtime::launch_grid_for_kernel;
use cuda_core::{LaunchConfig1D, PreparedLaunch};
use std::{cell::RefCell, sync::Arc};

thread_local! {
    static SCAN_CACHE: RefCell<Option<ScanCache>> = const { RefCell::new(None) };
}

fn release_query_cache() {
    SCAN_CACHE.with(|cache| cache.borrow_mut().take());
}

// RCCL exposes borrowing programs and owning, stream-borrowing workspaces,
// but no decomposition/reconstruction API for a scan workspace. Keep their
// owner in this private cache. Declaration order drops both borrowed resources
// before the retained pool; none of the extended references escape this module.
struct ScanCache {
    workspace: Option<rccl_cub::DeviceScanWorkspace<'static, &'static MemoryResource, u32>>,
    cub: rccl_cub::cuda::Program<'static>,
    module: offset_kernels::LoadedModule,
    check: Option<(usize, PreparedLaunch<offset_kernels::__check_CudaKernel>)>,
    pool: Arc<Pool>,
}

impl ScanCache {
    fn new(pool: &Arc<Pool>) -> Result<Self> {
        pool.context().bind_to_thread()?;
        let owner = pool.clone();
        // SAFETY: Arc keeps Pool at a stable address. `owner` is moved into
        // the last field, after every resource borrowing it. On construction
        // failure, locals also drop before `owner`. This cache never replaces
        // its owner or returns a reference into it.
        let retained: &'static Pool = unsafe { &*Arc::as_ptr(&owner) };
        let cub = rccl_cub::cuda::Program::load(retained.context())?;
        let module = unsafe { offset_kernels::load(retained.context())? };
        Ok(Self {
            workspace: None,
            cub,
            module,
            check: None,
            pool: owner,
        })
    }

    fn prepare(&mut self, count: usize) -> Result<()> {
        let entries = count + 1; // The public entrypoint checked this extent.
        let required = rccl_cub::device_scan_workspace_counts(entries)?.temporary_items;
        if self.workspace.as_ref().is_none_or(|workspace| {
            workspace.counts().temporary_items < required
        }) {
            // Drop the old allocation first. RCCL returns it to the pool on
            // its retained stream, so growth and subsequent reuse stay ordered.
            drop(self.workspace.take());
            // SAFETY: the same stable-owner/drop-order invariant as in new().
            let retained: &'static Pool = unsafe { &*Arc::as_ptr(&self.pool) };
            self.workspace = Some(rccl_cub::device_scan_workspace_in(
                retained.resource(),
                &self.cub.on(retained.stream())?,
                entries,
            )?);
        }
        if self.check.as_ref().is_none_or(|(rows, _)| *rows != count) {
            let prepared = self.module.prepare_check(LaunchConfig1D::new(1, 256, 0))?;
            let blocks =
                launch_grid_for_kernel(prepared.function(), count.div_ceil(256), 256, 0)?;
            self.check = Some((
                count,
                self.module.prepare_check(LaunchConfig1D::new(blocks, 256, 0))?,
            ));
        }
        Ok(())
    }
}

/// Formal rule GPU-OffsetScan:
/// Scan the initialized `count` range lengths with a zero sentinel into
/// `offsets[..=count]`. Every adjacent decrease in the wrapping u32 prefix
/// flags `meta[2]`, including overflow in the final range; `meta[3]` receives
/// the final prefix. Existing metadata errors are preserved and rejected.
///
/// The buffers must share `pool`; the caller supplies the live row count from
/// metadata, not the allocation capacity. The suffix after the sentinel is
/// unchanged. The result is the checked total, not the number of offsets.
///
/// RCCL performs a device-wide hierarchical CUB scan, followed by a parallel
/// adjacent-prefix check. One eight-byte metadata readback synchronizes the
/// pool stream; callers must record this range-offset synchronization.
///
/// RCCL's public in-place API requires an owned CUDA `DeviceBuffer`, whereas
/// `Buffer` may own a pooled suballocation. Reusing the runtime's native scratch
/// and copying the live prefix in and out avoids an invalid ownership adapter.
/// No range lengths or scanned offsets are downloaded to the host.
///
/// The current thread retains the named module, RCCL program, growing pooled
/// CUB workspace and last live-count check launch for the most recent pool.
/// Pool identity fixes both context and stream. Query-scope cleanup releases
/// them; changing pools drops the old workspace on its original stream.
pub fn scan(
    pool: &Arc<Pool>,
    offsets: &Buffer<u32>,
    count: usize,
    meta: &Buffer<u32>,
) -> Result<usize> {
    let entries = count.checked_add(1).ok_or("range offset extent overflow")?;
    if offsets.len() < entries
        || meta.len() < 4
        || !Arc::ptr_eq(offsets.pool(), pool)
        || !Arc::ptr_eq(meta.pool(), pool)
    {
        return Err("invalid range offset extent or pool".into());
    }
    let bytes = entries
        .checked_mul(std::mem::size_of::<u32>())
        .ok_or("range offset byte extent overflow")?;
    crate::register_query_cleanup(release_query_cache);
    pool.context().bind_to_thread()?;
    SCAN_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.as_ref().is_none_or(|cached| !Arc::ptr_eq(&cached.pool, pool)) {
            drop(cache.take());
            *cache = Some(ScanCache::new(pool)?);
        }
        let cached = cache.as_mut().unwrap();
        cached.prepare(count)?;
        scan_cached(cached, offsets, count, meta, bytes)
    })
}

fn scan_cached(
    cached: &mut ScanCache,
    offsets: &Buffer<u32>,
    count: usize,
    meta: &Buffer<u32>,
    bytes: usize,
) -> Result<usize> {
    let entries = count + 1;
    let input_bytes = bytes - std::mem::size_of::<u32>();
    let stream = cached.pool.stream();
    let launch = cached.cub.on(stream)?;
    let mut scratch = temporary::<u32>(stream, entries)?;
    // Only the live lengths are initialized. Initialize the extra scan input
    // explicitly instead of reading an uninitialized sentinel or capacity.
    unsafe {
        if input_bytes != 0 {
            cuda_core::simt::memory::memcpy_dtod_async(
                scratch.cu_deviceptr(),
                offsets.cu_deviceptr(),
                input_bytes,
                stream.cu_stream(),
            )?;
        }
        cuda_core::simt::memory::memset_d8_async(
            scratch.cu_deviceptr() + input_bytes as u64,
            0,
            std::mem::size_of::<u32>(),
            stream.cu_stream(),
        )?;
    }
    rccl_cub::device_exclusive_sum_in_place(
        &launch,
        &mut scratch,
        cached.workspace.as_mut().unwrap(),
    )?;
    unsafe {
        cuda_core::simt::memory::memcpy_dtod_async(
            offsets.cu_deviceptr(),
            scratch.cu_deviceptr(),
            bytes,
            stream.cu_stream(),
        )?;
    }
    release(stream, scratch)?;
    cached.module.check(
        stream,
        &cached.check.as_ref().unwrap().1,
        offsets.ptr(),
        count,
        meta.ptr(),
    )?;
    let mut status = [0_u32; 2];
    unsafe {
        cuda_core::simt::memory::memcpy_dtoh_async(
            status.as_mut_ptr(),
            meta.cu_deviceptr() + (2 * std::mem::size_of::<u32>()) as u64,
            std::mem::size_of_val(&status),
            stream.cu_stream(),
        )?;
    }
    stream.synchronize()?;
    if status[0] != 0 {
        return Err("Sequence range offset overflow or input failure".into());
    }
    Ok(status[1] as usize)
}

#[cuda_device::cuda_module]
mod offset_kernels {
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn check(offsets: *const u32, count: usize, meta: *mut u32) {
        let mut row = thread::index_1d().get();
        let stride = thread::gridDim_x() as usize * 256;
        unsafe {
            if row == 0 {
                *meta.add(3) = *offsets.add(count);
            }
            while row < count {
                // Each addend is a u32, so any crossing of 2^32 strictly
                // decreases one adjacent prefix, even across multiple wraps.
                if *offsets.add(row + 1) < *offsets.add(row) {
                    crate::device::flag_overflow(meta.add(2));
                }
                row += stride;
            }
        }
    }
}
