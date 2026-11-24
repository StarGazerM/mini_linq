//! Exact projected-set admission over encoded device columns.
//!
//! One stable row-hash radix sort bounds candidate comparisons independently of
//! tuple width. Hash equality is never evidence of tuple equality: each row is
//! compared with preceding candidates in its entire equal-hash run. Ordinary
//! duplicate runs stop at the first matching tuple; adversarial distinct hash
//! collisions can require quadratic comparisons, but cannot change the answer.
//! Stable flagged selection restores original row order. Only one scalar count
//! crosses back to the host; values, hashes and positions remain on the device.
use crate::{Buffer, ColumnView, Pool, Result};
use cuda_core::{CudaContext, DeviceCopy};
use rccl_cub::{DeviceArithmeticRange, DeviceSliceRead, DeviceSliceWrite};
use std::{cell::RefCell, mem::size_of, rc::Rc, sync::Arc};

#[path = "unique_kernels.rs"]
mod kernels;

struct Modules {
    rows: kernels::rows::LoadedModule,
    cub: rccl_cub::cuda::Program<'static>,
    controls: RefCell<Option<Controls>>,
    context: Arc<CudaContext>,
}

type ControlBuffer<T> = rccl_memory::DeviceBuffer<'static, T, &'static crate::MemoryResource>;
struct Controls {
    workspace: rccl_cub::DeviceRadixSortBufferWorkspace<
        ControlBuffer<u64>, ControlBuffer<i32>, ControlBuffer<i32>,
    >,
    rows: usize,
    pool: Arc<Pool>,
}
thread_local! {
    static MODULES: RefCell<Option<Rc<Modules>>> = const { RefCell::new(None) };
}
fn modules(pool: &Pool) -> Result<Rc<Modules>> {
    MODULES.with(|cache| {
        if let Some(modules) = cache.borrow().as_ref() {
            if Arc::ptr_eq(&modules.context, pool.context()) {
                return Ok(modules.clone());
            }
        }
        let context = pool.context().clone();
        let rows = unsafe { kernels::rows::load(&context)? };
        let cub = rccl_cub::cuda::Program::load(&context)?;
        // SAFETY: Program borrows the stable CudaContext pointee, not its Arc
        // handle. This private owner retains that context and drops it last.
        // Every launch borrows this owner, so the extended lifetime cannot escape.
        let cub = unsafe {
            std::mem::transmute::<rccl_cub::cuda::Program<'_>, rccl_cub::cuda::Program<'static>>(cub)
        };
        let modules = Rc::new(Modules { rows, cub, controls: RefCell::new(None), context });
        *cache.borrow_mut() = Some(modules.clone());
        Ok(modules)
    })
}

/// Release this thread's uniqueness modules after their work has completed.
pub fn release_cached_kernels() {
    MODULES.with(|cache| cache.borrow_mut().take());
}

// Exclusive pooled buffers satisfy CUB's slice-based radix interface without
// copying row-sized inputs into a second allocator's buffers.
struct SortBuffer<T: DeviceCopy>(Buffer<T>);
impl<T: DeviceCopy> rccl_cub::DeviceRange for SortBuffer<T> {
    fn len(&self) -> usize {
        self.0.len()
    }
}
// SAFETY: the wrapper owns the allocation and exposes exactly its extent.
unsafe impl<T: DeviceCopy> cuda_host::KernelSliceArg for SortBuffer<T> {
    type Elem = T;
    fn cu_deviceptr(&self) -> u64 {
        self.0.cu_deviceptr()
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}
unsafe impl<T: DeviceCopy> cuda_host::KernelSliceArgMut for SortBuffer<T> {}


/// Return first-occurrence row positions, in original input order, or `None`
/// exactly when the projected tuples are already distinct.
///
/// The returned buffer's `len()` is the distinct count, not scratch capacity.
/// With no columns, every nonempty row is the same empty tuple: zero or one row
/// is already distinct, and larger inputs return the single position zero.
/// Rows must be fewer than `u32::MAX`. All work uses `pool.stream()`.
///
/// # Safety
/// Each descriptor must address an initialized encoded column containing at
/// least `rows` logical values in the pool's CUDA context.
/// The caller retains these allocations unchanged until the stream completes,
/// and orders their initialization before this call's work on that stream.
pub unsafe fn unique_rows(
    pool: Arc<Pool>,
    columns: &[ColumnView],
    rows: usize,
) -> Result<Option<Buffer<u32>>> {
    if rows >= u32::MAX as usize {
        return Err("encoded uniqueness exceeds the u32 row-ID domain".into());
    }
    if rows <= 1 {
        return Ok(None);
    }
    if columns.is_empty() {
        return Ok(Some(Buffer::zeroed(pool, 1)?));
    }
    let modules = modules(&pool)?;
    let launch = modules.cub.on(pool.stream())?;
    let descriptors = Buffer::from_host_async(pool.clone(), columns)?;
    // Each kernel/sort initializes precisely the extent its successor reads.
    // Hashes only choose candidate runs, so 32 bits saves eight scratch bytes
    // per source row without weakening exact tuple equality.
    let mut hashes = SortBuffer(unsafe { Buffer::<u32>::uninitialized(pool.clone(), rows)? });
    let mut alternate_hashes = SortBuffer(unsafe { Buffer::<u32>::uninitialized(pool.clone(), rows)? });
    let mut order = SortBuffer(unsafe { Buffer::<u32>::uninitialized(pool.clone(), rows)? });
    let mut alternate_order = SortBuffer(unsafe { Buffer::<u32>::uninitialized(pool.clone(), rows)? });
    modules.rows.hash_rows(
        pool.stream(),
        &modules.rows.prepare_hash_rows(crate::config(rows))?,
        descriptors.ptr(),
        columns.len(),
        rows,
        hashes.0.ptr(),
        order.0.ptr(),
    )?;
    let mut scratch = modules.controls.borrow_mut();
    if scratch.as_ref().is_none_or(|cached| cached.rows < rows || !Arc::ptr_eq(&cached.pool, &pool)) {
        drop(scratch.take());
        let owner = pool.clone();
        // SAFETY: Controls drops its workspace before the stable-address owner.
        let retained: &'static Pool = unsafe { &*Arc::as_ptr(&owner) };
        let workspace = rccl_cub::device_radix_sort_pairs_buffer_workspace_in::<u32, u32, _>(
            retained.resource(), &modules.cub.on(retained.stream())?, rows,
        )?;
        *scratch = Some(Controls { workspace, rows, pool: owner });
    }
    let flags;
    {
        let mut buffers = rccl_cub::DoubleBuffer::paired(
            (&mut hashes, &mut order),
            (&mut alternate_hashes, &mut alternate_order),
        ).ok_or("unequal encoded uniqueness radix buffers")?;
        rccl_cub::device_radix_sort_pairs_buffer(
            &launch,
            &mut buffers,
            0..32,
            &mut scratch.as_mut().unwrap().workspace,
        )?;
        // Release the borrow; retain controls for the next sort on this pool.
        drop(scratch);
        flags = unsafe { Buffer::<u32>::uninitialized(pool.clone(), rows)? };
        let (hashes, order) = buffers.current();
        modules.rows.first_occurrences(
            pool.stream(),
            &modules.rows.prepare_first_occurrences(crate::config(rows))?,
            descriptors.ptr(),
            columns.len(),
            rows,
            hashes.0.ptr(),
            order.0.ptr(),
            flags.ptr(),
        )?;
    }
    // Retire sort storage before selection scratch is allocated. Pool reuse
    // remains stream ordered, including if a later allocation/launch fails.
    drop(hashes);
    drop(alternate_hashes);
    drop(order);
    drop(alternate_order);
    drop(descriptors);
    let positions = unsafe { Buffer::<u32>::uninitialized(pool.clone(), rows)? };
    let mut count = rccl_memory::DeviceBuffer::<u64, _>::zeroed_in(
        pool.resource(), pool.stream(), 1,
    )?;
    let mut workspace = rccl_cub::thrust_integration::compaction_workspace_in(
        pool.resource(), &launch, rows,
    )?;
    rccl_cub::thrust_integration::device_select_flagged(
        &launch,
        DeviceArithmeticRange::new(0_u32, 1_u32),
        DeviceSliceRead::new(flags.ptr() as *const u32),
        DeviceSliceWrite::new(positions.ptr()),
        &mut count,
        rows,
        &mut workspace,
    )?;
    let mut distinct = 0_u64;
    unsafe {
        cuda_core::simt::memory::memcpy_dtoh_async(
            &mut distinct as *mut u64,
            count.cu_deviceptr(),
            size_of::<u64>(),
            pool.stream().cu_stream(),
        )?;
    }
    pool.stream().synchronize()?;
    if distinct as usize == rows {
        Ok(None)
    } else {
        // Selection initialized this prefix. Retain it without a device copy.
        Ok(Some(unsafe { Buffer::shared_slice(Arc::new(positions), 0, distinct as usize)? }))
    }
}
