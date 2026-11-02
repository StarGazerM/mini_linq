//! Device-column adapters for runtime-sized, low-lane-first packed rows.

use crate::{Auxiliary, Geometry, Result};
use cq_gpu_runtime::{Bounds, Buffer, DeviceColumns, Dictionary, EncodedColumn, Pool, RowCodec};
use cuda_core::{CudaContext, DeviceCopy};
use gpu_datalog_runtime::{
    radix_run::{EncodedRow, PackedRow},
    RadixLayout,
};
use gpu_device_columns::{ColumnView, OutputCount, Table};
use std::{cell::RefCell, rc::Rc, sync::Arc};

thread_local! {
    static CACHED_KERNELS: RefCell<Option<(Arc<CudaContext>, Rc<canon_kernels::LoadedModule>)>> =
        const { RefCell::new(None) };
}

pub(crate) fn release_query_cache() {
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

/// One recorded field in a packed key. Its decoded value is
/// `base + ((key >> shift) & ((1 << bits) - 1))`.
/// Zero-bit fields represent constants. Packed values are encodings of i64
/// values, so a field's decoded domain is `[base, min(base + 2^bits - 1,
/// i64::MAX)]`: a span rounded up to `bits` never claims values above i64.
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
        let maximum = (self.base as i128 + ((1_i128 << self.bits) - 1)).min(i64::MAX as i128);
        let max = maximum as i64;
        Ok(Bounds::Inclusive {
            min: self.base,
            max,
        })
    }
}

/// Adapt a live prefix of packed rows without sorting or removing duplicates.
/// Row order and multiplicity are preserved; this function makes no canonical
/// or distinctness claim. The initialized prefix must be ready on the keys'
/// pool stream. Dictionary-valued fields must use the supplied dictionary's
/// encoding, and statuses must retain all consumed ancestor failures.
// DECODE V: packed_rows wrapper eagerly expands into device columns before consumers; P6 retain packed resident fields.
pub fn packed_columns<'data, Row: RowCodec<'data>>(
    keys: Buffer<u64>,
    rows: usize,
    fields: &[PackedField],
    dictionary: Arc<Dictionary<'data>>,
    statuses: Vec<Arc<Buffer<OutputCount>>>,
    geometry: Geometry,
) -> Result<DeviceColumns<'data, Row>> {
    PackedColumnsAdapter::new(keys.pool().clone(), fields.len(), geometry)?
        // DECODE V: packed_rows wrapper dispatches device expansion before publication; P6 defer logical inversion.
        .packed_columns(keys, rows, fields, dictionary, statuses)
}

fn field_lanes(fields: &[PackedField]) -> Result<usize> {
    let mut bits = 0;
    for field in fields {
        field.bounds()?;
        bits = bits.max(field.shift + field.bits);
    }
    RadixLayout::<EncodedRow, (), PackedRow, u64, ()>::key_lanes(bits)
}

/// Setup-time unpacking resources for one output width and runtime pool.
///
/// Construction warms the canon module and reserves count/view buffers.
/// Uploads reuse those buffers only while their Arcs are unique; published
/// outputs retain immutable storage even across later calls or adapter drop.
/// Generated programs can retain this adapter outside their timed dispatch.
/// Unpacking launches with the recorded `canon` family geometry.
pub struct PackedColumnsAdapter {
    pool: Arc<Pool>,
    module: Rc<canon_kernels::LoadedModule>,
    metadata: Arc<Buffer<OutputCount>>,
    views: Arc<Buffer<ColumnView>>,
    host_views: Vec<ColumnView>,
    geometry: Geometry,
}

impl PackedColumnsAdapter {
    pub fn new(pool: Arc<Pool>, width: usize, geometry: Geometry) -> Result<Self> {
        geometry.check("canon", Auxiliary::SUPPORTED.canon)?;
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
            geometry,
        })
    }

    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }

    /// Adapt a live prefix without changing its order or multiplicity.
    /// Dictionary-valued fields use the supplied dictionary's encoding;
    /// statuses retain all consumed ancestor failures.
    // DECODE V: packed adapter expands the live prefix before possible chained relations; P6 keep its encoded view.
    pub fn packed_columns<'data, Row: RowCodec<'data>>(
        &mut self,
        keys: Buffer<u64>,
        rows: usize,
        fields: &[PackedField],
        dictionary: Arc<Dictionary<'data>>,
        statuses: Vec<Arc<Buffer<OutputCount>>>,
    ) -> Result<DeviceColumns<'data, Row>> {
        let lanes = field_lanes(fields)?;
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
        // DECODE V: packed_columns expands every field into device buffers; P6 defer inversion until logical reads.
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
    // DECODE V: canonical output adapter expands packed values before possible chained relations; P6 retain encoded fields.
    pub fn unpack_columns<'data, Row: RowCodec<'data>>(
        &mut self,
        keys: RadixLayout<EncodedRow, (), PackedRow, u64, ()>,
        fields: &[PackedField],
        dictionary: Arc<Dictionary<'data>>,
        statuses: Vec<Arc<Buffer<OutputCount>>>,
    ) -> Result<DeviceColumns<'data, Row>> {
        let capacity = keys.capacity();
        // DECODE V: canonical unpack delegates eager device expansion; P6 publish a packed resident view.
        self.decode_columns(
            keys.keys(),
            keys.lanes(),
            keys.metadata().clone(),
            capacity,
            fields,
            dictionary,
            statuses,
        )
    }

    // DECODE V: output adapter materializes full i64 columns before their consumers; P6 keep packed values resident.
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
        if fields
            .iter()
            .any(|field| (field.shift + field.bits) as usize > lanes * 64)
        {
            return Err("packed field exceeds the stored key width".into());
        }
        let mut columns = Vec::with_capacity(fields.len());
        let prepared = if capacity != 0 && !fields.is_empty() {
            let shape = self.module.prepare_unpack_field(self.geometry.config(1))?;
            let grid = self.geometry.grid(shape.function(), capacity)?;
            Some(
                self.module
                    .prepare_unpack_field(self.geometry.config(grid))?,
            )
        } else {
            None
        };
        for (field, &bound) in fields.iter().zip(&bounds) {
            let column = unsafe { Buffer::<i64>::uninitialized(self.pool.clone(), capacity)? };
            if let Some(prepared) = &prepared {
                // DECODE V: per-field unpack launch expands all live rows before chaining; P6 decode only at the allowed consumer.
                self.module.unpack_field(
                    self.pool.stream(),
                    prepared,
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

    /// Unpack only initialized packed rows, leaving spare capacity unread.
    #[kernel]
    #[launch_bounds(256)]
    #[launch_contract(domain = 1, block = (256, 1, 1))]
    // DECODE V: unpack_field constructs device logical columns before possible relational consumers; P6 preserve packed fields.
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
            // DECODE V: packed FOR base-plus-offset inversion precedes host publication and may feed chaining; P6 delay reconstruction.
            unsafe { *output.add(row as usize) = base.wrapping_add(offset as i64) };
            row += stride;
        }
    }
}
