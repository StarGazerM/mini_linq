//! Exact canonicalization of packed keys. `PackedCanon` handles up to two
//! lanes; `WideCanon` stably radix-sorts arbitrary-width rows lane by lane.
//! Recorded codec bounds determine the lane count before dispatch.

use crate::Result;
use cq_gpu_runtime::{Bounds, Buffer, DeviceColumns, Dictionary, EncodedColumn, Pool, RowCodec};
use cuda_core::{CudaContext, DeviceBuffer, DeviceCopy, LaunchConfig1D};
use gpu_device_columns::{ColumnView, OutputCount, Table};
use rccl_cub::{DeviceRadixSortBufferWorkspace, DoubleBuffer};
use rccl_thrust::prelude::*;
use std::{cell::RefCell, rc::Rc, sync::Arc};

thread_local! {
    static CACHED_SCRATCH: RefCell<Option<CanonScratch>> = const { RefCell::new(None) };
    static CACHED_KERNELS: RefCell<Option<(Arc<CudaContext>, Rc<canon_kernels::LoadedModule>)>> =
        const { RefCell::new(None) };
}

pub(crate) fn release_query_cache() {
    CACHED_SCRATCH.with(|cache| cache.borrow_mut().take());
    CACHED_KERNELS.with(|cache| cache.borrow_mut().take());
}

fn module(pool: &Pool) -> Result<Rc<canon_kernels::LoadedModule>> {
    CACHED_KERNELS.with(|cache| {
        if let Some((context, module)) = cache.borrow().as_ref() {
            if Arc::ptr_eq(context, pool.context()) {
                return Ok(module.clone());
            }
        }
        let module = Rc::new(unsafe { canon_kernels::load(pool.context())? });
        *cache.borrow_mut() = Some((pool.context().clone(), module.clone()));
        Ok(module)
    })
}

// Only owned resources cross invocations. Programs still borrow the caller's
// pool, and each checked-out scratch set has exclusive access on its stream.
struct CanonScratch {
    pool: Arc<Pool>,
    sort: DeviceRadixSortBufferWorkspace,
    selected: Option<DeviceBuffer<u64>>,
    first: Option<DeviceBuffer<u64>>,
    alternate: Option<DeviceBuffer<u64>>,
    wide: Option<DeviceBuffer<u128>>,
    pair_sort: Option<DeviceRadixSortBufferWorkspace>,
    indices: Option<DeviceBuffer<u32>>,
    alternate_indices: Option<DeviceBuffer<u32>>,
    capacity: usize,
}

impl CanonScratch {
    fn prepare_keys(&mut self, rows: usize) -> Result<()> {
        // DoubleBuffer uses each allocation's full length as the live sort
        // domain. Keep exact extents rather than sorting spare capacity.
        for buffer in [&mut self.first, &mut self.alternate] {
            if buffer.as_ref().is_none_or(|buffer| buffer.len() != rows) {
                let replacement =
                    unsafe { DeviceBuffer::uninitialized_async(self.pool.stream(), rows)? };
                if let Some(previous) = buffer.replace(replacement) {
                    unsafe { previous.drop_async(self.pool.stream())? };
                }
            }
        }
        Ok(())
    }
}

impl Drop for CanonScratch {
    fn drop(&mut self) {
        // RCCL exposes mutable control buffers but not consuming decomposition.
        // Empty replacements allocate nothing; all real frees follow their
        // last use on the retained pool's stream, even after a context change.
        fn release<T: DeviceCopy>(pool: &Pool, buffer: &mut DeviceBuffer<T>) {
            let empty = unsafe { DeviceBuffer::uninitialized_async(pool.stream(), 0) }
                .expect("zero-length device buffer needs no allocation");
            let buffer = std::mem::replace(buffer, empty);
            let _ = unsafe { buffer.drop_async(pool.stream()) };
        }
        let (bins, lookback, counters) = self.sort.__parts_mut();
        release(&self.pool, bins);
        release(&self.pool, lookback);
        release(&self.pool, counters);
        if let Some(sort) = self.pair_sort.as_mut() {
            let (bins, lookback, counters) = sort.__parts_mut();
            release(&self.pool, bins);
            release(&self.pool, lookback);
            release(&self.pool, counters);
        }
        for buffer in [&mut self.indices, &mut self.alternate_indices] {
            if let Some(buffer) = buffer.take() {
                let _ = unsafe { buffer.drop_async(self.pool.stream()) };
            }
        }
        if let Some(selected) = self.selected.take() {
            let _ = unsafe { selected.drop_async(self.pool.stream()) };
        }
        for buffer in [&mut self.first, &mut self.alternate] {
            if let Some(buffer) = buffer.take() {
                let _ = unsafe { buffer.drop_async(self.pool.stream()) };
            }
        }
        if let Some(buffer) = self.wide.take() {
            let _ = unsafe { buffer.drop_async(self.pool.stream()) };
        }
    }
}

/// One recorded field in a packed key. Its decoded value is
/// `base + ((key >> shift) & ((1 << bits) - 1))`.
/// Zero-bit fields represent constants; a full-width field is also supported
/// when its complete unsigned offset domain fits after adding `base`.
#[derive(Clone, Copy, Debug)]
pub struct PackedField {
    pub shift: u32,
    pub bits: u32,
    pub base: i64,
}

impl PackedField {
    fn bounds(self) -> Result<Bounds> {
        if self.bits > 64 || self.shift.checked_add(self.bits).is_none() {
            return Err("packed field width overflows".into());
        }
        let maximum = self.base as i128 + ((1_i128 << self.bits) - 1);
        let max =
            i64::try_from(maximum).map_err(|_| "packed field's decoded domain exceeds i64")?;
        Ok(Bounds::Inclusive {
            min: self.base,
            max,
        })
    }
}

/// Reusable RCCL programs, sort controls and count scratch on one runtime pool.
/// The fixed capacity is supplied by generated host code, not chosen here.
/// Sort/count and exact-live-size key scratch are cached per thread across
/// providers sharing the same pool. Sort controls reuse a larger allocation
/// when it fits; results own their allocations and survive provider reuse.
pub struct PackedCanon<'pool> {
    pool: &'pool Arc<Pool>,
    cub: rccl_cub::cuda::Program<'pool>,
    thrust: rccl_thrust::cuda::Program<'pool>,
    scratch: Option<CanonScratch>,
    capacity: usize,
}

impl<'pool> PackedCanon<'pool> {
    pub const MAX_PACKED_BITS: u32 = 128;

    /// Select the physical width during setup, before allocating or packing keys.
    pub fn key_lanes(key_bits: u32) -> Result<usize> {
        if key_bits > Self::MAX_PACKED_BITS {
            return Err("packed canon key exceeds its 128-bit capability".into());
        }
        Ok(if key_bits > 64 { 2 } else { 1 })
    }

    pub fn field_lanes(fields: &[PackedField]) -> Result<usize> {
        let mut bits = 0;
        for field in fields {
            field.bounds()?;
            bits = bits.max(field.shift + field.bits);
        }
        Self::key_lanes(bits)
    }
    pub fn new(pool: &'pool Arc<Pool>, capacity: usize) -> Result<Self> {
        if capacity > u32::MAX as usize {
            return Err("packed canon capacity exceeds u32::MAX".into());
        }
        let scratch = CACHED_SCRATCH.with(|cache| cache.borrow_mut().take());
        let scratch = scratch
            .filter(|scratch| Arc::ptr_eq(&scratch.pool, pool) && scratch.capacity >= capacity);
        // Disposing a cache from another context binds that context for its
        // asynchronous frees. Restore the requested context before allocation.
        pool.context().bind_to_thread()?;
        let cub = rccl_cub::cuda::Program::load(pool.context())?;
        let thrust = rccl_thrust::cuda::Program::load(pool.context())?;
        let scratch = match scratch {
            Some(scratch) => scratch,
            None => {
                let sort = rccl_cub::device_radix_sort_keys_buffer_workspace::<u64>(
                    &cub.on(pool.stream())?,
                    capacity,
                )?;
                let mut scratch = CanonScratch {
                    pool: pool.clone(),
                    sort,
                    selected: None,
                    first: None,
                    alternate: None,
                    wide: None,
                    pair_sort: None,
                    indices: None,
                    alternate_indices: None,
                    capacity,
                };
                scratch.selected =
                    Some(unsafe { DeviceBuffer::uninitialized_async(pool.stream(), 1)? });
                scratch
            }
        };
        Ok(Self {
            pool,
            cub,
            thrust,
            scratch: Some(scratch),
            capacity,
        })
    }

    pub fn pool(&self) -> &Arc<Pool> {
        self.pool
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Sort the live prefix and retain each distinct complete packed key once.
    /// The input is not modified. The returned count stays on the device;
    /// `CanonicalKeys::logical_count` is an explicit optional host readback.
    /// RCCL requires native scratch buffers; result storage and metadata use
    /// the runtime pool, and all work is ordered on that pool's stream.
    ///
    /// # Safety
    /// `input[0..rows * key_lanes(key_bits)]` must be initialized on the pool
    /// stream and remain immutable until the submitted work completes. Each
    /// row stores its low lane first, followed by its high lane for wide keys.
    /// Every key must fit in `key_bits` low bits (zero bits means all keys are zero).
    /// Packing must encode the entire equality key without collisions. Unused capacity is
    /// never read. Input and provider must share the identical pool.
    pub unsafe fn canonicalize(
        &mut self,
        input: &Buffer<u64>,
        rows: usize,
        key_bits: u32,
    ) -> Result<CanonicalKeys> {
        if !Arc::ptr_eq(&self.pool, input.pool()) {
            return Err("packed canon input must share the provider pool and stream".into());
        }
        let lanes = Self::key_lanes(key_bits)?;
        if rows > input.len() / lanes || rows > self.capacity {
            return Err("packed canon live length exceeds input or provider capacity".into());
        }
        let keys = unsafe { Buffer::<u64>::uninitialized(self.pool.clone(), rows * lanes)? };
        let metadata = Arc::new(Buffer::<OutputCount>::zeroed(self.pool.clone(), 1)?);
        if rows != 0 && lanes == 2 {
            let scratch = self.scratch.as_mut().expect("live packed canon scratch");
            if scratch.wide.as_ref().is_none_or(|buffer| buffer.len() != rows) {
                let replacement = unsafe { DeviceBuffer::<u128>::uninitialized_async(self.pool.stream(), rows)? };
                if let Some(previous) = scratch.wide.replace(replacement) {
                    unsafe { previous.drop_async(self.pool.stream())? };
                }
            }
            let wide = scratch.wide.as_mut().expect("wide canon scratch");
            let selected = scratch.selected.as_mut().expect("packed canon count scratch");
            unsafe {
                cuda_core::memory::memcpy_dtod_async(wide.cu_deviceptr(), input.cu_deviceptr(),
                    rows * std::mem::size_of::<u128>(), self.pool.stream().cu_stream())?;
            }
            let execution = self.thrust.par_in(self.pool.stream(), self.pool.resource())?;
            wide.sort_unstable_by(&execution, |a: &u128, b: &u128| a.cmp(b))?;
            let output = keys.ptr();
            rccl_thrust::unique_copy_by(
                &execution,
                wide.view(),
                rccl_thrust::indexed_output(rows, move |index, value: u128| unsafe {
                    output.add(index * 2).write(value as u64);
                    output.add(index * 2 + 1).write((value >> 64) as u64);
                }),
                selected,
                |a: u128, b: u128| a == b,
            )?;
            unsafe {
                cuda_core::memory::memcpy_dtod_async(metadata.cu_deviceptr(), selected.cu_deviceptr(),
                    std::mem::size_of::<u64>(), self.pool.stream().cu_stream())?;
            }
        } else if rows != 0 {
            let scratch = self.scratch.as_mut().expect("live packed canon scratch");
            scratch.prepare_keys(rows)?;
            let selected = scratch
                .selected
                .as_mut()
                .expect("packed canon count scratch");
            let stream = self.pool.stream();
            let first = scratch.first.as_mut().expect("packed canon first scratch");
            let alternate = scratch
                .alternate
                .as_mut()
                .expect("packed canon alternate scratch");
            unsafe {
                cuda_core::memory::memcpy_dtod_async(
                    first.cu_deviceptr(),
                    input.cu_deviceptr(),
                    rows * std::mem::size_of::<u64>(),
                    stream.cu_stream(),
                )?;
            }
            let launch = self.cub.on(stream)?;
            let execution = self.thrust.par_in(stream, self.pool.resource())?;
            let mut buffers = DoubleBuffer::new(first, alternate)
                .ok_or("packed canon radix buffer extents differ")?;
            if key_bits != 0 {
                rccl_cub::device_radix_sort_keys_buffer(
                    &launch,
                    &mut buffers,
                    0..key_bits,
                    &mut scratch.sort,
                )?;
            }
            let output = keys.ptr();
            rccl_thrust::unique_copy_by(
                &execution,
                buffers.current().view(),
                rccl_thrust::indexed_output(rows, move |index, value: u64| unsafe {
                    output.add(index).write(value);
                }),
                selected,
                |a: u64, b: u64| a == b,
            )?;
            unsafe {
                // OutputCount starts with the logical u64 row count. The
                // zeroed error/padding words remain untouched.
                cuda_core::memory::memcpy_dtod_async(
                    metadata.cu_deviceptr(),
                    selected.cu_deviceptr(),
                    std::mem::size_of::<u64>(),
                    stream.cu_stream(),
                )?;
            }
        }
        Ok(CanonicalKeys {
            keys: Arc::new(keys),
            metadata,
            lanes,
        })
    }
}

impl Drop for PackedCanon<'_> {
    fn drop(&mut self) {
        let mut scratch = self.scratch.take();
        // try_with also handles a provider dropped during TLS destruction.
        // Leave any displaced resource outside the RefCell borrow for cleanup.
        let _ = CACHED_SCRATCH.try_with(|cache| {
            let mut cached = cache.borrow_mut();
            let keep_cached = cached.as_ref().is_some_and(|cached| {
                scratch.as_ref().is_some_and(|scratch| {
                    Arc::ptr_eq(&cached.pool, &scratch.pool) && cached.capacity >= scratch.capacity
                })
            });
            if !keep_cached {
                std::mem::swap(&mut *cached, &mut scratch);
            }
        });
        if let Some(scratch) = scratch {
            drop(scratch);
            // A displaced cache may have belonged to a different context.
            let _ = self.pool.context().bind_to_thread();
        }
    }
}

/// Arbitrary-width instance of the canon law, using stable least-significant
/// lane radix passes over row indices and full-row adjacent equality.
pub struct WideCanon<'pool> {
    inner: PackedCanon<'pool>,
}

impl<'pool> WideCanon<'pool> {
    pub fn key_lanes(key_bits: u32) -> Result<usize> {
        Ok((key_bits as usize).div_ceil(64).max(1))
    }

    pub fn field_lanes(fields: &[PackedField]) -> Result<usize> {
        let mut bits = 0;
        for field in fields {
            field.bounds()?;
            bits = bits.max(field.shift + field.bits);
        }
        Self::key_lanes(bits)
    }

    pub fn new(pool: &'pool Arc<Pool>, capacity: usize) -> Result<Self> {
        Ok(Self {
            inner: PackedCanon::new(pool, capacity)?,
        })
    }

    /// Sort complete rows into ascending packed-key order, retaining one copy.
    ///
    /// # Safety
    /// The same initialized-prefix, encoding and stream requirements as
    /// `PackedCanon::canonicalize` apply, with `WideCanon::key_lanes`.
    // Formal rule GPU-WideCanon: Stable radix passes preserve complete packed-row equality, an instance of BE-Canon.
    pub unsafe fn canonicalize(
        &mut self,
        input: &Buffer<u64>,
        rows: usize,
        key_bits: u32,
    ) -> Result<CanonicalKeys> {
        let pool = self.inner.pool;
        let lanes = Self::key_lanes(key_bits)?;
        if !Arc::ptr_eq(pool, input.pool()) {
            return Err("wide canon input must share the provider pool and stream".into());
        }
        if rows > input.len() / lanes || rows > self.inner.capacity {
            return Err("wide canon live length exceeds input or provider capacity".into());
        }
        let keys = Arc::new(unsafe { Buffer::<u64>::uninitialized(pool.clone(), rows * lanes)? });
        let metadata = Arc::new(Buffer::<OutputCount>::zeroed(pool.clone(), 1)?);
        if rows != 0 {
            let scratch = self.inner.scratch.as_mut().expect("live canon scratch");
            scratch.prepare_keys(rows)?;
            for buffer in [&mut scratch.indices, &mut scratch.alternate_indices] {
                if buffer.as_ref().is_none_or(|buffer| buffer.len() != rows) {
                    let replacement =
                        unsafe { DeviceBuffer::uninitialized_async(pool.stream(), rows)? };
                    if let Some(previous) = buffer.replace(replacement) {
                        unsafe { previous.drop_async(pool.stream())? };
                    }
                }
            }
            let launch = self.inner.cub.on(pool.stream())?;
            if scratch.pair_sort.is_none() {
                scratch.pair_sort = Some(rccl_cub::device_radix_sort_pairs_buffer_workspace::<
                    u64,
                    u32,
                >(&launch, scratch.capacity)?);
            }
            let workspace = scratch.pair_sort.as_mut().expect("wide radix controls");
            let first = scratch.first.as_mut().expect("first lane scratch");
            let second = scratch.alternate.as_mut().expect("alternate lane scratch");
            let indices = scratch.indices.as_mut().expect("first row scratch");
            let alternate = scratch
                .alternate_indices
                .as_mut()
                .expect("alternate row scratch");
            let mut buffers = DoubleBuffer::paired((first, indices), (second, alternate))
                .ok_or("wide canon radix buffer extents differ")?;
            let module = module(pool)?;
            let config = LaunchConfig1D::new(rows.div_ceil(256).clamp(1, 2272) as u32, 256, 0);
            let prepared = module.prepare_gather_lane(config)?;
            for lane in 0..lanes {
                let (values, order) = buffers.current_mut();
                module.gather_lane(
                    pool.stream(),
                    &prepared,
                    input.ptr(),
                    values.cu_deviceptr() as *mut u64,
                    order.cu_deviceptr() as *mut u32,
                    rows as u32,
                    lanes as u32,
                    lane as u32,
                )?;
                rccl_cub::device_radix_sort_pairs_buffer(&launch, &mut buffers, 0..64, workspace)?;
            }
            let execution = self.inner.thrust.par_in(pool.stream(), pool.resource())?;
            let selected = scratch.selected.as_mut().expect("canon count scratch");
            let source = input.ptr();
            let output = keys.ptr();
            rccl_thrust::unique_copy_by(
                &execution,
                buffers.current().1.view(),
                rccl_thrust::indexed_output(rows, move |index, row: u32| unsafe {
                    for lane in 0..lanes {
                        output
                            .add(index * lanes + lane)
                            .write(*source.add(row as usize * lanes + lane));
                    }
                }),
                selected,
                move |a: u32, b: u32| unsafe {
                    (0..lanes).all(|lane| {
                        *source.add(a as usize * lanes + lane)
                            == *source.add(b as usize * lanes + lane)
                    })
                },
            )?;
            unsafe {
                cuda_core::memory::memcpy_dtod_async(
                    metadata.cu_deviceptr(),
                    selected.cu_deviceptr(),
                    std::mem::size_of::<u64>(),
                    pool.stream().cu_stream(),
                )?;
            }
        }
        Ok(CanonicalKeys {
            keys,
            metadata,
            lanes,
        })
    }
}

/// Owning, ascending distinct packed keys in row-major, low-lane-first storage.
/// Only the first `*count_ptr()` rows are live; capacity is an allocation bound.
pub struct CanonicalKeys {
    keys: Arc<Buffer<u64>>,
    metadata: Arc<Buffer<OutputCount>>,
    lanes: usize,
}

impl CanonicalKeys {
    /// Borrow storage for count-gated generated kernels. The tail is uninitialized.
    pub fn keys(&self) -> &Arc<Buffer<u64>> {
        &self.keys
    }

    pub fn capacity(&self) -> usize {
        self.keys.len() / self.lanes
    }


    pub fn lanes(&self) -> usize {
        self.lanes
    }
    pub fn count_ptr(&self) -> *const u64 {
        self.metadata.ptr().cast()
    }

    pub fn metadata(&self) -> &Arc<Buffer<OutputCount>> {
        &self.metadata
    }

    /// Explicitly synchronize and download the logical count, never key data.
    pub fn logical_count(&self) -> Result<usize> {
        Ok(usize::try_from(self.metadata.to_host()?[0].rows)?)
    }

    /// Unpack the recorded fields into native i64 columns on the same stream.
    /// The output retains its own device count, dictionary and ancestor status
    /// owners, independently of the packed result and provider. No key data or
    /// logical count is downloaded. Metadata and descriptor uploads are
    /// stream ordered and do not synchronize the host.
    ///
    /// Field values must use the supplied dictionary's encoding for any
    /// dictionary-valued columns. `statuses` must include all consumed ancestor
    /// failures; they are preserved for DeviceColumns' normal host validation.
    pub fn into_columns<'data, Row: RowCodec<'data>>(
        self,
        fields: &[PackedField],
        dictionary: Arc<Dictionary<'data>>,
        statuses: Vec<Arc<Buffer<OutputCount>>>,
    ) -> Result<DeviceColumns<'data, Row>> {
        PackedColumnsAdapter::new(self.keys.pool().clone(), fields.len())?
            .unpack_columns(self, fields, dictionary, statuses)
    }
}

/// Adapt a live prefix of packed rows without sorting or removing duplicates.
/// Row order and multiplicity are preserved; this function makes no canonical
/// or distinctness claim. The initialized prefix must be ready on the keys'
/// pool stream. Field values and ancestor statuses follow the same contracts
/// as `CanonicalKeys::into_columns`.
pub fn packed_columns<'data, Row: RowCodec<'data>>(
    keys: Buffer<u64>,
    rows: usize,
    fields: &[PackedField],
    dictionary: Arc<Dictionary<'data>>,
    statuses: Vec<Arc<Buffer<OutputCount>>>,
) -> Result<DeviceColumns<'data, Row>> {
    PackedColumnsAdapter::new(keys.pool().clone(), fields.len())?
        .packed_columns(keys, rows, fields, dictionary, statuses)
}

/// Setup-time unpacking resources for one output width and runtime pool.
///
/// Construction warms the canon module and reserves count/view buffers.
/// Uploads reuse those buffers only while their Arcs are unique; published
/// outputs retain immutable storage even across later calls or adapter drop.
/// Generated programs can retain this adapter outside their timed dispatch.
pub struct PackedColumnsAdapter {
    pool: Arc<Pool>,
    module: Rc<canon_kernels::LoadedModule>,
    metadata: Arc<Buffer<OutputCount>>,
    views: Arc<Buffer<ColumnView>>,
    host_views: Vec<ColumnView>,
}

impl PackedColumnsAdapter {
    pub fn new(pool: Arc<Pool>, width: usize) -> Result<Self> {
        let module = module(&pool)?;
        // These private buffers are initialized by upload before publication.
        let metadata = Arc::new(unsafe { Buffer::uninitialized(pool.clone(), 1)? });
        let views = Arc::new(unsafe { Buffer::uninitialized(pool.clone(), width)? });
        Ok(Self {
            pool,
            module,
            metadata,
            views,
            host_views: Vec::with_capacity(width),
        })
    }

    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }

    /// Adapt a live prefix without changing its order or multiplicity.
    /// Fields and ancestor statuses follow `CanonicalKeys::into_columns`.
    pub fn packed_columns<'data, Row: RowCodec<'data>>(
        &mut self,
        keys: Buffer<u64>,
        rows: usize,
        fields: &[PackedField],
        dictionary: Arc<Dictionary<'data>>,
        statuses: Vec<Arc<Buffer<OutputCount>>>,
    ) -> Result<DeviceColumns<'data, Row>> {
        let lanes = WideCanon::field_lanes(fields)?;
        if rows > keys.len() / lanes {
            return Err("packed output live length exceeds key capacity".into());
        }
        if !Arc::ptr_eq(keys.pool(), &self.pool) {
            return Err("packed output keys must share the adapter pool and stream".into());
        }
        upload_unique(
            &mut self.metadata,
            &[OutputCount {
                rows: rows as u64,
                error: 0,
                padding: 0,
            }],
        )?;
        self.decode_columns(
            &keys,
            lanes,
            self.metadata.clone(),
            rows,
            fields,
            dictionary,
            statuses,
        )
    }

    /// Unpack canonical keys without downloading their device-resident count.
    /// The result owns its columns and retains the original count and statuses.
    pub fn unpack_columns<'data, Row: RowCodec<'data>>(
        &mut self,
        keys: CanonicalKeys,
        fields: &[PackedField],
        dictionary: Arc<Dictionary<'data>>,
        statuses: Vec<Arc<Buffer<OutputCount>>>,
    ) -> Result<DeviceColumns<'data, Row>> {
        let capacity = keys.capacity();
        self.decode_columns(
            &keys.keys,
            keys.lanes,
            keys.metadata,
            capacity,
            fields,
            dictionary,
            statuses,
        )
    }

    fn decode_columns<'data, Row: RowCodec<'data>>(
        &mut self,
        keys: &Buffer<u64>,
        lanes: usize,
        metadata: Arc<Buffer<OutputCount>>,
        capacity: usize,
        fields: &[PackedField],
        dictionary: Arc<Dictionary<'data>>,
        statuses: Vec<Arc<Buffer<OutputCount>>>,
    ) -> Result<DeviceColumns<'data, Row>> {
        if fields.len() != Row::WIDTH || fields.len() != self.views.len() {
            return Err("packed field count differs from output row or adapter width".into());
        }
        if !Arc::ptr_eq(keys.pool(), &self.pool) {
            return Err("packed output keys must share the adapter pool and stream".into());
        }
        if statuses
            .iter()
            .any(|status| !Arc::ptr_eq(status.pool(), &self.pool))
        {
            return Err("packed output ancestor statuses must share the output pool".into());
        }
        let bounds = fields
            .iter()
            .map(|field| field.bounds())
            .collect::<Result<Vec<_>>>()?;
        if fields.iter().any(|field| (field.shift + field.bits) as usize > lanes * 64) {
            return Err("packed field exceeds the stored key width".into());
        }
        let mut columns = Vec::with_capacity(fields.len());
        for (field, &bound) in fields.iter().zip(&bounds) {
            let column = unsafe { Buffer::<i64>::uninitialized(self.pool.clone(), capacity)? };
            if capacity != 0 {
                let launch =
                    LaunchConfig1D::new(capacity.div_ceil(256).clamp(1, 2272) as u32, 256, 0);
                self.module.unpack_field(
                    self.pool.stream(),
                    &self.module.prepare_unpack_field(launch)?,
                    keys.ptr(),
                    lanes as u32,
                    metadata.ptr().cast(),
                    capacity as u64,
                    field.shift,
                    field.bits,
                    field.base,
                    column.ptr(),
                )?;
            }
            columns.push(Arc::new(EncodedColumn::from_plain_buffer(
                column,
                metadata.clone(),
                0,
                bound,
            )?));
        }
        self.host_views.clear();
        self.host_views
            .extend(columns.iter().map(|column| unsafe { column.view() }));
        upload_unique(&mut self.views, &self.host_views)?;
        let table = Table {
            columns,
            views: self.views.clone(),
            metadata,
            capacity,
            bounds,
            statuses,
        };
        // Every raw view/count pointer is retained by the table, all columns
        // have the same gated capacity, and decoding is ordered before use.
        Ok(unsafe { DeviceColumns::from_parts(table, dictionary, self.pool.clone()) })
    }
}

fn upload_unique<T: DeviceCopy>(slot: &mut Arc<Buffer<T>>, values: &[T]) -> Result<()> {
    if let Some(buffer) = Arc::get_mut(slot) {
        // No published owner or weak reference can observe this overwrite.
        // Outstanding device reads precede the upload on the same pool stream.
        unsafe { buffer.write_async(values)? };
    } else {
        *slot = Arc::new(Buffer::from_host_async(slot.pool().clone(), values)?);
    }
    Ok(())
}

#[cuda_device::cuda_module]
mod canon_kernels {
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};

    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn gather_lane(
        input: *const u64,
        values: *mut u64,
        order: *mut u32,
        rows: u32,
        lanes: u32,
        lane: u32,
    ) {
        let mut index = thread::index_1d().get() as usize;
        let stride = thread::gridDim_x() as usize * 256;
        while index < rows as usize {
            unsafe {
                if lane == 0 {
                    *order.add(index) = index as u32;
                }
                *values.add(index) =
                    *input.add(*order.add(index) as usize * lanes as usize + lane as usize);
            }
            index += stride;
        }
    }

    /// Unpack only initialized packed rows, leaving spare capacity unread.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn unpack_field(
        keys: *const u64,
        lanes: u32,
        count: *const u64,
        capacity: u64,
        shift: u32,
        bits: u32,
        base: i64,
        output: *mut i64,
    ) {
        let live = unsafe { *count }.min(capacity);
        let mask = if bits == 64 {
            u64::MAX
        } else {
            (1_u64 << bits) - 1
        };
        let mut row = thread::index_1d().get() as u64;
        let stride = thread::gridDim_x() as u64 * 256;
        while row < live {
            let offset = if bits == 0 {
                0
            } else {
                let index = row as usize * lanes as usize + (shift / 64) as usize;
                let local = shift % 64;
                let mut value = unsafe { *keys.add(index) } >> local;
                if local + bits > 64 {
                    value |= unsafe { *keys.add(index + 1) } << (64 - local);
                }
                value & mask
            };
            // Host validation proves base + offset fits i64. Wrapping also
            // handles a full-width unsigned offset above i64::MAX correctly.
            unsafe { *output.add(row as usize) = base.wrapping_add(offset as i64) };
            row += stride;
        }
    }
}
