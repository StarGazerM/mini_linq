//! Device-column adapters for runtime-sized, low-lane-first packed rows.

use crate::{Auxiliary, Geometry, Result};
use cq_gpu_runtime::{Bounds, Buffer, DeviceColumns, Dictionary, EncodedColumn, Pool, RowCodec};
use cuda_core::DeviceCopy;
use gpu_datalog_runtime::{
    radix_run::{EncodedRow, PackedRow},
    RadixLayout,
};
use gpu_device_columns::{Codec, ColumnView, OutputCount, SourceView, Table};
use std::sync::Arc;

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
// DECODE V: packed_rows wrapper reconstructs fields during FOR re-encoding; P6 retain original packed fields.
pub fn packed_columns<'data, Row: RowCodec<'data>>(
    keys: Buffer<u64>,
    rows: usize,
    fields: &[PackedField],
    dictionary: Arc<Dictionary<'data>>,
    statuses: Vec<Arc<Buffer<OutputCount>>>,
    geometry: Geometry,
) -> Result<DeviceColumns<'data, Row>> {
    PackedColumnsAdapter::new(keys.pool().clone(), fields.len(), geometry)?
        // DECODE V: P6 packed_rows dispatches fused reconstruction/re-encoding before publication.
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

/// Setup-time publication resources for one output width and runtime pool.
///
/// Construction warms the shared column encoder and reserves count/view buffers.
/// Uploads reuse those buffers only while their Arcs are unique; published
/// outputs retain immutable storage even across later calls or adapter drop.
/// Packed fields are gathered directly into FOR storage, measuring exact
/// logical bounds during encoding without an unpacked intermediate.
pub struct PackedColumnsAdapter {
    pool: Arc<Pool>,
    metadata: Arc<Buffer<OutputCount>>,
    views: Arc<Buffer<ColumnView>>,
    host_views: Vec<ColumnView>,
}

impl PackedColumnsAdapter {
    pub fn new(pool: Arc<Pool>, width: usize, geometry: Geometry) -> Result<Self> {
        geometry.check("canon", Auxiliary::SUPPORTED.canon)?;
        gpu_device_columns::prepare_kernels(&pool)?;
        // These private buffers are initialized by upload before publication.
        let metadata = Arc::new(unsafe { Buffer::uninitialized(pool.clone(), 1)? });
        let views = Arc::new(unsafe { Buffer::uninitialized(pool.clone(), width)? });
        Ok(Self {
            pool,
            metadata,
            views,
            host_views: Vec::with_capacity(width),
        })
    }

    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }

    /// Adapt a live prefix without changing its order or multiplicity.
    /// Dictionary-valued fields use the supplied dictionary's encoding;
    /// statuses retain all consumed ancestor failures.
    // DECODE V: P6 packed adapter reconstructs during FOR re-encoding before possible chaining.
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
        // DECODE V: P6 packed fields are reconstructed only inside the fused FOR encoder.
        self.encode_columns(
            &keys,
            lanes,
            self.metadata.clone(),
            rows,
            fields,
            dictionary,
            statuses,
        )
    }

    /// Encode canonical keys without downloading their device-resident count.
    /// Only measured scalar extrema are read; original count/status owners remain.
    // DECODE V: P6 canonical output reconstructs packed fields during FOR re-encoding before possible chaining.
    pub fn unpack_columns<'data, Row: RowCodec<'data>>(
        &mut self,
        keys: RadixLayout<EncodedRow, (), PackedRow, u64, ()>,
        fields: &[PackedField],
        dictionary: Arc<Dictionary<'data>>,
        statuses: Vec<Arc<Buffer<OutputCount>>>,
    ) -> Result<DeviceColumns<'data, Row>> {
        let capacity = keys.capacity();
        // DECODE V: P6 canonical publication delegates fused field reconstruction/re-encoding.
        self.encode_columns(
            keys.keys(),
            keys.lanes(),
            keys.metadata().clone(),
            capacity,
            fields,
            dictionary,
            statuses,
        )
    }

    // DECODE V: P6 fused encoding reconstructs fields without a full i64 intermediate.
    fn encode_columns<'data, Row: RowCodec<'data>>(
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
        let supplied_bounds = fields
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
        for (field, &bound) in fields.iter().zip(&supplied_bounds) {
            let source = SourceView {
                column: ColumnView {
                    headers: std::ptr::null(),
                    data: keys.ptr(),
                    len: capacity as u64,
                    count: metadata.ptr().cast(),
                    codec: Codec::Plain as u32,
                },
                stride: lanes as u64,
                packed: 1,
                shift: field.shift,
                mask: if field.bits == 64 { u64::MAX } else { (1u64 << field.bits) - 1 },
                base: field.base,
            };
            columns.push(unsafe {
                EncodedColumn::from_gather_source_pending::<_, i64>(
                    self.pool.clone(),
                    source,
                    std::ptr::null(),
                    capacity,
                    metadata.clone(),
                    0,
                    bound,
                )?
            });
        }
        let columns = gpu_device_columns::PendingColumn::publish(columns)?;
        let bounds = columns.iter().map(|column| column.bounds()).collect();
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

#[cfg(test)]
mod publication_tests {
    use super::*;

    #[test]
    fn packed_publication_measures_fields_and_keeps_error_owners() -> Result<()> {
        let pool = Pool::new(0, 1 << 20, 256 << 20)?;
        let fields = [
            PackedField { shift: 0, bits: 64, base: i64::MIN },
            PackedField { shift: 64, bits: 8, base: -100 },
            PackedField { shift: 72, bits: 0, base: -7 },
        ];
        let dictionary = Arc::new(Dictionary::default());
        let mut adapter = PackedColumnsAdapter::new(pool.clone(), 3, Auxiliary::SUPPORTED.canon)?;
        for values in [vec![], vec![-9, -5, -9], vec![i64::MIN, i64::MAX]] {
            let capacity = values.len() + 2048;
            let words: Vec<u64> = values.iter().enumerate().flat_map(|(row, &value)| {
                [(value as u64).wrapping_sub(i64::MIN as u64), row as u64 + 3]
            }).collect();
            let keys = unsafe { Buffer::<u64>::uninitialized(pool.clone(), capacity * 2)? };
            unsafe { keys.write_async(&words)? };
            let metadata = Arc::new(Buffer::from_host(pool.clone(), &[
                OutputCount { rows: values.len() as u64, error: 0, padding: 0 },
            ])?);
            let output = adapter.encode_columns::<(i64, i64, i64)>(
                &keys, 2, metadata.clone(), capacity, &fields, dictionary.clone(), vec![],
            )?;
            let expected: Vec<_> = values.iter().enumerate()
                .map(|(row, &value)| (value, row as i64 - 97, -7)).collect();
            assert_eq!(output.to_host()?, expected);
            let bounds = vec![
                Bounds::from_values(&values),
                if values.is_empty() { Bounds::Empty } else {
                    Bounds::Inclusive { min: -97, max: values.len() as i64 - 98 }
                },
                if values.is_empty() { Bounds::Empty } else { Bounds::singleton(-7) },
            ];
            assert_eq!(output.storage().bounds, bounds);
            assert_eq!(
                output.storage().columns.iter().map(|column| column.bounds()).collect::<Vec<_>>(),
                bounds,
            );
            let failed = Arc::new(Buffer::from_host(pool.clone(), &[
                OutputCount { rows: 0, error: 73, padding: 0 },
            ])?);
            let failed_owner = Arc::downgrade(&failed);
            let failure = adapter.encode_columns::<(i64, i64, i64)>(
                &keys, 2, metadata.clone(), capacity, &fields, dictionary.clone(), vec![failed],
            )?;
            drop(keys);
            assert!(failed_owner.upgrade().is_some());
            assert!(failure.to_host().is_err());
            assert!(Arc::ptr_eq(&failure.storage().metadata, &metadata));
            assert_eq!(failure.storage().bounds, bounds);
            drop(failure);
            assert!(failed_owner.upgrade().is_none());
        }
        gpu_device_columns::release_cached_kernels();
        Ok(())
    }

    #[test]
    fn packed_publications_remain_exact_after_adapter_reuse_and_drop() -> Result<()> {
        let pool = Pool::new(0, 1 << 20, 256 << 20)?;
        let field = [PackedField { shift: 0, bits: 8, base: -100 }];
        let dictionary = Arc::new(Dictionary::default());
        let mut adapter = PackedColumnsAdapter::new(pool.clone(), 1, Auxiliary::SUPPORTED.canon)?;
        let first = adapter.packed_columns::<(i64,)>(
            Buffer::from_host(pool.clone(), &[7u64, 3, 7])?,
            3, &field, dictionary.clone(), vec![],
        )?;
        let empty = adapter.packed_columns::<(i64,)>(
            Buffer::from_host(pool, &[] as &[u64])?, 0, &field, dictionary, vec![],
        )?;
        drop(adapter);
        assert_eq!(first.storage().bounds, [Bounds::Inclusive { min: -97, max: -93 }]);
        assert_eq!(first.to_host()?, [(-93,), (-97,), (-93,)]);
        assert_eq!(empty.storage().bounds, [Bounds::Empty]);
        assert!(empty.to_host()?.is_empty());
        gpu_device_columns::release_cached_kernels();
        Ok(())
    }
}
