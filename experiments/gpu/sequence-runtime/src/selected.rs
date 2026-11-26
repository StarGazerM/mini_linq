//! Row-ID selection for resident scans without an index build (GPU-SelectRows).
use crate::{
    build_module, kernels, pointer, release, temporary, Auxiliary, Geometry, Pool, RangeSelection,
    Result,
};
use crate::DeviceBuffer;
use std::sync::Arc;

/// Original base-row IDs satisfying a range predicate, in source order within
/// each 256-row chunk; chunks follow in unspecified order.
/// Only the first `len()` entries are initialized. Capacity is the source row
/// count, not the selected count. The selected count stays on the device
/// (GPU-SelectedCount): dispatches read it there, and `len()` reads it back
/// only when host code needs it. Device accesses must use the retained pool's
/// stream; destruction releases storage asynchronously on that stream.
pub struct SelectedRows {
    rows: Option<DeviceBuffer<u32>>,
    count: Option<DeviceBuffer<u32>>,
    capacity: usize,
    len: std::cell::OnceCell<usize>,
    pool: Arc<Pool>,
}
impl SelectedRows {
    /// The selected count, read back once on first use.
    pub fn len(&self) -> Result<usize> {
        if let Some(&len) = self.len.get() {
            return Ok(len);
        }
        let len = match &self.count {
            Some(count) => count.to_host_vec()?[0] as usize,
            None => 0,
        };
        Ok(*self.len.get_or_init(|| len))
    }
    /// Source rows: an upper bound on `len()` that needs no readback.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    /// Allocated row-ID storage, including the unused suffix.
    pub fn bytes(&self) -> usize {
        self.rows().len() * std::mem::size_of::<u32>()
    }
    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }
    /// Storage for generated scan kernels; read only the `len()` live prefix.
    pub fn rows(&self) -> &DeviceBuffer<u32> {
        self.rows.as_ref().unwrap()
    }
    /// Nonowning device pointer, valid while this owner lives on its stream.
    pub fn ptr(&self) -> *const u32 {
        self.rows().cu_deviceptr() as *const u32
    }
    /// Nonowning device pointer to the selected count (one `u32`), valid while
    /// this owner lives on its stream.
    pub fn count_ptr(&self) -> *const u32 {
        self.count.as_ref().unwrap().cu_deviceptr() as *const u32
    }
}

/// GPU-SelectRows: launch the one-pass ordered-chunk selection of row IDs with
/// `start <= decoded_value < end` into `out` (capacity `base_rows`) and return
/// the device count. Rows of one 256-row chunk keep source order.
pub(crate) fn launch_select(
    pool: &Arc<Pool>,
    module: &kernels::build::LoadedModule,
    geometry: Geometry,
    selection: &RangeSelection,
    base_rows: usize,
    out: &DeviceBuffer<u32>,
) -> Result<DeviceBuffer<u32>> {
    let stream = pool.stream();
    let count = temporary::<u32>(pool, 1)?;
    unsafe {
        cuda_core::simt::memory::memset_d8_async(
            count.cu_deviceptr(),
            0,
            std::mem::size_of::<u32>(),
            stream.cu_stream(),
        )?;
    }
    if base_rows == 0 {
        return Ok(count);
    }
    let shape = module.prepare_range_select(geometry.config(1))?;
    let blocks = geometry.grid(shape.function(), base_rows)?;
    // DECODE V: selection launches value reconstruction before range comparison; P6 evaluate translated encoded predicates.
    module.range_select(
        stream,
        &module.prepare_range_select(geometry.config(blocks))?,
        selection.column.view(),
        base_rows as u32,
        selection.start,
        selection.end,
        pointer(out),
        pointer(&count),
    )?;
    Ok(count)
}

/// Select original row IDs by `start <= decoded_value < end`, without sorting
/// or building a directory (GPU-SelectRows: source order within each 256-row
/// chunk). The count stays on the device; nothing is read back here.
/// `geometry` is the recorded `selection` family geometry.
pub fn select(
    pool: Arc<Pool>,
    selection: &RangeSelection,
    geometry: Geometry,
) -> Result<SelectedRows> {
    geometry.check("selection", Auxiliary::SUPPORTED.selection)?;
    if !Arc::ptr_eq(selection.column.pool(), &pool) {
        return Err("selection column must share the execution pool".into());
    }
    let base_rows = selection.column.rows();
    if base_rows > u32::MAX as usize {
        return Err("selection exceeds the u32 row-ID domain".into());
    }
    let rows = temporary::<u32>(&pool, base_rows)?;
    let module = build_module(&pool)?;
    let count = Some(launch_select(&pool, &module, geometry, selection, base_rows, &rows)?);
    Ok(SelectedRows {
        rows: Some(rows),
        count,
        capacity: base_rows,
        len: std::cell::OnceCell::new(),
        pool,
    })
}
