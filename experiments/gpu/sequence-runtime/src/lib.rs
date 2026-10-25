//! Sequence execution primitives over resident columns. Callers supply the
//! recorded key order, selection and payload; this crate makes no plan choices.
pub mod canon;
pub mod device;
pub mod group;
mod kernels;
pub mod selected;
pub use selected::{select, SelectedRows};

pub use cq_gpu_runtime::{Bounds, Buffer, Codec, ColumnView, DeviceColumns, EncodedColumn, Pool};
use cuda_core::{CudaContext, CudaStream, DeviceBuffer, DeviceCopy, LaunchConfig1D};
use gpu_compress::{Catalog, Domain};
use gpu_compress_host::Kind;
pub use gpu_device_columns::{MemoryResource, OutputCount};
use rccl_cub::DoubleBuffer;
use rccl_thrust::prelude::*;
use rccl_thrust::{
    arithmetic_range, gather, lower_bound, select_flagged, sort_columns_lexicographic,
};
use std::{cell::RefCell, sync::Arc};

pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

thread_local! {
    // Scratch capacity only: every invocation repopulates every live lane.
    static NUMERIC_UPLOAD_SCRATCH: RefCell<Vec<Vec<i64>>> = const { RefCell::new(Vec::new()) };
}

thread_local! {
    static QUERY_CLEANUP: RefCell<Option<Vec<fn()>>> = const { RefCell::new(None) };
}

/// Own the lifetime of generated thread-local caches for one complete query.
/// Keep this scope alive across warmup and measured iterations, then drop it
/// after all query outputs and inputs. It must stay on its creating thread.
pub struct QueryScope(std::marker::PhantomData<std::rc::Rc<()>>);

impl QueryScope {
    pub fn new() -> Self {
        QUERY_CLEANUP.with(|cleanup| {
            let mut cleanup = cleanup.borrow_mut();
            assert!(cleanup.is_none(), "GPU query scopes cannot be nested");
            *cleanup = Some(Vec::new());
        });
        Self(std::marker::PhantomData)
    }
}

impl Default for QueryScope {
    fn default() -> Self {
        Self::new()
    }
}

/// Register an emitted cache's release hook once per active query scope.
#[doc(hidden)]
pub fn register_query_cleanup(release: fn()) {
    QUERY_CLEANUP.with(|cleanup| {
        if let Some(cleanup) = cleanup.borrow_mut().as_mut() {
            if !cleanup.iter().any(|&existing| std::ptr::fn_addr_eq(existing, release)) {
                cleanup.push(release);
            }
        }
    });
}

impl Drop for QueryScope {
    fn drop(&mut self) {
        let cleanup = QUERY_CLEANUP.with(|cleanup| cleanup.borrow_mut().take());
        for release in cleanup.into_iter().flatten() {
            release();
        }
        RANGE_RADIX_WORKSPACE.with(|cache| cache.borrow_mut().take());
        TEMP_U32.with(|cache| cache.borrow_mut().take());
        TEMP_U64.with(|cache| cache.borrow_mut().take());
        canon::release_query_cache();
        BUILD_MODULE.with(|cache| cache.borrow_mut().take());
        gpu_device_columns::release_cached_kernels();
        NUMERIC_UPLOAD_SCRATCH.with(|cache| cache.borrow_mut().clear());
    }
}

/// Upload primitive numeric columns emitted by a host Sequence boundary.
/// The translator supplies one i64 lane per declared field, in layout order.
/// This avoids re-encoding primitive tuples through a dictionary and scratch
/// row vector. Scratch capacities are reused, never relation contents, and all
/// realization and upload work remains inside the provider's build timing.
#[doc(hidden)]
pub fn upload_numeric_columns<Row: cq_gpu_runtime::OwnedRowCodec, F>(
    pool: Arc<Pool>,
    fill: F,
) -> Result<DeviceColumns<'static, Row>>
where
    F: FnOnce(&mut [Vec<i64>]) -> Result<()>,
{
    // Check out the scratch rather than holding a RefCell borrow across host
    // evaluation: an opaque host expression may invoke another query.
    let mut raw = NUMERIC_UPLOAD_SCRATCH.with(|cache| std::mem::take(&mut *cache.borrow_mut()));
    raw.resize_with(Row::WIDTH, Vec::new);
    let result = (|| {
        fill(&mut raw)?;
        upload_numeric_lanes(pool, &raw)
    })();
    for column in &mut raw {
        column.clear();
    }
    NUMERIC_UPLOAD_SCRATCH.with(|cache| *cache.borrow_mut() = raw);
    result
}

fn upload_numeric_lanes<Row: cq_gpu_runtime::OwnedRowCodec>(
    pool: Arc<Pool>,
    raw: &[Vec<i64>],
) -> Result<DeviceColumns<'static, Row>> {
    let rows = raw.first().ok_or("numeric upload needs a column")?.len();
    if raw.len() != Row::WIDTH || raw.iter().any(|column| column.len() != rows) {
        return Err("numeric upload columns must match the row layout".into());
    }
    if rows > u32::MAX as usize {
        return Err("numeric upload exceeds the u32 row domain".into());
    }
    let metadata = Arc::new(Buffer::from_host(pool.clone(), &[gpu_device_columns::OutputCount {
        rows: rows as u64,
        error: 0,
        padding: 0,
    }])?);
    let columns = raw
        .iter()
        .map(|values| {
            let input = Buffer::from_host(pool.clone(), values)?;
            let bounds = if rows == 0 {
                Bounds::Empty
            } else {
                let blocks = rows.div_ceil(256).min(568);
                let partial = unsafe { Buffer::<i64>::uninitialized(pool.clone(), blocks * 16)? };
                let module = build_module(&pool)?;
                module.numeric_bounds(
                    pool.stream(),
                    &module.prepare_numeric_bounds(LaunchConfig1D::new(blocks as u32, 256, 0))?,
                    input.ptr(),
                    rows as u32,
                    partial.ptr(),
                )?;
                let partial = partial.to_host()?;
                let (min, max) = partial.chunks_exact(2).fold(
                    (i64::MAX, i64::MIN),
                    |(min, max), pair| (min.min(pair[0]), max.max(pair[1])),
                );
                Bounds::Inclusive { min, max }
            };
            // This upload initialized the complete prefix. The immutable
            // metadata retains its exact count; no compact-extent readback
            // is needed in a timed provider build.
            unsafe {
                EncodedColumn::from_device(
                    pool.clone(), input.ptr(), rows, metadata.clone(), 0, Codec::For, bounds,
                ).map(Arc::new)
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let bounds = columns.iter().map(|column| column.bounds()).collect();
    let host_views: Vec<_> = columns.iter().map(|column| unsafe { column.view() }).collect();
    let views = Arc::new(Buffer::from_host(pool.clone(), &host_views)?);
    // Columns are initialized on this pool's stream; the table retains every
    // owner and its exact row count. Primitive lanes need no dictionary IDs.
    Ok(unsafe {
        DeviceColumns::from_parts(
            gpu_device_columns::Table {
                columns, views, metadata, capacity: rows, bounds, statuses: Vec::new(),
            },
            Arc::new(cq_gpu_runtime::Dictionary::default()),
            pool,
        )
    })
}

/// Host codec for the catalog's shared, order-preserving dense ID domain.
/// Missing constants stay absent rather than aliasing any stored ID.
#[derive(Clone, Copy)]
pub struct DenseDictionary<'a> {
    domain: &'a Domain,
}
impl<'a> DenseDictionary<'a> {
    pub fn new(domain: &'a Domain) -> Self {
        Self { domain }
    }
    pub fn from_catalog(catalog: &'a Catalog, name: &str) -> Result<Self> {
        Ok(Self::new(catalog.domain(name)?))
    }
    pub fn domain(&self) -> &'a Domain {
        self.domain
    }
    pub fn encode(&self, value: i64) -> Option<u32> {
        self.domain.id(value)
    }
    pub fn decode(&self, id: u32) -> Option<i64> {
        ((id as usize) < self.len()).then(|| self.domain.value(id))
    }
    pub fn len(&self) -> usize {
        self.domain.len()
    }
    pub fn is_empty(&self) -> bool {
        self.domain.is_empty()
    }
    pub fn bits(&self) -> u32 {
        self.domain.bits()
    }
    pub fn bounds(&self) -> Bounds {
        self.domain.bounds()
    }
}

/// An owning source column and its known live row prefix.
#[derive(Clone)]
pub struct ColumnDescriptor {
    owner: ColumnOwner,
    rows: usize,
    bounds: Bounds,
    source: gpu_device_columns::SourceView,
}
#[derive(Clone)]
enum ColumnOwner {
    Encoded(Arc<EncodedColumn>),
    Rows(Arc<Buffer<i64>>),
    Packed(Arc<Buffer<u64>>),
}
impl ColumnDescriptor {
    pub fn from_catalog(catalog: &Catalog, table: &str, column: &str) -> Result<Self> {
        unsafe {
            Self::new(
                catalog.column(table, column)?.column.clone(),
                catalog.rows(table)?,
            )
        }
    }
    /// Describe an explicit resident column without reading its device count.
    ///
    /// # Safety
    /// `rows` must be the initialized live prefix, ready on the column's pool
    /// stream. Its count must not change during the build.
    pub unsafe fn new(column: Arc<EncodedColumn>, rows: usize) -> Result<Self> {
        if rows > column.capacity() {
            return Err("source rows exceed column capacity".into());
        }
        Ok(Self {
            bounds: column.bounds(),
            source: unsafe { column.view() }.into(),
            owner: ColumnOwner::Encoded(column),
            rows,
        })
    }
    /// Project a field from resident row-major storage without copying.
    ///
    /// # Safety
    /// The first `count` rows must be initialized on the buffer's pool stream,
    /// remain immutable during the build, and have values enclosed by `bounds`.
    pub unsafe fn from_rows(
        rows: Arc<Buffer<i64>>,
        count: usize,
        width: usize,
        slot: usize,
        bounds: Bounds,
    ) -> Result<Self> {
        if slot >= width || count.checked_mul(width).is_none_or(|len| len > rows.len()) {
            return Err("stored row projection exceeds its buffer".into());
        }
        let data = rows
            .ptr()
            .wrapping_add(if count == 0 { 0 } else { slot })
            .cast();
        let mut source = Self::native_source(data, count);
        source.stride = width as u64;
        Ok(Self {
            owner: ColumnOwner::Rows(rows),
            rows: count,
            bounds,
            source,
        })
    }
    /// Project a recorded canonical-key field without unpacking its column.
    ///
    /// # Safety
    /// The first `count` keys must be initialized on the buffer's pool stream
    /// and immutable during the build. Decoded values must fit i64 and `bounds`.
    pub unsafe fn from_packed(
        keys: Arc<Buffer<u64>>,
        count: usize,
        lanes: usize,
        field: canon::PackedField,
        bounds: Bounds,
    ) -> Result<Self> {
        if lanes == 0
            || count > keys.len() / lanes
            || field.bits > 64
            || field.shift as usize > lanes.saturating_mul(64)
            || field.bits as usize > lanes.saturating_mul(64) - field.shift as usize
        {
            return Err("packed source projection exceeds its buffer or key".into());
        }
        let mut source = Self::native_source(keys.ptr(), count);
        source.packed = 1;
        source.stride = lanes as u64;
        source.shift = field.shift;
        source.mask = if field.bits == 64 {
            u64::MAX
        } else {
            (1u64 << field.bits) - 1
        };
        source.base = field.base;
        Ok(Self {
            owner: ColumnOwner::Packed(keys),
            rows: count,
            bounds,
            source,
        })
    }
    fn native_source(data: *const u64, rows: usize) -> gpu_device_columns::SourceView {
        ColumnView {
            headers: std::ptr::null(),
            data,
            len: rows as u64,
            count: std::ptr::null(),
            codec: Codec::Plain as u32,
        }
        .into()
    }
    pub fn pool(&self) -> &Arc<Pool> {
        match &self.owner {
            ColumnOwner::Encoded(column) => column.pool(),
            ColumnOwner::Rows(rows) => rows.pool(),
            ColumnOwner::Packed(keys) => keys.pool(),
        }
    }
    pub fn bounds(&self) -> Bounds {
        self.bounds
    }
    pub fn rows(&self) -> usize {
        self.rows
    }
    fn view(&self) -> gpu_device_columns::SourceView {
        self.source
    }
}

/// One dense u32 key level, in lexicographic order. The first level owns the
/// domain+1 lower-bound directory; subsequent levels order rows within ranges.
#[derive(Clone)]
pub struct KeyDescriptor {
    pub column: ColumnDescriptor,
    pub domain_size: usize,
}
#[derive(Clone)]
pub struct RangeSelection {
    pub column: ColumnDescriptor,
    pub start: i64,
    pub end: i64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadKind {
    U32,
    I32,
    I64,
}
#[derive(Clone)]
pub struct PayloadDescriptor {
    pub column: ColumnDescriptor,
    pub kind: PayloadKind,
}
#[derive(Clone)]
pub struct RangeDescriptor {
    pub key: Vec<KeyDescriptor>,
    /// A half-open predicate applied before sorting, not a post-build filter.
    pub selection: Option<RangeSelection>,
    pub payload: Vec<PayloadDescriptor>,
}

impl RangeDescriptor {
    fn base_rows(&self) -> Result<usize> {
        self.key
            .first()
            .map(|key| &key.column)
            .or_else(|| self.payload.first().map(|payload| &payload.column))
            .or_else(|| self.selection.as_ref().map(|selection| &selection.column))
            .map(|column| column.rows)
            .ok_or_else(|| "range descriptor needs a column establishing its live row count".into())
    }

    fn directory_entries(&self) -> Option<usize> {
        self.key
            .first()
            .map_or(Some(0), |key| key.domain_size.checked_add(1))
    }
}

/// Catalog names are relative to `table`; dictionary sizes remain explicit.
pub struct CatalogKeyDescriptor {
    pub column: String,
    pub domain_size: usize,
}
pub struct CatalogRangeSelection {
    pub column: String,
    pub start: i64,
    pub end: i64,
}
pub struct CatalogRangeDescriptor {
    pub table: String,
    pub key: Vec<CatalogKeyDescriptor>,
    pub selection: Option<CatalogRangeSelection>,
    pub payload: Vec<String>,
}
impl RangeDescriptor {
    pub fn from_catalog(catalog: &Catalog, descriptor: &CatalogRangeDescriptor) -> Result<Self> {
        let column = |name: &str| ColumnDescriptor::from_catalog(catalog, &descriptor.table, name);
        let key = descriptor
            .key
            .iter()
            .map(|key| {
                if !matches!(
                    &catalog.column(&descriptor.table, &key.column)?.kind,
                    Kind::Id(_)
                ) {
                    return Err("range keys must be dense domain ID columns".into());
                }
                Ok(KeyDescriptor {
                    column: column(&key.column)?,
                    domain_size: key.domain_size,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let selection = descriptor
            .selection
            .as_ref()
            .map(|selection| -> Result<RangeSelection> {
                Ok(RangeSelection {
                    column: column(&selection.column)?,
                    start: selection.start,
                    end: selection.end,
                })
            })
            .transpose()?;
        let payload = descriptor
            .payload
            .iter()
            .map(|name| {
                let kind = match &catalog.column(&descriptor.table, name)?.kind {
                    Kind::Id(_) => PayloadKind::U32,
                    Kind::I32 => PayloadKind::I32,
                };
                Ok(PayloadDescriptor {
                    column: column(name)?,
                    kind,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            key,
            selection,
            payload,
        })
    }
}

/// Nonowning generated-kernel ABI. Keep the provider alive and use its stream.
/// Offsets have `domain + 1` entries; payload columns have `len` entries.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RangeView {
    pub offsets: ColumnView,
    pub payload: *const ColumnView,
    pub payload_columns: u32,
    pub domain: u32,
    pub len: u32,
}

/// An immutable, duplicate-preserving sorted range index. A keyed lookup is
/// [offsets[key], offsets[key+1]); an empty key denotes [0,len) without a directory.
pub struct RangeProvider {
    offsets: EncodedColumn,
    payload: Vec<EncodedColumn>,
    payload_views: Arc<Buffer<ColumnView>>,
    domain: u32,
    len: u32,
}
impl RangeProvider {
    pub fn build(pool: Arc<Pool>, descriptor: &RangeDescriptor) -> Result<Self> {
        RangeBuilder::new(&pool)?.build(descriptor)
    }
    pub fn from_catalog(
        pool: Arc<Pool>,
        catalog: &Catalog,
        descriptor: &CatalogRangeDescriptor,
    ) -> Result<Self> {
        Self::build(pool, &RangeDescriptor::from_catalog(catalog, descriptor)?)
    }
    pub fn len(&self) -> usize {
        self.len as usize
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn domain_size(&self) -> usize {
        self.domain as usize
    }
    pub fn pool(&self) -> &Arc<Pool> {
        self.offsets.pool()
    }
    pub fn offsets(&self) -> &EncodedColumn {
        &self.offsets
    }
    pub fn payload(&self) -> &[EncodedColumn] {
        &self.payload
    }
    pub fn bytes(&self) -> usize {
        self.offsets.bytes()
            + self.payload_views.bytes()
            + self.payload.iter().map(EncodedColumn::bytes).sum::<usize>()
    }
    /// # Safety
    /// The returned views may only be read on `self.pool().stream()` while
    /// this provider is alive, within the advertised domain/row/column bounds.
    pub unsafe fn view(&self) -> RangeView {
        RangeView {
            offsets: unsafe { self.offsets.view() },
            payload: self.payload_views.ptr(),
            payload_columns: self.payload.len() as u32,
            domain: self.domain,
            len: self.len,
        }
    }
}

thread_local! {
    static BUILD_MODULE: RefCell<Option<(Arc<CudaContext>, kernels::build::LoadedModule)>> =
        const { RefCell::new(None) };
    static RANGE_RADIX_WORKSPACE: RefCell<Option<RangeRadixWorkspace>> =
        const { RefCell::new(None) };
}

/// Release this thread's idle canonicalization and range-sort device scratch.
///
/// Call after dropping stratum-local providers, before [`Pool::release_unused`].
/// Published results and checked-out providers retain their own allocations.
/// Frees follow prior uses on the matching pool's stream; this does not wait
/// for completion. Native CUDA scratch is not included in pool memory stats.
pub fn release_cached_scratch(pool: &Arc<Pool>) {
    canon::release_cached_scratch(pool);
    let scratch = RANGE_RADIX_WORKSPACE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.as_ref().is_some_and(|scratch| Arc::ptr_eq(&scratch.pool, pool)) {
            cache.take()
        } else {
            None
        }
    });
    drop(scratch);
}

/// Keep only the most recently used context's named build module on this thread.
pub(crate) fn build_module(pool: &Pool) -> Result<kernels::build::LoadedModule> {
    BUILD_MODULE.with(|cache| {
        if let Some((context, module)) = cache.borrow().as_ref() {
            if Arc::ptr_eq(context, pool.context()) {
                return Ok(module.clone());
            }
        }
        let module = unsafe { kernels::build::load(pool.context())? };
        *cache.borrow_mut() = Some((pool.context().clone(), module.clone()));
        Ok(module)
    })
}

struct RangeRadixWorkspace {
    rows: usize,
    workspace: rccl_cub::DeviceRadixSortBufferWorkspace,
    pool: Arc<Pool>,
}
impl Drop for RangeRadixWorkspace {
    fn drop(&mut self) {
        // Match CanonScratch: consuming these controls is not exposed by RCCL.
        // Empty replacements allocate nothing; real frees stay stream ordered.
        fn release<T: DeviceCopy>(pool: &Pool, buffer: &mut DeviceBuffer<T>) {
            let empty = unsafe { DeviceBuffer::uninitialized_async(pool.stream(), 0) }
                .expect("zero-length device buffer needs no allocation");
            let buffer = std::mem::replace(buffer, empty);
            pool.context().record_err(unsafe { buffer.drop_async(pool.stream()) });
        }
        let (bins, lookback, counters) = self.workspace.__parts_mut();
        release(&self.pool, bins);
        release(&self.pool, lookback);
        release(&self.pool, counters);
    }
}

fn with_range_radix_workspace(
    pool: &Arc<Pool>,
    launch: &rccl_cub::cuda::Launch<'_, '_>,
    rows: usize,
    sort: impl FnOnce(&mut rccl_cub::DeviceRadixSortBufferWorkspace) -> Result<()>,
) -> Result<()> {
    RANGE_RADIX_WORKSPACE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache
            .as_ref()
            .is_none_or(|scratch| !Arc::ptr_eq(&scratch.pool, pool) || scratch.rows != rows)
        {
            // Keep only the latest exact-row workspace, not one allocation
            // for every relation size encountered on this thread. Free before
            // allocating its replacement to avoid overlapping their lifetimes.
            drop(cache.take());
            pool.context().bind_to_thread()?;
            *cache = Some(RangeRadixWorkspace {
                rows,
                workspace: rccl_cub::device_radix_sort_pairs_buffer_workspace::<u32, u32>(launch, rows)?,
                pool: pool.clone(),
            });
        }
        let scratch = cache.as_mut().unwrap();
        // RCCL clears the entire lookback allocation each pass, so retain exact
        // row-sized controls rather than clearing the largest provider's scratch.
        // The retained pool fixes the stream, so successive uses remain ordered.
        sort(&mut scratch.workspace)
    })
}

/// Per-provider count owners and descriptor storage. Construct during program
/// setup; published results retain their owners across later builds.
pub struct RangeWorkspace {
    row_count: Arc<Buffer<u64>>,
    offset_count: Arc<Buffer<u64>>,
    payload_views: Arc<Buffer<ColumnView>>,
    host_views: Vec<ColumnView>,
    row_value: usize,
    base_rows: usize,
    entries: usize,
}
impl RangeWorkspace {
    pub fn new(pool: Arc<Pool>, descriptor: &RangeDescriptor) -> Result<Self> {
        let rows = descriptor.base_rows()?;
        let entries = descriptor
            .directory_entries()
            .ok_or("range domain overflow")?;
        Ok(Self {
            row_count: Arc::new(Buffer::from_host(pool.clone(), &[rows as u64])?),
            offset_count: Arc::new(Buffer::from_host(pool.clone(), &[entries as u64])?),
            payload_views: Arc::new(unsafe {
                Buffer::uninitialized(pool, descriptor.payload.len())?
            }),
            host_views: Vec::with_capacity(descriptor.payload.len()),
            row_value: rows,
            base_rows: rows,
            entries,
        })
    }
    pub fn matches(&self, pool: &Arc<Pool>, descriptor: &RangeDescriptor) -> bool {
        Arc::ptr_eq(self.row_count.pool(), pool)
            && descriptor.base_rows().ok() == Some(self.base_rows)
            && descriptor.directory_entries() == Some(self.entries)
            && descriptor.payload.len() == self.payload_views.len()
    }
}

/// Reuse loaded build/RCCL programs across providers on the existing pool.
/// Fresh builders also share the named module and radix controls on this thread.
pub struct RangeBuilder<'pool> {
    pool: &'pool Arc<Pool>,
    module: kernels::build::LoadedModule,
    thrust: rccl_thrust::cuda::Program<'pool>,
    cub: rccl_cub::cuda::Program<'pool>,
}
impl<'pool> RangeBuilder<'pool> {
    pub fn new(pool: &'pool Arc<Pool>) -> Result<Self> {
        Ok(Self {
            module: build_module(pool)?,
            thrust: rccl_thrust::cuda::Program::load(pool.context())?,
            cub: rccl_cub::cuda::Program::load(pool.context())?,
            pool,
        })
    }
    pub fn pool(&self) -> &Arc<Pool> {
        self.pool
    }
    pub fn from_catalog(
        &self,
        catalog: &Catalog,
        descriptor: &CatalogRangeDescriptor,
    ) -> Result<RangeProvider> {
        self.build(&RangeDescriptor::from_catalog(catalog, descriptor)?)
    }
    pub fn build(&self, descriptor: &RangeDescriptor) -> Result<RangeProvider> {
        let mut workspace = RangeWorkspace::new(self.pool.clone(), descriptor)?;
        self.build_with_workspace(descriptor, &mut workspace)
    }
    /// Build on the existing stream, reusing setup-time count and view owners.
    pub fn build_with_workspace(
        &self,
        descriptor: &RangeDescriptor,
        workspace: &mut RangeWorkspace,
    ) -> Result<RangeProvider> {
        if !workspace.matches(self.pool, descriptor) {
            return Err("range workspace does not match the provider descriptor".into());
        }
        let base_rows = descriptor.base_rows()?;
        if base_rows > u32::MAX as usize || descriptor.payload.len() > u32::MAX as usize {
            return Err("range provider exceeds the u32 row/column domain".into());
        }
        let check_column = |column: &ColumnDescriptor| -> Result<()> {
            if column.rows != base_rows || !Arc::ptr_eq(column.pool(), &self.pool) {
                return Err(
                    "range columns must have identical live row counts and share the builder pool"
                        .into(),
                );
            }
            Ok(())
        };
        for key in &descriptor.key {
            check_column(&key.column)?;
            if key.domain_size >= u32::MAX as usize {
                return Err("dense key domain must leave u32::MAX available as a sentinel".into());
            }
            match key.column.bounds() {
                Bounds::Empty if base_rows == 0 => {}
                Bounds::Inclusive { min, max }
                    if min >= 0 && max >= min && max < key.domain_size as i64 => {}
                _ => return Err("key bounds do not fit the declared dense domain".into()),
            }
        }
        if let Some(selection) = &descriptor.selection {
            check_column(&selection.column)?;
        }
        for payload in &descriptor.payload {
            check_column(&payload.column)?;
            let (low, high) = match payload.kind {
                PayloadKind::U32 => (0, u32::MAX as i64),
                PayloadKind::I32 => (i32::MIN as i64, i32::MAX as i64),
                PayloadKind::I64 => (i64::MIN, i64::MAX),
            };
            match payload.column.bounds() {
                Bounds::Empty if base_rows == 0 => {}
                Bounds::Inclusive { min, max } if min >= low && min <= max && max <= high => {}
                _ => return Err("payload bounds do not fit the declared lane type".into()),
            }
        }
        let pool = self.pool;
        let stream = pool.stream();
        let exec = self.thrust.par_in(stream, pool.resource())?;
        let launch = self.cub.on(stream)?;
        let module = &self.module;
        let (rows, selected) = match &descriptor.selection {
            Some(selection) if base_rows > 0 => {
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
                let mut selected = temporary::<u32>(stream, base_rows)?;
                let mut count = temporary::<u64>(stream, 1)?;
                select_flagged(
                    &exec,
                    arithmetic_range(0_u32, 1, base_rows),
                    flags.view(),
                    selected.view_mut(),
                    &mut count,
                )?;
                let rows = count.to_host_vec(stream)?[0] as usize;
                release(stream, flags)?;
                release(stream, count)?;
                (rows, Some(selected))
            }
            _ => (base_rows, None),
        };
        let entries = workspace.entries;
        if rows != workspace.row_value {
            if Arc::strong_count(&workspace.row_count) != 1 {
                workspace.row_count =
                    Arc::new(Buffer::from_host_async(pool.clone(), &[rows as u64])?);
            } else {
                // The workspace is the only owner; all prior reads on this
                // pool stream precede this asynchronous update.
                unsafe { workspace.row_count.write_async(&[rows as u64])? };
            }
            workspace.row_value = rows;
        }
        let row_count = workspace.row_count.clone();
        let offset_count = workspace.offset_count.clone();
        if descriptor.key.is_empty() {
            let source = selected
                .as_ref()
                .map_or(std::ptr::null(), |selected| pointer(selected) as *const u32);
            let payload = descriptor
                .payload
                .iter()
                .map(|payload| unsafe {
                    pack_payload(pool, payload, source, rows, row_count.clone())
                })
                .collect::<Result<Vec<_>>>()?;
            let offsets = unsafe {
                EncodedColumn::from_device(
                    pool.clone(),
                    std::ptr::null::<i64>(),
                    0,
                    offset_count,
                    0,
                    Codec::For,
                    Bounds::Empty,
                )?
            };
            if let Some(selected) = selected {
                release(stream, selected)?;
            }
            return finish(offsets, payload, 0, rows, workspace);
        }
        let first = &descriptor.key[0];
        // Empty selections still own a complete all-zero directory.
        if rows == 0 {
            let zeros = Buffer::<i64>::zeroed(pool.clone(), entries)?;
            let offsets = unsafe {
                EncodedColumn::from_device(
                    pool.clone(),
                    zeros.ptr(),
                    entries,
                    offset_count,
                    0,
                    Codec::For,
                    Bounds::singleton(0),
                )?
            };
            let payload = descriptor
                .payload
                .iter()
                .map(|payload| unsafe {
                    pack_payload(pool, payload, std::ptr::null(), 0, row_count.clone())
                })
                .collect::<Result<Vec<_>>>()?;
            if let Some(selected) = selected {
                release(stream, selected)?;
            }
            return finish(offsets, payload, first.domain_size, rows, workspace);
        }
        let mut keys = temporary::<u32>(stream, rows)?;
        let mut order = temporary::<u32>(stream, rows)?;
        let source = selected
            .as_ref()
            .map_or(std::ptr::null(), |selected| pointer(selected) as *const u32);
        if descriptor.key.len() > 1 {
            let plain = descriptor
                .key
                .iter()
                .map(|key| {
                    let out = temporary::<u32>(stream, rows)?;
                    module.sequence_decode_column(
                        stream,
                        &module.prepare_sequence_decode_column(config(rows))?,
                        key.column.view(),
                        source,
                        rows as u32,
                        pointer(&out),
                    )?;
                    Ok(out)
                })
                .collect::<Result<Vec<_>>>()?;
            let widths = descriptor
                .key
                .iter()
                .map(|key| key_bits(key.domain_size))
                .collect::<Vec<_>>();
            let mut permutation = temporary::<u32>(stream, rows)?;
            sort_columns_lexicographic(
                &exec,
                &plain.iter().collect::<Vec<_>>(),
                &widths,
                &mut permutation,
                None,
            )?;
            let base_order = match &selected {
                Some(selected) => {
                    let mut mapped = temporary::<u32>(stream, rows)?;
                    gather(
                        &exec,
                        selected.view(),
                        permutation.view(),
                        mapped.view_mut(),
                    )?;
                    release(stream, permutation)?;
                    mapped
                }
                None => permutation,
            };
            module.keys_and_rows(
                stream,
                &module.prepare_keys_and_rows(config(rows))?,
                first.column.view(),
                pointer(&base_order),
                rows as u32,
                pointer(&keys),
                pointer(&order),
            )?;
            for buffer in plain.into_iter().chain([base_order]) {
                release(stream, buffer)?;
            }
        } else {
            module.keys_and_rows(
                stream,
                &module.prepare_keys_and_rows(config(rows))?,
                first.column.view(),
                source,
                rows as u32,
                pointer(&keys),
                pointer(&order),
            )?;
            let bits = key_bits(first.domain_size);
            if rows > 1 && bits > 0 {
                let mut alternate_keys = temporary::<u32>(stream, rows)?;
                let mut alternate_order = temporary::<u32>(stream, rows)?;
                let mut pairs = DoubleBuffer::paired(
                    (&mut keys, &mut order),
                    (&mut alternate_keys, &mut alternate_order),
                )
                .ok_or("radix double buffer extents differ")?;
                with_range_radix_workspace(pool, &launch, rows, |workspace| {
                    rccl_cub::device_radix_sort_pairs_buffer(
                        &launch,
                        &mut pairs,
                        0..bits,
                        workspace,
                    )?;
                    Ok(())
                })?;
                let swapped = !pairs.current_is_first();
                drop(pairs);
                if swapped {
                    std::mem::swap(&mut keys, &mut alternate_keys);
                    std::mem::swap(&mut order, &mut alternate_order);
                }
                release(stream, alternate_keys)?;
                release(stream, alternate_order)?;
            }
        }
        let mut offsets = temporary::<u64>(stream, entries)?;
        lower_bound(
            &exec,
            keys.view(),
            arithmetic_range(0_u32, 1, entries),
            offsets.view_mut(),
        )?;
        let payload = descriptor
            .payload
            .iter()
            .map(|payload| unsafe {
                pack_payload(pool, payload, pointer(&order), rows, row_count.clone())
            })
            .collect::<Result<Vec<_>>>()?;
        let packed_offsets = unsafe {
            EncodedColumn::from_device(
                pool.clone(),
                pointer(&offsets) as *const i64,
                entries,
                offset_count,
                0,
                Codec::For,
                Bounds::Inclusive {
                    min: 0,
                    max: rows as i64,
                },
            )?
        };
        for buffer in [keys, order].into_iter().chain(selected) {
            release(stream, buffer)?;
        }
        release(stream, offsets)?;
        finish(packed_offsets, payload, first.domain_size, rows, workspace)
    }
}

fn finish(
    offsets: EncodedColumn,
    payload: Vec<EncodedColumn>,
    domain: usize,
    len: usize,
    workspace: &mut RangeWorkspace,
) -> Result<RangeProvider> {
    workspace.host_views.clear();
    workspace
        .host_views
        .extend(payload.iter().map(|column| unsafe { column.view() }));
    let views = &workspace.host_views;
    if Arc::strong_count(&workspace.payload_views) != 1 {
        workspace.payload_views = Arc::new(Buffer::from_host_async(offsets.pool().clone(), views)?);
    } else {
        unsafe { workspace.payload_views.write_async(views)? };
    }
    let payload_views = workspace.payload_views.clone();
    Ok(RangeProvider {
        offsets,
        payload,
        payload_views,
        domain: domain as u32,
        len: len as u32,
    })
}
unsafe fn pack_payload(
    pool: &Arc<Pool>,
    payload: &PayloadDescriptor,
    order: *const u32,
    rows: usize,
    count: Arc<Buffer<u64>>,
) -> Result<EncodedColumn> {
    let bounds = payload.column.bounds();
    match payload.kind {
        PayloadKind::U32 => unsafe {
            EncodedColumn::from_gather_source::<_, u32>(
                pool.clone(),
                payload.column.view(),
                order,
                rows,
                count,
                0,
                bounds,
            )
        },
        PayloadKind::I32 => unsafe {
            EncodedColumn::from_gather_source::<_, i32>(
                pool.clone(),
                payload.column.view(),
                order,
                rows,
                count,
                0,
                bounds,
            )
        },
        PayloadKind::I64 => unsafe {
            EncodedColumn::from_gather_source::<_, i64>(
                pool.clone(),
                payload.column.view(),
                order,
                rows,
                count,
                0,
                bounds,
            )
        },
    }
}
fn config(rows: usize) -> LaunchConfig1D {
    LaunchConfig1D::new(rows.div_ceil(256).clamp(1, 568 * 4) as u32, 256, 0)
}
fn key_bits(domain: usize) -> u32 {
    if domain <= 1 {
        0
    } else {
        usize::BITS - (domain - 1).leading_zeros()
    }
}
struct TemporaryCache<T: DeviceCopy> {
    stream: Arc<CudaStream>,
    buffers: Vec<DeviceBuffer<T>>,
}
impl<T: DeviceCopy> Drop for TemporaryCache<T> {
    fn drop(&mut self) {
        for buffer in self.buffers.drain(..) {
            self.stream
                .context()
                .record_err(unsafe { buffer.drop_async(&self.stream) });
        }
    }
}
thread_local! {
    static TEMP_U32: RefCell<Option<TemporaryCache<u32>>> = const { RefCell::new(None) };
    static TEMP_U64: RefCell<Option<TemporaryCache<u64>>> = const { RefCell::new(None) };
}
trait TemporaryElement: DeviceCopy + 'static {
    fn cache() -> &'static std::thread::LocalKey<RefCell<Option<TemporaryCache<Self>>>>;
}
impl TemporaryElement for u32 {
    fn cache() -> &'static std::thread::LocalKey<RefCell<Option<TemporaryCache<Self>>>> {
        &TEMP_U32
    }
}
impl TemporaryElement for u64 {
    fn cache() -> &'static std::thread::LocalKey<RefCell<Option<TemporaryCache<Self>>>> {
        &TEMP_U64
    }
}
fn temporary<T: TemporaryElement>(stream: &Arc<CudaStream>, len: usize) -> Result<DeviceBuffer<T>> {
    T::cache().with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache
            .as_ref()
            .is_none_or(|cache| !Arc::ptr_eq(&cache.stream, stream))
        {
            *cache = Some(TemporaryCache {
                stream: stream.clone(),
                buffers: Vec::new(),
            });
            stream.context().bind_to_thread()?;
        }
        let buffers = &mut cache.as_mut().unwrap().buffers;
        if let Some(index) = buffers.iter().position(|buffer| buffer.len() == len) {
            return Ok(buffers.swap_remove(index));
        }
        Ok(unsafe { DeviceBuffer::uninitialized_async(stream, len)? })
    })
}
fn pointer<T: DeviceCopy>(buffer: &DeviceBuffer<T>) -> *mut T {
    buffer.cu_deviceptr() as *mut T
}
fn release<T: TemporaryElement>(stream: &CudaStream, buffer: DeviceBuffer<T>) -> Result<()> {
    T::cache().with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(cache) = cache.as_mut() {
            if cache.stream.cu_stream() == stream.cu_stream()
                && Arc::ptr_eq(cache.stream.context(), stream.context())
            {
                cache.buffers.push(buffer);
                return Ok(());
            }
        }
        unsafe { buffer.drop_async(stream)? };
        Ok(())
    })
}
