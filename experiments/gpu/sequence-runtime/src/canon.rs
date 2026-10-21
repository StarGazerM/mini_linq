//! Exact canonicalization of explicitly packed u64 keys using RCCL radix sort
//! followed by adjacent unique. No query layout or scheduling decision lives
//! here: callers provide the live input length and the complete key bit width.

use crate::Result;
use cq_gpu_runtime::{Bounds, Buffer, DeviceColumns, Dictionary, EncodedColumn, Pool, RowCodec};
use cuda_core::{CudaContext, DeviceBuffer, DeviceCopy, LaunchConfig1D};
use gpu_device_columns::{OutputCount, Table};
use rccl_cub::{DeviceRadixSortBufferWorkspace, DoubleBuffer};
use rccl_thrust::prelude::*;
use std::{cell::RefCell, rc::Rc, sync::Arc};

thread_local! {
    static CACHED_SCRATCH: RefCell<Option<CanonScratch>> = const { RefCell::new(None) };
    static CACHED_KERNELS: RefCell<Option<(Arc<CudaContext>, Rc<canon_kernels::LoadedModule>)>> =
        const { RefCell::new(None) };
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
    capacity: usize,
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
        if let Some(selected) = self.selected.take() {
            let _ = unsafe { selected.drop_async(self.pool.stream()) };
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
        if self.bits > 64 || self.shift > 64 || self.bits > 64 - self.shift {
            return Err("packed field exceeds the u64 key".into());
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
/// Sort/count scratch is cached per thread across providers sharing the same
/// pool, reusing a larger allocation when it fits. Individual results own their
/// allocations and survive reuse or destruction of this provider.
pub struct PackedCanon<'pool> {
    pool: &'pool Arc<Pool>,
    cub: rccl_cub::cuda::Program<'pool>,
    thrust: rccl_thrust::cuda::Program<'pool>,
    scratch: Option<CanonScratch>,
    capacity: usize,
}

impl<'pool> PackedCanon<'pool> {
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
    /// `input[0..rows]` must be initialized on the pool stream and remain
    /// immutable until the submitted work completes. Every key must fit in
    /// `key_bits` low bits (zero bits means all keys are zero). The packing must
    /// encode the entire equality key without collisions. Unused capacity is
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
        if rows > input.len() || rows > self.capacity {
            return Err("packed canon live length exceeds input or provider capacity".into());
        }
        if key_bits > 64 {
            return Err("packed canon key width exceeds 64 bits".into());
        }
        let keys = unsafe { Buffer::<u64>::uninitialized(self.pool.clone(), rows)? };
        let metadata = Arc::new(Buffer::<OutputCount>::zeroed(self.pool.clone(), 1)?);
        if rows != 0 {
            let scratch = self.scratch.as_mut().expect("live packed canon scratch");
            let selected = scratch
                .selected
                .as_mut()
                .expect("packed canon count scratch");
            let stream = self.pool.stream();
            let mut first = unsafe { DeviceBuffer::<u64>::uninitialized_async(stream, rows)? };
            let mut alternate = unsafe { DeviceBuffer::<u64>::uninitialized_async(stream, rows)? };
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
            let mut buffers = DoubleBuffer::new(&mut first, &mut alternate)
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
                first.drop_async(stream)?;
                alternate.drop_async(stream)?;
            }
        }
        Ok(CanonicalKeys { keys, metadata })
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

/// Owning, ascending distinct packed keys. Only `[0, *count_ptr())` is live;
/// capacity is an allocation bound, never the logical cardinality.
pub struct CanonicalKeys {
    keys: Buffer<u64>,
    metadata: Arc<Buffer<OutputCount>>,
}

impl CanonicalKeys {
    /// Borrow storage for count-gated generated kernels. The tail is uninitialized.
    pub fn keys(&self) -> &Buffer<u64> {
        &self.keys
    }

    pub fn capacity(&self) -> usize {
        self.keys.len()
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
    /// logical count is downloaded. Descriptor upload follows DeviceColumns'
    /// existing synchronous `Buffer::from_host` convention.
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
        let capacity = self.capacity();
        unpack_columns(
            self.keys,
            self.metadata,
            capacity,
            fields,
            dictionary,
            statuses,
        )
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
    if rows > keys.len() {
        return Err("packed output live length exceeds key capacity".into());
    }
    let metadata = Arc::new(Buffer::from_host(
        keys.pool().clone(),
        &[OutputCount {
            rows: rows as u64,
            error: 0,
            padding: 0,
        }],
    )?);
    unpack_columns(keys, metadata, rows, fields, dictionary, statuses)
}

/// Both output paths use the same count-gated unpacking and ownership rules.
fn unpack_columns<'data, Row: RowCodec<'data>>(
    keys: Buffer<u64>,
    metadata: Arc<Buffer<OutputCount>>,
    capacity: usize,
    fields: &[PackedField],
    dictionary: Arc<Dictionary<'data>>,
    statuses: Vec<Arc<Buffer<OutputCount>>>,
) -> Result<DeviceColumns<'data, Row>> {
    if fields.len() != Row::WIDTH {
        return Err("packed field count differs from output row width".into());
    }
    let pool = keys.pool().clone();
    if statuses
        .iter()
        .any(|status| !Arc::ptr_eq(status.pool(), &pool))
    {
        return Err("packed output ancestor statuses must share the output pool".into());
    }
    let bounds = fields
        .iter()
        .map(|field| field.bounds())
        .collect::<Result<Vec<_>>>()?;
    let mut columns = Vec::with_capacity(fields.len());
    let module = if fields.is_empty() || capacity == 0 {
        None
    } else {
        Some(module(&pool)?)
    };
    for (field, &bound) in fields.iter().zip(&bounds) {
        let column = unsafe { Buffer::<i64>::uninitialized(pool.clone(), capacity)? };
        if let Some(module) = &module {
            let launch = LaunchConfig1D::new(capacity.div_ceil(256).clamp(1, 2272) as u32, 256, 0);
            module.unpack_field(
                pool.stream(),
                &module.prepare_unpack_field(launch)?,
                keys.ptr(),
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
    let host_views: Vec<_> = columns
        .iter()
        .map(|column| unsafe { column.view() })
        .collect();
    let views = Arc::new(Buffer::from_host(pool.clone(), &host_views)?);
    let table = Table {
        columns,
        views,
        metadata,
        capacity,
        bounds,
        statuses,
    };
    // Every raw view/count pointer is retained by the table, all columns
    // have the same gated capacity, and decoding is ordered before use.
    Ok(unsafe { DeviceColumns::from_parts(table, dictionary, pool) })
}

#[cuda_device::cuda_module]
mod canon_kernels {
    use cuda_device::{kernel, launch_bounds, launch_contract, thread};

    /// Unpack only initialized packed rows, leaving spare capacity unread.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    pub fn unpack_field(
        keys: *const u64,
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
                (unsafe { *keys.add(row as usize) } >> shift) & mask
            };
            // Host validation proves base + offset fits i64. Wrapping also
            // handles a full-width unsigned offset above i64::MAX correctly.
            unsafe { *output.add(row as usize) = base.wrapping_add(offset as i64) };
            row += stride;
        }
    }
}
