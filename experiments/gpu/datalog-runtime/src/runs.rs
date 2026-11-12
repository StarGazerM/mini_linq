use crate::{Result, Word};
use cuda_core::{DeviceBuffer, DeviceCopy, LaunchConfig1D};
use gpu_device_columns::{Buffer, Pool};
use rccl_cub::{DeviceSliceRead, DeviceSliceWrite};
use std::sync::Arc;

/// A borrowed kernel ABI. Its owner must outlive all stream-ordered reads.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub(crate) struct RunView<W: Word> {
    pub ptr: *const W,
    pub len: usize,
}
// SAFETY: this C-layout descriptor contains only a device pointer and length.
unsafe impl<W: Word> DeviceCopy for RunView<W> {}


/// Cheap shared ownership of an immutable sorted, unique initialized prefix.
#[derive(Clone)]
pub struct SortedBatch<W: Word> {
    buffer: Arc<Buffer<W>>,
    rows: usize,
}
impl<W: Word> SortedBatch<W> {
    /// # Safety
    /// The prefix must be initialized, sorted and unique on its pool's stream.
    /// No alias may mutate its words for the lifetime of this run or its clones.
    pub unsafe fn from_sorted(buffer: Buffer<W>, rows: usize) -> Result<Self> {
        if rows > buffer.len() {
            return Err("run length exceeds buffer capacity".into());
        }
        // An empty immutable prefix need not keep its entire rejected candidate
        // allocation alive. Releasing it is stream ordered and copies no rows.
        let buffer = if rows == 0 && !buffer.is_empty() {
            let pool = buffer.pool().clone();
            drop(buffer);
            unsafe { Buffer::uninitialized(pool, 0)? }
        } else {
            buffer
        };
        let buffer = Arc::new(buffer);
        // SAFETY: the caller publishes an immutable initialized prefix.
        let buffer = if rows == buffer.len() {
            buffer
        } else {
            Arc::new(unsafe { Buffer::shared_slice(buffer, 0, rows)? })
        };
        Ok(Self { buffer, rows })
    }
    pub fn len(&self) -> usize {
        self.rows
    }
    pub fn is_empty(&self) -> bool {
        self.rows == 0
    }
    /// Initialized packed-row bytes, excluding retained spare capacity.
    pub fn live_bytes(&self) -> usize {
        self.rows * std::mem::size_of::<W>()
    }
    /// Full backing allocation retained by this run, including a sliced suffix.
    /// Clones and layouts may retain the same allocation; do not sum aliases.
    pub fn allocated_bytes(&self) -> usize {
        self.buffer.allocated_bytes()
    }
    pub fn reserved_capacity(&self) -> usize {
        self.allocated_bytes() / std::mem::size_of::<W>()
    }
    pub fn buffer(&self) -> &Buffer<W> {
        &self.buffer
    }
    pub(crate) fn shared_buffer(&self) -> Result<Buffer<W>> {
        // SAFETY: the owner retains this immutable initialized prefix.
        unsafe { Buffer::shared_slice(self.buffer.clone(), 0, self.rows) }
    }
    pub(crate) fn view(&self) -> RunView<W> {
        RunView {
            ptr: self.buffer.ptr(),
            len: self.rows,
        }
    }
}
// SAFETY: the retained immutable buffer owns the reported initialized prefix.
unsafe impl<W: Word> cuda_host::KernelSliceArg for SortedBatch<W> {
    type Elem = W;
    fn cu_deviceptr(&self) -> cuda_core::sys::CUdeviceptr {
        self.buffer.cu_deviceptr()
    }
    fn len(&self) -> usize {
        self.rows
    }
}

// Only unpublished output storage implements the writable kernel ABI.
struct Output<W: Word>(Buffer<W>);
// SAFETY: this affine wrapper owns the entire allocation.
unsafe impl<W: Word> cuda_host::KernelSliceArg for Output<W> {
    type Elem = W;
    fn cu_deviceptr(&self) -> cuda_core::sys::CUdeviceptr {
        self.0.cu_deviceptr()
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}
// SAFETY: mutable access is exclusive and output is unpublished until launch.
unsafe impl<W: Word> cuda_host::KernelSliceArgMut for Output<W> {}

/// Shared stateless full-key sorting, compaction, subtraction and merge kernels.
/// Accepted state belongs exclusively to DeviceRadixRun.
pub(crate) struct SetOps<'a> {
    pub pool: &'a Arc<Pool>,
    pub module: &'a run_kernels::LoadedModule,
    pub params: crate::RadixRunParams,
    pub active_bits: u32,
}
impl SetOps<'_> {
    /// Formal rule DL-SortKnownUnique: certified injective full-key permutations
    /// skip unique compaction and count readback, but never skip sorting.
    pub(crate) fn sort<W: Word>(&self, input: &Buffer<W>, rows: usize, known_unique: bool, validation: Option<Native<u64>>) -> Result<SortedBatch<W>> {
        if !self.pool.shares_memory(input.pool()) || rows > input.len() || rows > u32::MAX as usize
        {
            return Err("invalid sort input pool or live length".into());
        }
        if self.active_bits > W::BITS {
            return Err("sort active bits exceed the key word".into());
        }
        if validation.is_some() && (known_unique || rows == 0 || self.active_bits == 0) {
            return Err("pending domain validation requires the existing unique-count barrier".into());
        }
        crate::stats::record_sort(
            if known_unique { crate::stats::SortKind::Layout } else { crate::stats::SortKind::Canonical },
            rows, self.active_bits, W::BITS != 64,
        );
        if rows == 0 || self.active_bits == 0 {
            // Zero-bit packing has exactly one possible value. This decision
            // depends only on the recorded width, never the candidate values.
            let rows = rows.min(1);
            return unsafe { SortedBatch::from_sorted(Buffer::zeroed(self.pool.clone(), rows)?, rows) };
        }
        // SAFETY: kernels initialize the returned prefix before publication.
        let cub = rccl_cub::cuda::Program::load(self.pool.context())?;
        let launch = cub.on(self.pool.stream())?;
        let checked = validation.is_some();
        let mut compaction = if known_unique {
            None
        } else {
            Some((
                match validation { Some(control) => control, None => Native::<u64>::new(&self.pool, 1)? },
                rccl_cub::thrust_integration::compaction_workspace_in(
                    self.pool.resource(),
                    &launch,
                    rows,
                )?,
            ))
        };
        if W::BITS == 64 {
            let sorted = stable_keys(&self.pool, &launch, input, rows, self.active_bits)?;
            if let Some((count, compact)) = &mut compaction {
                let output = unsafe { Buffer::<W>::uninitialized(self.pool.clone(), rows)? };
                rccl_cub::thrust_integration::device_unique_copy_by(
                    &launch,
                    DeviceSliceRead::new(sorted.ptr().cast::<u64>() as *const u64),
                    DeviceSliceWrite::new(output.ptr().cast::<u64>()),
                    count.get_mut(),
                    |a: u64, b: u64| a == b,
                    rows,
                    compact,
                )?;
                let rows = read_count_validated(&self.pool, count.get(), checked)?;
                return unsafe { SortedBatch::from_sorted(output, rows) };
            }
            // Adopt the actual sorted buffer: no post-sort copy or spare output.
            return unsafe { SortedBatch::from_sorted(sorted, rows) };
        }
        let output = unsafe { Buffer::<W>::uninitialized(self.pool.clone(), rows)? };
        {
            // The permutation owner lives until this branch has enqueued its consumer.
            let permutation = stable_order(
                &self.pool,
                &self.module,
                &launch,
                input,
                rows,
                self.params.block_threads,
                self.active_bits,
            )?;
            let order = permutation.get().cu_deviceptr() as *const u32;
            if let Some((count, compact)) = &mut compaction {
                let source = input.ptr();
                let words =
                    rccl_cub::DeviceMap::new(DeviceSliceRead::new(order), move |row: u32| unsafe {
                        *source.add(row as usize)
                    });
                rccl_cub::thrust_integration::device_unique_copy_by(
                    &launch,
                    words,
                    DeviceSliceWrite::new(output.ptr()),
                    count.get_mut(),
                    |a: W, b: W| a.compare(&b).is_eq(),
                    rows,
                    compact,
                )?;
            } else {
                let initial = self.module.prepare_gather_sorted::<W>(LaunchConfig1D::new(
                    1,
                    self.params.block_threads,
                    0,
                ))?;
                let grid = cq_gpu_runtime::launch_grid_for_kernel(
                    initial.function(),
                    rows.div_ceil(self.params.block_threads as usize),
                    self.params.block_threads,
                    0,
                )?;
                let config = LaunchConfig1D::new(grid, self.params.block_threads, 0);
                self.module.gather_sorted::<W>(
                    self.pool.stream(),
                    &self.module.prepare_gather_sorted::<W>(config)?,
                    input.ptr(),
                    rows as u32,
                    order,
                    output.ptr(),
                )?;
            }
        }
        let rows = if let Some((count, _)) = &compaction {
            read_count_validated(&self.pool, count.get(), checked)?
        } else {
            rows
        };
        // SAFETY: compaction established uniqueness, or the caller guaranteed it.
        // Both paths initialize the complete sorted prefix before publication.
        unsafe { SortedBatch::from_sorted(output, rows) }
    }
    pub(crate) fn subtract<W: Word>(&self, unique: &SortedBatch<W>, accepted: &[RunView<W>]) -> Result<SortedBatch<W>> {
        let rows = unique.len();
        let descriptors = Buffer::from_host_async(self.pool.clone(), accepted)?;
        crate::stats::record_work(|work| {
            work.descriptor_materializations += 1;
            work.descriptor_bytes += (accepted.len() * std::mem::size_of::<RunView<W>>()) as u64;
        });
        // SAFETY: the flag kernel and stable select initialize these outputs.
        let flags = unsafe { Buffer::<u32>::uninitialized(self.pool.clone(), rows)? };
        let output = unsafe { Buffer::<W>::uninitialized(self.pool.clone(), rows)? };
        let initial = self.module.prepare_absent::<W>(LaunchConfig1D::new(
            1,
            self.params.block_threads,
            0,
        ))?;
        let grid = cq_gpu_runtime::launch_grid_for_kernel(
            initial.function(),
            rows.div_ceil(self.params.block_threads as usize),
            self.params.block_threads,
            0,
        )?;
        let config = LaunchConfig1D::new(grid, self.params.block_threads, 0);
        self.module.absent::<W>(
            self.pool.stream(),
            &self.module.prepare_absent::<W>(config)?,
            unique.buffer.ptr(),
            rows as u64,
            descriptors.ptr(),
            accepted.len() as u32,
            flags.ptr(),
        )?;
        let cub = rccl_cub::cuda::Program::load(self.pool.context())?;
        let launch = cub.on(self.pool.stream())?;
        let mut count = Native::<u64>::new(&self.pool, 1)?;
        let mut workspace = rccl_cub::thrust_integration::compaction_workspace_in(
            self.pool.resource(),
            &launch,
            rows,
        )?;
        rccl_cub::thrust_integration::device_select_flagged(
            &launch,
            DeviceSliceRead::new(unique.buffer.ptr()),
            DeviceSliceRead::new(flags.ptr()),
            DeviceSliceWrite::new(output.ptr()),
            count.get_mut(),
            rows,
            &mut workspace,
        )?;
        let fresh = read_count(&self.pool, count.get())?;
        // SAFETY: stable selection preserves sorted uniqueness of the input.
        unsafe { SortedBatch::from_sorted(output, fresh) }
    }
    pub(crate) fn merge<W: Word>(&self, first: &SortedBatch<W>, second: &SortedBatch<W>) -> Result<SortedBatch<W>> {
        let rows = first
            .len()
            .checked_add(second.len())
            .ok_or("merged length overflow")?;
        crate::stats::record_work(|work| work.merges += 1);
        // SAFETY: merge writes every output element before publication.
        let mut output = Output(unsafe { Buffer::uninitialized(self.pool.clone(), rows)? });
        if W::BITS > 128 {
            let tiles = rows.div_ceil(self.params.merge_tile_rows as usize);
            let initial = self.module.prepare_merge_wide::<W>(LaunchConfig1D::new(
                1,
                self.params.block_threads,
                0,
            ))?;
            let grid = cq_gpu_runtime::launch_grid_for_kernel(
                initial.function(),
                tiles.div_ceil(self.params.block_threads as usize),
                self.params.block_threads,
                0,
            )?;
            let config = LaunchConfig1D::new(grid, self.params.block_threads, 0);
            self.module.merge_wide::<W>(
                self.pool.stream(),
                &self.module.prepare_merge_wide::<W>(config)?,
                first.view(),
                second.view(),
                output.0.ptr(),
                self.params.merge_tile_rows as usize,
            )?;
            // SAFETY: disjoint merge-path tiles initialize the sorted union.
            return unsafe { SortedBatch::from_sorted(output.0, rows) };
        }
        let cub = rccl_cub::cuda::Program::load(self.pool.context())?;
        let launch = cub.on(self.pool.stream())?;
        let partitions = rccl_cub::thrust_integration::device_merge_partition_count::<W>(
            first.len(),
            second.len(),
        )?;
        let mut partitions = Native::<usize>::new(&self.pool, partitions)?;
        let mut workspace = rccl_cub::DeviceMergeWorkspace::__from_partitions(partitions.get_mut());
        rccl_cub::device_merge_keys(
            &launch,
            first,
            second,
            &mut workspace,
            &mut output,
            |a: &W, b: &W| a.compare(b),
        )?;
        // SAFETY: merging disjoint sorted unique runs produces a sorted unique run.
        unsafe { SortedBatch::from_sorted(output.0, rows) }
    }
}

// Native RCCL count/control buffers need stream-ordered destruction too.
pub(crate) struct Native<T: DeviceCopy> {
    buffer: Option<DeviceBuffer<T>>,
    pool: Arc<Pool>,
}
impl<T: DeviceCopy> Native<T> {
    pub(crate) fn new(pool: &Arc<Pool>, rows: usize) -> Result<Self> {
        // SAFETY: every consumer initializes scratch before reading it.
        Ok(Self {
            buffer: Some(unsafe { DeviceBuffer::uninitialized_async(pool.stream(), rows)? }),
            pool: pool.clone(),
        })
    }
    pub(crate) fn get(&self) -> &DeviceBuffer<T> {
        self.buffer.as_ref().unwrap()
    }
    pub(crate) fn get_mut(&mut self) -> &mut DeviceBuffer<T> {
        self.buffer.as_mut().unwrap()
    }
}
impl<T: DeviceCopy> Drop for Native<T> {
    fn drop(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            // SAFETY: all accesses were enqueued on this pool stream.
            if let Err(error) = unsafe { buffer.drop_async(self.pool.stream()) } {
                self.pool.context().record_err::<()>(Err(error));
            }
        }
    }
}
struct RadixScratch {
    workspace: rccl_cub::DeviceRadixSortBufferWorkspace,
    pool: Arc<Pool>,
}
impl Drop for RadixScratch {
    fn drop(&mut self) {
        fn release<T: DeviceCopy>(pool: &Pool, buffer: &mut DeviceBuffer<T>) {
            // SAFETY: zero-length replacement owns no allocation; old uses are stream ordered.
            let empty = unsafe { DeviceBuffer::uninitialized_async(pool.stream(), 0) }
                .expect("empty allocation");
            let old = std::mem::replace(buffer, empty);
            if let Err(error) = unsafe { old.drop_async(pool.stream()) } {
                pool.context().record_err::<()>(Err(error));
            }
        }
        let (bins, lookback, counters) = self.workspace.__parts_mut();
        release(&self.pool, bins);
        release(&self.pool, lookback);
        release(&self.pool, counters);
    }
}

/// A live prefix of exclusively borrowed scratch. Capacity is not sort length.
struct SortPrefix<'a, T: DeviceCopy> {
    buffer: &'a mut DeviceBuffer<T>,
    rows: usize,
}
impl<'a, T: DeviceCopy> SortPrefix<'a, T> {
    fn new(buffer: &'a mut Native<T>, rows: usize) -> Self {
        assert!(rows <= buffer.get().len());
        Self {
            buffer: buffer.get_mut(),
            rows,
        }
    }
    fn ptr(&self) -> u64 {
        self.buffer.cu_deviceptr()
    }
}
impl<T: DeviceCopy> rccl_cub::DeviceRange for SortPrefix<'_, T> {
    fn len(&self) -> usize {
        self.rows
    }
}
// SAFETY: the checked prefix retains an exclusive borrow of its live owner.
unsafe impl<T: DeviceCopy> cuda_host::KernelSliceArg for SortPrefix<'_, T> {
    type Elem = T;
    fn cu_deviceptr(&self) -> u64 {
        self.ptr()
    }
    fn len(&self) -> usize {
        self.rows
    }
}
unsafe impl<T: DeviceCopy> cuda_host::KernelSliceArgMut for SortPrefix<'_, T> {}

pub(crate) struct KeySortScratch {
    first: Native<u64>,
    alternate: Native<u64>,
    controls: RadixScratch,
}
impl KeySortScratch {
    pub(crate) fn new(
        pool: &Arc<Pool>,
        launch: &rccl_cub::cuda::Launch<'_, '_>,
        rows: usize,
    ) -> Result<Self> {
        Ok(Self {
            first: Native::new(pool, rows)?,
            alternate: Native::new(pool, rows)?,
            controls: RadixScratch {
                workspace: rccl_cub::device_radix_sort_keys_buffer_workspace::<u64>(launch, rows)?,
                pool: pool.clone(),
            },
        })
    }
    pub(crate) fn capacity(&self) -> usize {
        self.first.get().len()
    }

    pub(crate) fn sort(
        &mut self,
        pool: &Arc<Pool>,
        launch: &rccl_cub::cuda::Launch<'_, '_>,
        input: *const u64,
        rows: usize,
        bits: u32,
    ) -> Result<*const u64> {
        unsafe {
            cuda_core::simt::memory::memcpy_dtod_async(
                self.first.get().cu_deviceptr(),
                input as u64,
                rows * 8,
                pool.stream().cu_stream(),
            )?;
        }
        let mut first = SortPrefix::new(&mut self.first, rows);
        let mut alternate = SortPrefix::new(&mut self.alternate, rows);
        let mut buffers = rccl_cub::DoubleBuffer::new(&mut first, &mut alternate)
            .ok_or("unequal scalar radix buffers")?;
        rccl_cub::device_radix_sort_keys_buffer(
            launch,
            &mut buffers,
            0..bits,
            &mut self.controls.workspace,
        )?;
        Ok(buffers.current().ptr() as *const u64)
    }
}

pub(crate) struct RowSortScratch {
    first: Native<u64>,
    alternate: Native<u64>,
    indices: Native<u32>,
    alternate_indices: Native<u32>,
    controls: RadixScratch,
}
impl RowSortScratch {
    pub(crate) fn new(
        pool: &Arc<Pool>,
        launch: &rccl_cub::cuda::Launch<'_, '_>,
        rows: usize,
    ) -> Result<Self> {
        Ok(Self {
            first: Native::new(pool, rows)?,
            alternate: Native::new(pool, rows)?,
            indices: Native::new(pool, rows)?,
            alternate_indices: Native::new(pool, rows)?,
            controls: RadixScratch {
                workspace: rccl_cub::device_radix_sort_pairs_buffer_workspace::<u64, u32>(
                    launch, rows,
                )?,
                pool: pool.clone(),
            },
        })
    }
    pub(crate) fn capacity(&self) -> usize {
        self.first.get().len()
    }

    pub(crate) fn sort(
        &mut self,
        pool: &Arc<Pool>,
        module: &run_kernels::LoadedModule,
        launch: &rccl_cub::cuda::Launch<'_, '_>,
        input: *const u64,
        rows: usize,
        bits: u32,
        physical_lanes: u32,
        ms_first: bool,
        block_threads: u32,
    ) -> Result<*const u32> {
        if rows > u32::MAX as usize || bits == 0 || bits.div_ceil(64) > physical_lanes
            || block_threads != 256
        {
            return Err("invalid stable row sort dimensions or launch width".into());
        }
        let lanes = bits.div_ceil(64);
        let mut first = SortPrefix::new(&mut self.first, rows);
        let mut alternate = SortPrefix::new(&mut self.alternate, rows);
        let mut indices = SortPrefix::new(&mut self.indices, rows);
        let mut alternate_indices = SortPrefix::new(&mut self.alternate_indices, rows);
        let mut buffers = rccl_cub::DoubleBuffer::paired(
            (&mut first, &mut indices),
            (&mut alternate, &mut alternate_indices),
        )
        .ok_or("unequal lane radix buffers")?;
        let initial = module.prepare_gather_lane(LaunchConfig1D::new(1, block_threads, 0))?;
        let grid = cq_gpu_runtime::launch_grid_for_kernel(
            initial.function(),
            rows.div_ceil(block_threads as usize),
            block_threads,
            0,
        )?;
        let prepared = module.prepare_gather_lane(LaunchConfig1D::new(grid, block_threads, 0))?;
        // Stable least-significant-lane passes retain full-row order.
        for lane in 0..lanes {
            let (values, order) = buffers.current_mut();
            module.gather_lane(
                pool.stream(),
                &prepared,
                input,
                rows as u32,
                lane,
                physical_lanes,
                ms_first,
                values.ptr() as *mut u64,
                order.ptr() as *mut u32,
            )?;
            rccl_cub::device_radix_sort_pairs_buffer(
                launch,
                &mut buffers,
                0..(bits - lane * 64).min(64),
                &mut self.controls.workspace,
            )?;
        }
        Ok(buffers.current().1.ptr() as *const u32)
    }
}
pub(crate) fn read_count(pool: &Pool, buffer: &DeviceBuffer<u64>) -> Result<usize> {
    let mut count = 0_u64;
    // SAFETY: preceding compaction initialized one u64; stack storage survives synchronization.
    unsafe {
        cuda_core::simt::memory::memcpy_dtoh_async(
            &mut count as *mut u64,
            buffer.cu_deviceptr(),
            8,
            pool.stream().cu_stream(),
        )?;
    }
    crate::record_readback();
    pool.stream().synchronize()?;
    Ok(usize::try_from(count)?)
}

/// Consume a pending domain error beside the count in one existing transfer.
/// The ordinary path keeps its original one-word readback unchanged.
fn read_count_validated(pool: &Pool, count: &DeviceBuffer<u64>, checked: bool) -> Result<usize> {
    if !checked { return read_count(pool, count); }
    let mut result = [0u64; 2];
    // SAFETY: validation and compaction initialized this shared control buffer.
    unsafe {
        cuda_core::simt::memory::memcpy_dtoh_async(
            result.as_mut_ptr(), count.cu_deviceptr(), 16, pool.stream().cu_stream(),
        )?;
    }
    crate::record_readback();
    pool.stream().synchronize()?;
    if result[1] != 0 {
        return Err("candidate value is outside its recorded equality component".into());
    }
    Ok(usize::try_from(result[0])?)
}

/// The stable sorting permutation of an initialized packed-word prefix.
///
/// Least-significant-lane-first stable radix passes over row indices keep
/// equal words in input order, so the permutation also orders occurrences.
/// Shared by full-key set canonicalization and occurrence-preserving grouped radix runs.
pub(crate) fn stable_order<W: Word>(
    pool: &Arc<Pool>,
    module: &run_kernels::LoadedModule,
    launch: &rccl_cub::cuda::Launch<'_, '_>,
    input: &Buffer<W>,
    rows: usize,
    block_threads: u32,
    active_bits: u32,
) -> Result<Native<u32>> {
    stable_row_order(
        pool,
        module,
        launch,
        input.ptr().cast(),
        rows,
        active_bits,
        W::BITS / 64,
        W::MS_FIRST,
        block_threads,
    )
}

/// Stable row ordering shared by typed words and runtime-sized encoded rows.
/// `bits` records significant bits; padding above it must be zero.
/// `physical_lanes` is the row stride, independent of the active lane count.
pub(crate) fn stable_row_order(
    pool: &Arc<Pool>,
    module: &run_kernels::LoadedModule,
    launch: &rccl_cub::cuda::Launch<'_, '_>,
    input: *const u64,
    rows: usize,
    bits: u32,
    physical_lanes: u32,
    ms_first: bool,
    block_threads: u32,
) -> Result<Native<u32>> {
    let mut scratch = RowSortScratch::new(pool, launch, rows)?;
    let current = scratch.sort(
        pool,
        module,
        launch,
        input,
        rows,
        bits,
        physical_lanes,
        ms_first,
        block_threads,
    )?;
    Ok(if current as u64 == scratch.indices.get().cu_deviceptr() {
        scratch.indices
    } else {
        scratch.alternate_indices
    })
}

/// Affine pooled output reinterpreted as scalar lanes only after checking that
/// the sealed Word type occupies exactly one u64 (u64 or [u64; 1]).
struct ScalarOutput<W: Word>(Buffer<W>);
impl<W: Word> rccl_cub::DeviceRange for ScalarOutput<W> {
    fn len(&self) -> usize { self.0.len() }
}
// SAFETY: only stable_keys constructs this wrapper, after checking its layout.
unsafe impl<W: Word> cuda_host::KernelSliceArg for ScalarOutput<W> {
    type Elem = u64;
    fn cu_deviceptr(&self) -> u64 { self.0.cu_deviceptr() }
    fn len(&self) -> usize { self.0.len() }
}
unsafe impl<W: Word> cuda_host::KernelSliceArgMut for ScalarOutput<W> {}

/// Direct stable scalar sort with a pooled output that can be adopted whole.
fn stable_keys<W: Word>(
    pool: &Arc<Pool>,
    launch: &rccl_cub::cuda::Launch<'_, '_>,
    input: &Buffer<W>,
    rows: usize,
    bits: u32,
) -> Result<Buffer<W>> {
    if W::BITS != 64 || bits > 64 {
        return Err("scalar radix sort requires one physical lane".into());
    }
    let mut first = ScalarOutput(unsafe { Buffer::<W>::uninitialized(pool.clone(), rows)? });
    let mut alternate = ScalarOutput(unsafe { Buffer::<W>::uninitialized(pool.clone(), rows)? });
    unsafe {
        cuda_core::simt::memory::memcpy_dtod_async(
            first.0.cu_deviceptr(), input.cu_deviceptr(), rows * 8,
            pool.stream().cu_stream(),
        )?;
    }
    let mut controls = RadixScratch {
        workspace: rccl_cub::device_radix_sort_keys_buffer_workspace::<u64>(launch, rows)?,
        pool: pool.clone(),
    };
    let first_is_current = {
        let mut buffers = rccl_cub::DoubleBuffer::new(&mut first, &mut alternate)
            .ok_or("unequal scalar radix buffers")?;
        rccl_cub::device_radix_sort_keys_buffer(
            launch, &mut buffers, 0..bits, &mut controls.workspace,
        )?;
        buffers.current_is_first()
    };
    Ok(if first_is_current { first.0 } else { alternate.0 })
}

/// Stable merge-path co-rank: how many `first` rows precede output `diagonal`
/// when equal words take the first run before the second.
/// # Safety
/// Both pointers must address initialized sorted words of the given lengths.
#[inline(always)]
pub(crate) unsafe fn corank<W: Word>(
    first: *const W,
    first_len: usize,
    second: *const W,
    second_len: usize,
    diagonal: usize,
) -> usize {
    let mut lo = diagonal.saturating_sub(second_len);
    let mut hi = diagonal.min(first_len);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let other = diagonal - mid;
        if other > 0
            && mid < first_len
            && !unsafe { &*first.add(mid) }
                .compare(unsafe { &*second.add(other - 1) })
                .is_gt()
        {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo
}

#[cuda_host::cuda_module]
pub(crate) mod run_kernels {
    use crate::{runs::RunView, Word};
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};

    /// Each thread finds one merge-path diagonal, then merges a bounded tile.
    /// Tiles partition the output, so rows need neither atomics nor scratch.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn merge_wide<W: Word>(
        first: RunView<W>,
        second: RunView<W>,
        output: *mut W,
        merge_tile_rows: usize,
    ) {
        let rows = first.len + second.len;
        let mut tile = thread::index_1d().get() as usize;
        let stride = thread::gridDim_x() as usize * 256;
        let tiles = rows.div_ceil(merge_tile_rows);
        while tile < tiles {
            let start = tile * merge_tile_rows;
            let end = start + (rows - start).min(merge_tile_rows);
            // Find how many first-run rows precede this output diagonal.
            // Equal keys prefer the first run, matching the merge below.
            let lo =
                unsafe { crate::runs::corank(first.ptr, first.len, second.ptr, second.len, start) };
            let mut a = lo;
            let mut b = start - lo;
            let mut row = start;
            while row < end {
                let take_first = a < first.len
                    && (b == second.len
                        || !unsafe { &*first.ptr.add(a) }
                            .compare(unsafe { &*second.ptr.add(b) })
                            .is_gt());
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
    pub fn gather_sorted<W: Word>(input: *const W, rows: u32, order: *const u32, output: *mut W) {
        let mut row = thread::index_1d().get() as usize;
        let stride = thread::gridDim_x() as usize * 256;
        while row < rows as usize {
            unsafe {
                *output.add(row) = *input.add(*order.add(row) as usize);
            }
            row += stride;
        }
    }

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn gather_lane(
        input: *const u64,
        rows: u32,
        lane: u32,
        lanes: u32,
        ms_first: bool,
        values: *mut u64,
        order: *mut u32,
    ) {
        let mut row = thread::index_1d().get() as usize;
        let stride = thread::gridDim_x() as usize * 256;
        let physical = if ms_first { lanes - 1 - lane } else { lane };
        while row < rows as usize {
            let source = if lane == 0 {
                row as u32
            } else {
                unsafe { *order.add(row) }
            };
            unsafe {
                *order.add(row) = source;
                *values.add(row) = *input.add(source as usize * lanes as usize + physical as usize);
            }
            row += stride;
        }
    }
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn absent<W: Word>(
        input: *const W,
        rows: u64,
        runs: *const RunView<W>,
        count: u32,
        flags: *mut u32,
    ) {
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
                    if unsafe { &*view.ptr.add(mid) }.compare(&value).is_lt() {
                        begin = mid + 1;
                    } else {
                        end = mid;
                    }
                }
                absent =
                    begin == view.len || !unsafe { &*view.ptr.add(begin) }.compare(&value).is_eq();
                run += 1;
            }
            unsafe {
                *flags.add(row) = absent as u32;
            }
            row += stride;
        }
    }
}
