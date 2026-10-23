//! Stable row-ID selection for resident scans without an index build.
use crate::{build_module, config, pointer, release, temporary, Pool, RangeSelection, Result};
use cuda_core::DeviceBuffer;
use rccl_thrust::{arithmetic_range, select_flagged, DeviceBufferExt};
use std::sync::Arc;

/// Original base-row IDs satisfying a range predicate, in input order.
/// Only the first `len()` entries are initialized. Capacity is the source row
/// count, not the selected count. Device accesses must use the retained pool's
/// stream; destruction releases storage asynchronously on that stream.
pub struct SelectedRows {
    rows: Option<DeviceBuffer<u32>>,
    len: usize,
    pool: Arc<Pool>,
}
impl SelectedRows {
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
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
}
impl Drop for SelectedRows {
    fn drop(&mut self) {
        if let Some(rows) = self.rows.take() {
            let result = unsafe { rows.drop_async(self.pool.stream()) };
            self.pool.context().record_err(result);
        }
    }
}

/// Select original row IDs by `start <= decoded_value < end`, without sorting
/// or building a directory. RCCL flagged selection preserves source order.
/// One selected-count readback establishes the returned logical length.
pub fn select(pool: Arc<Pool>, selection: &RangeSelection) -> Result<SelectedRows> {
    if !Arc::ptr_eq(selection.column.pool(), &pool) {
        return Err("selection column must share the execution pool".into());
    }
    let base_rows = selection.column.rows();
    if base_rows > u32::MAX as usize {
        return Err("selection exceeds the u32 row-ID domain".into());
    }
    let stream = pool.stream();
    let mut rows = temporary::<u32>(stream, base_rows)?;
    let len = if base_rows == 0 {
        0
    } else {
        let module = build_module(&pool)?;
        let thrust = rccl_thrust::cuda::Program::load(pool.context())?;
        let exec = thrust.par_in(stream, pool.resource())?;
        let flags = temporary::<u32>(stream, base_rows)?;
        module.range_flags(
            stream,
            &module.prepare_range_flags(config(base_rows))?,
            selection.column.view(),
            base_rows as u32,
            selection.start,
            selection.end,
            pointer(&flags),
        )?;
        let mut count = temporary::<u64>(stream, 1)?;
        select_flagged(
            &exec,
            arithmetic_range(0_u32, 1, base_rows),
            flags.view(),
            rows.view_mut(),
            &mut count,
        )?;
        let len = count.to_host_vec(stream)?[0] as usize;
        release(stream, flags)?;
        release(stream, count)?;
        len
    };
    Ok(SelectedRows {
        rows: Some(rows),
        len,
        pool,
    })
}
