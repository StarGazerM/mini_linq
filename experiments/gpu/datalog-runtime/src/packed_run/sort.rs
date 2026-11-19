use super::*;
use crate::{SortedBatch, runs::{Native, read_count}};
use rccl_cub::{DeviceSliceRead, DeviceSliceWrite};

type DeviceBuffer<T> = rccl_memory::DeviceBuffer<'static, T, &'static gpu_device_columns::MemoryResource>;

pub(super) struct OutputBuffer<T: DeviceCopy> { pub(super) buffer: Buffer<T> }
impl<T: DeviceCopy> rccl_cub::DeviceRange for OutputBuffer<T> { fn len(&self) -> usize { self.buffer.len() } }
// SAFETY: this private wrapper owns the allocation and exposes its exact extent.
unsafe impl<T: DeviceCopy> cuda_host::KernelSliceArg for OutputBuffer<T> {
    type Elem = T;
    fn cu_deviceptr(&self) -> u64 { self.buffer.cu_deviceptr() }
    fn len(&self) -> usize { self.buffer.len() }
}
unsafe impl<T: DeviceCopy> cuda_host::KernelSliceArgMut for OutputBuffer<T> {}
impl<T: DeviceCopy> OutputBuffer<T> {
    pub(super) unsafe fn new(pool: Arc<Pool>, rows: usize) -> Result<Self> {
        Ok(Self { buffer: Buffer::uninitialized(pool, rows)? })
    }
}

struct Scratch {
    masks: Buffer<u64>,
    count: Native<u64>,
    radix: rccl_cub::DeviceRadixSortBufferWorkspace<DeviceBuffer<u64>, DeviceBuffer<i32>, DeviceBuffer<i32>>,
    cub: rccl_cub::cuda::Program<'static>,
    capacity: usize,
    // Borrowing resources above must be dropped before their stable owner.
    pool: Arc<Pool>,
}
impl Scratch {
    unsafe fn new(pool: &Arc<Pool>, rows: usize, wide: bool) -> Result<Self> {
        let owner = pool.clone();
        let retained: &'static Pool = &*Arc::as_ptr(&owner);
        let cub = rccl_cub::cuda::Program::load(retained.context())?;
        let launch = cub.on(retained.stream())?;
        let radix = if wide {
            rccl_cub::device_radix_sort_pairs_buffer_workspace_in::<u64, u32, _>(retained.resource(), &launch, rows)?
        } else {
            rccl_cub::device_radix_sort_keys_buffer_workspace_in::<u64, _>(retained.resource(), &launch, rows)?
        };
        Ok(Self { masks: Buffer::uninitialized(pool.clone(), 4)?, count: Native::new(pool, 1)?,
            radix, cub, capacity: rows, pool: owner })
    }
}
thread_local! {
    static SCRATCH: std::cell::RefCell<[Option<Scratch>; 2]> = const { std::cell::RefCell::new([None, None]) };
}
/// Release the experimental sorter cache after stream-ordered admission work.
pub fn release_sort_scratch() { SCRATCH.with(|cache| *cache.borrow_mut() = [None, None]); }

impl<W: Word> PackedSet<W> {
    /// Sort arbitrary candidates using only the varying bit interval of each lane.
    /// Returns sorted unique rows and the number of selected eight-bit digits.
    ///
    /// # Safety
    /// The prefix is initialized with this set's encoding and stream ordering.
    pub unsafe fn sort_varying(&self, input: &Buffer<W>, rows: usize) -> Result<(SortedBatch<W>, u32)> {
        let _sort = cq_gpu_runtime::nvtx::range(c"packed_sort_varying");
        if rows > input.len() || rows > u32::MAX as usize || !self.pool.shares_memory(input.pool()) {
            return Err("invalid varying-digit sort input".into());
        }
        if rows == 0 { return Ok((SortedBatch::from_sorted(Buffer::uninitialized(self.pool.clone(), 0)?, 0)?, 0)); }
        SCRATCH.with(|cache| {
            let mut cache = cache.borrow_mut();
            let slot = &mut cache[(W::BITS > 64) as usize];
            if slot.as_ref().is_none_or(|s| s.capacity < rows || !Arc::ptr_eq(&s.pool, &self.pool)) {
                *slot = Some(Scratch::new(&self.pool, rows, W::BITS > 64)?);
            }
            let s = slot.as_mut().unwrap();
            let lanes = (W::BITS / 64) as usize;
            cuda_core::simt::memory::memset_d8_async(s.masks.cu_deviceptr(), 0, lanes * 8, self.pool.stream().cu_stream())?;
            let cfg = cuda_core::simt::LaunchConfig { grid_dim: (rows.div_ceil(256).min(256).max(1) as u32, 1, 1),
                block_dim: (256, 1, 1), shared_mem_bytes: (lanes * 256 * 8) as u32 };
            self.modules.sort.varying::<W>(self.pool.stream(), cfg, input.ptr(), rows as u64, s.masks.ptr())?;
            let mut masks = [0u64; 4];
            cuda_core::simt::memory::memcpy_dtoh_async(masks.as_mut_ptr(), s.masks.cu_deviceptr(), lanes * 8, self.pool.stream().cu_stream())?;
            self.pool.stream().synchronize()?;
            let digits = masks[..lanes].iter().filter(|&&m| m != 0).map(|m| (64 - m.leading_zeros() - m.trailing_zeros()).div_ceil(8)).sum();
            if digits == 0 {
                let output = Buffer::uninitialized(self.pool.clone(), 1)?;
                cuda_core::simt::memory::memcpy_dtod_async(output.cu_deviceptr(), input.cu_deviceptr(), std::mem::size_of::<W>(), self.pool.stream().cu_stream())?;
                return Ok((SortedBatch::from_sorted(output, 1)?, 0));
            }
            // Row-sized buffers retire after this sort; retaining maxima across
            // all states would add tens of MB to the live pool. Controls grow.
            let mut first_keys = OutputBuffer::<u64>::new(self.pool.clone(), rows)?;
            let mut alternate_keys = OutputBuffer::<u64>::new(self.pool.clone(), rows)?;
            let launch = s.cub.on(self.pool.stream())?;
            let output = Buffer::<W>::uninitialized(self.pool.clone(), rows)?;
            let mut compact = rccl_cub::thrust_integration::compaction_workspace_in(self.pool.resource(), &launch, rows)?;
            if W::BITS == 64 {
                cuda_core::simt::memory::memcpy_dtod_async(first_keys.buffer.cu_deviceptr(), input.cu_deviceptr(), rows * 8, self.pool.stream().cu_stream())?;
                let mut buffers = rccl_cub::DoubleBuffer::new(&mut first_keys, &mut alternate_keys).ok_or("unequal sort buffers")?;
                rccl_cub::device_radix_sort_keys_buffer(&launch, &mut buffers,
                    masks[0].trailing_zeros()..64 - masks[0].leading_zeros(), &mut s.radix)?;
                rccl_cub::thrust_integration::device_unique_copy_by(&launch,
                    DeviceSliceRead::new(buffers.current().buffer.ptr()), DeviceSliceWrite::new(output.ptr().cast::<u64>()),
                    s.count.get_mut(), |a: u64, b: u64| a == b, rows, &mut compact)?;
            } else {
                let mut first_order = OutputBuffer::<u32>::new(self.pool.clone(), rows)?;
                let mut second_order = OutputBuffer::<u32>::new(self.pool.clone(), rows)?;
                let mut buffers = rccl_cub::DoubleBuffer::paired(
                    (&mut first_keys, &mut first_order), (&mut alternate_keys, &mut second_order))
                    .ok_or("unequal paired sort buffers")?;
                let mut first = true;
                for (lane, &mask) in masks[..lanes].iter().enumerate() {
                    if mask == 0 { continue; }
                    let (keys, order) = buffers.current();
                    self.modules.sort.gather::<W>(self.pool.stream(), &self.modules.sort.prepare_gather::<W>(config(rows))?,
                        input.ptr(), rows as u64, lane as u32, first, keys.buffer.ptr(), order.buffer.ptr())?;
                    rccl_cub::device_radix_sort_pairs_buffer(&launch, &mut buffers,
                        mask.trailing_zeros()..64 - mask.leading_zeros(), &mut s.radix)?;
                    first = false;
                }
                let source = input.ptr();
                let out = output.ptr();
                rccl_cub::thrust_integration::device_unique_copy_by(&launch,
                    DeviceSliceRead::new(buffers.current().1.buffer.ptr()),
                    rccl_cub::DeviceIndexedWrite::new(move |index, row: u32| unsafe { *out.add(index) = *source.add(row as usize); }),
                    s.count.get_mut(), move |a: u32, b: u32| unsafe { (*source.add(a as usize)).compare(&*source.add(b as usize)).is_eq() },
                    rows, &mut compact)?;
            }
            let count = read_count(&self.pool, s.count.get())?;
            Ok((SortedBatch::from_sorted(output, count)?, digits))
        })
    }
}

#[cuda_host::cuda_module]
pub(super) mod sort_kernels {
    use crate::Word;
    use cuda_device::{kernel, launch_bounds, launch_contract, thread, DynamicSharedArray};
    use cuda_device::atomic::{AtomicOrdering, DeviceAtomicU64};

    #[kernel]
    #[launch_bounds(256)]
    pub fn varying<W: Word>(input: *const W, rows: u64, masks: *mut u64) {
        let scratch = DynamicSharedArray::<u64>::get();
        let tid = thread::threadIdx_x() as usize;
        unsafe {
            let first = *input;
            let mut local = [0u64; 4];
            let mut i = thread::index_1d().get() as u64;
            while i < rows {
                let key = *input.add(i as usize);
                let mut lane = 0;
                while lane < W::BITS / 64 { local[lane as usize] |= key.lane(lane) ^ first.lane(lane); lane += 1; }
                i += thread::gridDim_x() as u64 * 256;
            }
            let mut lane = 0;
            while lane < W::BITS / 64 { *scratch.add(lane as usize * 256 + tid) = local[lane as usize]; lane += 1; }
            thread::sync_threads();
            let mut distance = 128;
            while distance != 0 {
                if tid < distance {
                    lane = 0;
                    while lane < W::BITS / 64 {
                        let p = scratch.add(lane as usize * 256 + tid);
                        *p |= *p.add(distance);
                        lane += 1;
                    }
                }
                thread::sync_threads();
                distance /= 2;
            }
            if tid == 0 {
                lane = 0;
                while lane < W::BITS / 64 {
                    DeviceAtomicU64::from_ptr(masks.add(lane as usize)).fetch_or(*scratch.add(lane as usize * 256), AtomicOrdering::Relaxed);
                    lane += 1;
                }
            }
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn gather<W: Word>(input: *const W, rows: u64, lane: u32, first: bool, keys: *mut u64, order: *mut u32) {
        let mut i = thread::index_1d().get() as u64;
        while i < rows {
            unsafe {
                let index = if first { i as u32 } else { *order.add(i as usize) };
                *keys.add(i as usize) = (*input.add(index as usize)).lane(lane);
                *order.add(i as usize) = index;
            }
            i += thread::gridDim_x() as u64 * 256;
        }
    }
}
