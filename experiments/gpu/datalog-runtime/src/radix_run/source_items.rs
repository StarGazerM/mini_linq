//! Source-backed occurrence columns. Sorting and merging move one shared row
//! reference, never logical payload lanes. Original encodings remain owned.
use super::{
    radix_kernels::LoadedModule, sealed, DeviceItemColumns, ItemKernels, LaneKind, OffsetWord,
    RadixItemsView,
};
use crate::{Result, ScalarTuple};
use cuda_core::{DeviceCopy, LaunchConfig1D};
use gpu_device_columns::{Buffer, Codec, Pool, SourceView};
use std::{any::Any, marker::PhantomData, sync::Arc};

/// An immutable source descriptor paired with its allocation owner and live prefix.
#[derive(Clone)]
pub struct OwnedSource {
    pool: Arc<Pool>,
    view: SourceView,
    rows: usize,
    _owner: Arc<dyn Any + Send + Sync>,
}
// SAFETY: the constructor requires immutable device storage owned by a Send+Sync
// owner. Raw descriptor pointers are never dereferenced on the host.
unsafe impl Send for OwnedSource {}
unsafe impl Sync for OwnedSource {}
impl OwnedSource {
    /// Retain an already resident source without reading device metadata.
    ///
    /// # Safety
    /// `owner` must retain every allocation addressed by `view` (including its
    /// count and vector headers), allocated in `pool`'s shared memory domain.
    /// Its first `rows` source rows must be initialized, immutable and ready on
    /// the consuming stream. Strided/packed projections must fit their backing
    /// allocation; `column.len` records logical source capacity, not word count.
    /// Any resident count must certify at least `rows`, without changing later.
    pub unsafe fn new(
        pool: Arc<Pool>, view: SourceView, rows: usize,
        owner: Arc<dyn Any + Send + Sync>,
    ) -> Result<Self> {
        if rows as u64 > view.column.len || view.stride == 0 || view.packed > 1 {
            return Err("invalid owned source capacity or projection metadata".into());
        }
        if view.column.codec > Codec::Dictionary as u32
            || (rows != 0 && view.column.data.is_null() && view.column.headers.is_null()
                && (view.packed == 0 || view.mask != 0))
            || (rows != 0 && view.column.codec != Codec::Plain as u32 && view.column.headers.is_null())
        {
            return Err("invalid owned source encoding metadata".into());
        }
        if view.packed != 0 {
            let width = 64 - view.mask.leading_zeros();
            if view.column.codec != Codec::Plain as u32 || !view.column.headers.is_null()
                || (view.mask != u64::MAX && view.mask & view.mask.wrapping_add(1) != 0)
                || u64::from(view.shift) + u64::from(width) > view.stride.saturating_mul(64)
            {
                return Err("invalid owned packed source field".into());
            }
        }
        Ok(Self { pool, view, rows, _owner: owner })
    }
    pub fn view(&self) -> SourceView { self.view }
    pub fn rows(&self) -> usize { self.rows }
    pub fn pool(&self) -> &Arc<Pool> { &self.pool }
    pub(super) fn check(&self, pool: &Arc<Pool>, positions: *const u32, rows: usize) -> Result<()> {
        if !pool.shares_memory(&self.pool) || (positions.is_null() && rows > self.rows) {
            return Err("radix source memory or live prefix differs from the build".into());
        }
        Ok(())
    }
}

struct SourceTable {
    views: Buffer<SourceView>,
    owners: Vec<OwnedSource>,
}

/// The address in the radix column table is a descriptor, not a native lane.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SourceItemView {
    /// Each row is [original source-table address, original source row].
    pub rows: *const [u64; 2],
    pub column: u32,
    pub reserved: u32,
}
unsafe impl DeviceCopy for SourceItemView {}

/// Explicit encoded item representation. All logical columns share one map.
/// `V` supplies scalar schema facts, independently of Rust type spelling.
pub struct SourceColumns<V: ScalarTuple> {
    tables: Vec<Arc<SourceTable>>,
    rows: Buffer<[u64; 2]>,
    descriptors: Buffer<SourceItemView>,
    /// The owning run's loaded radix module, retained for diagnostic reads.
    module: Option<LoadedModule>,
    live_rows: usize,
    marker: PhantomData<fn() -> V>,
}
// SAFETY: all raw pointers name immutable device descriptors or row maps.
// Tables retain Send+Sync owners, and device buffers retain their pool/stream.
unsafe impl<V: ScalarTuple> Send for SourceColumns<V> {}
unsafe impl<V: ScalarTuple> Sync for SourceColumns<V> {}
impl<V: ScalarTuple> SourceColumns<V> {
    pub(super) fn check_sources(
        pool: &Arc<Pool>, keys: &[OwnedSource], values: &[OwnedSource],
        positions: *const u32, rows: usize,
    ) -> Result<()> {
        if values.len() != V::COLUMNS.len() {
            return Err("one retained source per value schema field".into());
        }
        u32::try_from(rows).map_err(|_| "radix source rows exceed u32")?;
        for source in keys.iter().chain(values) { source.check(pool, positions, rows)?; }
        Ok(())
    }
    /// Construct final row references directly from the stable permutation and
    /// optional source selection. Neither borrowed pointer survives this build.
    /// `order` may be null only for an empty build.
    pub(super) unsafe fn from_order(
        kernels: &ItemKernels, values: &[OwnedSource],
        positions: *const u32, order: *const u32, rows: usize,
    ) -> Result<Self> {
        let pool = &kernels.pool;
        if V::COLUMNS.is_empty() {
            return Self::without_payload(pool.clone(), rows);
        }
        let table = Arc::new(SourceTable {
            views: Buffer::from_host_async(pool.clone(), &values.iter().map(OwnedSource::view).collect::<Vec<_>>())?,
            owners: values.to_vec(),
        });
        let module = &kernels.module;
        let map = unsafe { Buffer::uninitialized(pool.clone(), rows)? };
        if rows != 0 {
            let initial = module.prepare_source_rows(LaunchConfig1D::new(1, 256, 0))?;
            let config = config(initial.function(), rows)?;
            module.source_rows(pool.stream(), &module.prepare_source_rows(config)?,
                table.views.cu_deviceptr(), positions, order, rows as u64, map.ptr())?;
        }
        Self::with_rows(vec![table], map, rows, Some(module.clone()))
    }
    fn without_payload(pool: Arc<Pool>, live_rows: usize) -> Result<Self> {
        // Zero-length buffers allocate no device storage; no table, map or
        // source kernel is needed when the schema contains no payload fields.
        Self::with_rows(Vec::new(), unsafe { Buffer::uninitialized(pool, 0)? }, live_rows, None)
    }
    fn with_rows(tables: Vec<Arc<SourceTable>>, rows: Buffer<[u64; 2]>, live_rows: usize,
        module: Option<LoadedModule>) -> Result<Self> {
        let views = (0..V::COLUMNS.len()).map(|column| SourceItemView {
            rows: rows.ptr(), column: column as u32, reserved: 0,
        }).collect::<Vec<_>>();
        let descriptors = Buffer::from_host_async(rows.pool().clone(), &views)?;
        Ok(Self { tables, rows, descriptors, module, live_rows, marker: PhantomData })
    }
}
impl<V: ScalarTuple> sealed::Columns for SourceColumns<V> {}
impl<V: ScalarTuple> DeviceItemColumns for SourceColumns<V> {
    type Host = Vec<Vec<i64>>;
    const COLUMNS: usize = V::COLUMNS.len();
    fn native_column(_: usize) -> bool { false }
    fn check(&self, pool: &Arc<Pool>, rows: usize) -> Result<()> {
        if !pool.shares_memory(self.rows.pool()) || rows > self.live_rows {
            return Err("retained item memory or live length differs from its keys".into());
        }
        for table in &self.tables {
            for source in &table.owners {
                if !pool.shares_memory(source.pool()) {
                    return Err("retained source does not share radix memory".into());
                }
            }
        }
        Ok(())
    }
    fn lanes(out: &mut Vec<LaneKind>) {
        out.extend(V::COLUMNS.iter().map(|column| LaneKind {
            bits: column.bits(), signed: column.is_signed(),
        }));
    }
    fn gather(&self, kernels: &ItemKernels, order: *const u32, rows: usize) -> Result<Self> {
        if Self::COLUMNS == 0 { return Self::without_payload(kernels.pool.clone(), rows); }
        Self::with_rows(self.tables.clone(), self.rows.gather(kernels, order, rows)?, rows, self.module.clone())
    }
    fn merge<O: OffsetWord>(left: &Self, right: &Self, kernels: &ItemKernels,
        left_items: usize, sources: *const O, rows: usize) -> Result<Self> {
        if Self::COLUMNS == 0 { return Self::without_payload(kernels.pool.clone(), rows); }
        let mut tables = left.tables.clone();
        for table in &right.tables {
            if !tables.iter().any(|old| Arc::ptr_eq(old, table)) { tables.push(table.clone()); }
        }
        // References carry the original table identity, so stable merge never
        // rewrites table IDs or reads any payload value.
        let map = <Buffer<[u64; 2]> as DeviceItemColumns>::merge(
            &left.rows, &right.rows, kernels, left_items, sources, rows)?;
        Self::with_rows(tables, map, rows, left.module.clone())
    }
    fn column_pointers(&self, out: &mut Vec<u64>) {
        out.extend((0..Self::COLUMNS).map(|column|
            self.descriptors.cu_deviceptr() + (column * std::mem::size_of::<SourceItemView>()) as u64));
    }
    fn to_host(&self, rows: usize) -> Result<Self::Host> {
        self.check(self.rows.pool(), rows)?;
        let mut columns = Vec::with_capacity(Self::COLUMNS);
        for column in 0..Self::COLUMNS {
            let output = unsafe { Buffer::<i64>::uninitialized(self.rows.pool().clone(), rows)? };
            if rows != 0 {
                let module = self.module.as_ref().expect("payload schema retains its read module");
                let initial = module.prepare_diagnostic_items(LaunchConfig1D::new(1, 256, 0))?;
                let config = config(initial.function(), rows)?;
                module.diagnostic_items(self.rows.pool().stream(),
                    &module.prepare_diagnostic_items(config)?, self.descriptors.ptr(),
                    column as u32, rows as u64, output.ptr())?;
            }
            columns.push(output.to_host()?);
        }
        Ok(columns)
    }
}
fn config(function: &cuda_core::CudaFunction, rows: usize) -> Result<LaunchConfig1D> {
    let grid = cq_gpu_runtime::launch_grid_for_kernel(function, rows.div_ceil(256), 256, 0)?;
    Ok(LaunchConfig1D::new(grid, 256, 0))
}

pub mod device {
    use super::{RadixItemsView, SourceItemView};
    use gpu_device_columns::SourceView;
    /// Late logical payload read, authorized only by a retained-source item recipe.
    ///
    /// # Safety
    /// The checked provider must record retained-source representation for this
    /// column. The snapshot and all retained owners must be live and ready;
    /// `run`, `column` and `occurrence` must be in bounds.
    #[inline(always)]
    pub unsafe fn item(view: RadixItemsView, run: u32, column: u32, occurrence: u64) -> i64 {
        let address = unsafe { *view.columns.add((run * view.column_count + column) as usize) };
        // DECODE V: retained payload reconstruction serves existing P6 relational
        // consumers, not a falsely claimed publication boundary.
        unsafe { read(*(address as *const SourceItemView), occurrence) }
    }
    #[inline(always)]
    pub(in crate::radix_run) unsafe fn read(view: SourceItemView, occurrence: u64) -> i64 {
        let [table, row] = unsafe { *view.rows.add(occurrence as usize) };
        let source = unsafe { *((table as *const SourceView).add(view.column as usize)) };
        // DECODE ?: shared retained-source inverse inherits consuming read facts.
        unsafe { gpu_device_columns::decode_source(source, row) }
    }
}
