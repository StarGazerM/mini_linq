// Resident Structures execution, compiled only for the isolated legacy workspace.
pub mod datalog;
mod device;
mod fused_retry;
mod host_set;
mod kernels;
mod leveled;
mod sets;

use cuda_core::{
    CudaContext, CudaStream, DeviceCopy, IntoResult, LaunchConfig1D, PinnedHostBuffer,
};
pub use device::*;
pub use fused_retry::{FusedRetryReport, FusedRetryScope, fused_retry_scope};
pub use gpu_blocked_run as blocked_run;
use gpu_blocked_run::{Buffer as Scratch, Run};
use gpu_device_columns::{OutputCount, Table};
pub use gpu_tiles as tiles;
pub use leveled::*;
use rccl_memory::ContiguousDeviceRange;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fmt,
    marker::PhantomData,
    sync::{Arc, Weak},
    time::Instant,
};

/// Materialize candidates before exact canonicalization (standalone default).
pub struct Staged;
/// Exact prefix compaction at resumable suspension boundaries.
pub struct Compacting;
/// Exact head-tuple admission before materialization, including bounded retry.
pub struct Fused;
pub struct DeviceSet<T, C = Plain, P = Staged>(PhantomData<(T, C, P)>);
/// A stable lexicographic run over the recorded key levels, flattened in order.
///
/// Every active row is retained, with original input-row order breaking ties
/// between equal complete keys. `PREFIX` is the original schema index of the
/// leading directory key, not a key-level depth. Device columns keep their
/// original schema indexes. Within each equal-key
/// prefix, the next key is sorted, enabling bounded device binary searches.
/// Construction uses integral key bounds and never reads rows or counts to host.
pub struct PrefixRun<K, V, C, const PREFIX: usize>(PhantomData<(K, V, C)>);
pub struct HashedRun<K, V>(PhantomData<(K, V)>);

#[derive(Debug)]
pub struct RuntimeError {
    pub code: &'static str,
    pub detail: String,
}
impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.detail)
    }
}
impl std::error::Error for RuntimeError {}
fn failure(code: &'static str, detail: impl Into<String>) -> Error {
    Box::new(RuntimeError {
        code,
        detail: detail.into(),
    })
}

thread_local! {
    static HOST_WAITS: Cell<usize> = const { Cell::new(0) };
    static CONTROL_READBACKS: Cell<usize> = const { Cell::new(0) };
    static RESUME_READBACKS: Cell<usize> = const { Cell::new(0) };
    static COMPACTION_READBACKS: Cell<usize> = const { Cell::new(0) };
    static FUSED_RETRY_READBACKS: Cell<usize> = const { Cell::new(0) };
    static RETRY_READBACKS: Cell<usize> = const { Cell::new(0) };
}
/// Explicit runtime stream waits, including checked control readbacks.
/// Excludes uploads, user downloads, and allocator-internal synchronization.
pub fn host_wait_count() -> usize {
    HOST_WAITS.with(Cell::get)
}
pub fn control_readback_count() -> usize {
    CONTROL_READBACKS.with(Cell::get)
}
/// Suspension checks required before dependent operators can consume an output.
pub fn resume_readback_count() -> usize {
    RESUME_READBACKS.with(Cell::get)
}
/// Distinct-cardinality checks at recorded Compacting suspension boundaries.
pub fn compaction_readback_count() -> usize {
    COMPACTION_READBACKS.with(Cell::get)
}
/// Checked metadata readbacks for Fused bounded attempts, including replays.
/// These are also included in host_wait_count and control_readback_count.
pub fn fused_retry_readback_count() -> usize {
    FUSED_RETRY_READBACKS.with(Cell::get)
}
/// Checked summaries from direct bounded retry attempts, including replays.
pub fn retry_readback_count() -> usize {
    RETRY_READBACKS.with(Cell::get)
}
pub fn reset_host_wait_count() {
    HOST_WAITS.with(|count| count.set(0));
    CONTROL_READBACKS.with(|count| count.set(0));
    RESUME_READBACKS.with(|count| count.set(0));
    COMPACTION_READBACKS.with(|count| count.set(0));
    FUSED_RETRY_READBACKS.with(|count| count.set(0));
    RETRY_READBACKS.with(|count| count.set(0));
}
fn record_host_wait(readback: bool) {
    HOST_WAITS.with(|count| count.set(count.get().saturating_add(1)));
    if readback {
        CONTROL_READBACKS.with(|count| count.set(count.get().saturating_add(1)));
    }
}

trait TableViewAccess {
    fn view(&self) -> TableView;
}
impl TableViewAccess for Table {
    fn view(&self) -> TableView {
        TableView {
            columns: self.views.ptr(),
            len: self.capacity as u64,
            count: self.metadata.ptr().cast(),
            error: unsafe { self.metadata.ptr().cast::<u8>().add(8).cast() },
        }
    }
}

/// Dispatch copies references, never borrows a generated temporary reference.
/// Resident handles retain their owners; Vec/slice lifetimes track only data.
pub trait HostColumns<'data>: Copy {
    type Row: RowCodec<'data>;
    fn pool(self) -> Option<Arc<Pool>> {
        None
    }
    fn seed_dictionary(self, _: &mut Dictionary<'data>) -> Result<()> {
        Ok(())
    }
    fn bind(
        self,
        execution: &mut Execution<'_, 'data>,
        schema: &'static str,
        width: usize,
    ) -> Result<TableId>;
}
impl<'data, T: RowCodec<'data> + 'data> HostColumns<'data> for &'data [T] {
    type Row = T;
    fn bind(
        self,
        execution: &mut Execution<'_, 'data>,
        schema: &'static str,
        width: usize,
    ) -> Result<TableId> {
        execution.encode_slice(self, schema, width, |row, dict, values| {
            row.encode(dict, values)
        })
    }
}
impl<'data, T: RowCodec<'data> + 'data> HostColumns<'data> for &'data Vec<T> {
    type Row = T;
    fn bind(
        self,
        execution: &mut Execution<'_, 'data>,
        schema: &'static str,
        width: usize,
    ) -> Result<TableId> {
        self.as_slice().bind(execution, schema, width)
    }
}
impl<'data, T: RowCodec<'data> + 'data, const N: usize> HostColumns<'data> for &'data [T; N] {
    type Row = T;
    fn bind(
        self,
        execution: &mut Execution<'_, 'data>,
        schema: &'static str,
        width: usize,
    ) -> Result<TableId> {
        self.as_slice().bind(execution, schema, width)
    }
}
impl<'data, 'source: 'data, T: RowCodec<'data>, C> HostColumns<'data>
    for &DeviceColumns<'source, T, C>
{
    type Row = T;
    fn pool(self) -> Option<Arc<Pool>> {
        Some(DeviceColumns::pool(self).clone())
    }
    fn seed_dictionary(self, dictionary: &mut Dictionary<'data>) -> Result<()> {
        dictionary.merge(self.dictionary())
    }
    fn bind(
        self,
        execution: &mut Execution<'_, 'data>,
        _: &'static str,
        width: usize,
    ) -> Result<TableId> {
        if width != self.storage().columns.len() {
            return Err(failure(
                "CQ_GPU_SCHEMA",
                "resident column count differs from recorded schema",
            ));
        }
        execution.bind_columns(self)
    }
}
impl<'data, H: HostColumns<'data>> HostColumns<'data> for &H {
    type Row = H::Row;
    fn pool(self) -> Option<Arc<Pool>> {
        (*self).pool()
    }
    fn seed_dictionary(self, dictionary: &mut Dictionary<'data>) -> Result<()> {
        (*self).seed_dictionary(dictionary)
    }
    fn bind(
        self,
        execution: &mut Execution<'_, 'data>,
        schema: &'static str,
        width: usize,
    ) -> Result<TableId> {
        (*self).bind(execution, schema, width)
    }
}

/// A schema-slot marker in a recorded frozen run's nested tuple Layout.
/// Structures uses these markers to retain the complete run certificate even
/// when a particular host read consumes only an earlier key prefix.
pub struct FrozenKey<const SLOT: usize>;

/// Immutable indexes retained across query holes on one runtime's pool stream.
///
/// Layouts use original column slots and preserve key-level boundaries. All
/// keys are sorted lexicographically (columns within each level use ascending
/// schema slots), with input-row order breaking complete-key ties. Every key prefix
/// exposes an exact contiguous, length-known range. Construction enqueues the
/// existing blocked-run builders without reading device rows or counts.
/// Keep this handle alive until all consumers on the pool stream are enqueued.
pub struct FrozenDeviceIndex<'runtime, 'data, Row, C = Plain, Layout = ()> {
    execution: Execution<'runtime, 'data>,
    table: TableId,
    layouts: Vec<Vec<Vec<usize>>>,
    runs: Vec<RunId>,
    marker: PhantomData<(Row, C, Layout)>,
}

fn validate_frozen_layout(levels: &[&[usize]], width: usize) -> Result<()> {
    if levels.is_empty()
        || levels.iter().any(|level| level.is_empty())
        || levels
            .iter()
            .flat_map(|level| level.iter())
            .any(|&slot| slot >= width)
    {
        return Err(failure(
            "CQ_GPU_FROZEN_LEVELS",
            "expected nonempty in-schema key levels",
        ));
    }
    let mut seen = Vec::new();
    for &slot in levels.iter().flat_map(|level| level.iter()) {
        if seen.contains(&slot) {
            return Err(failure("CQ_GPU_FROZEN_LEVELS", "duplicate key column"));
        }
        seen.push(slot);
    }
    Ok(())
}

fn frozen_layout_position(layouts: &[Vec<Vec<usize>>], levels: &[&[usize]]) -> Result<usize> {
    layouts
        .iter()
        .position(|existing| {
            existing.len() == levels.len()
                && existing.iter().zip(levels).all(|(a, b)| a.as_slice() == *b)
        })
        .ok_or_else(|| {
            failure(
                "CQ_GPU_FROZEN_LAYOUT_MISSING",
                format!("resident index has no exact key-level layout {levels:?}"),
            )
        })
}

impl<'runtime, 'data, Row: RowCodec<'data>, C> FrozenDeviceIndex<'runtime, 'data, Row, C> {
    pub fn new(
        runtime: &'runtime Runtime,
        columns: &DeviceColumns<'data, Row, C>,
        layouts: &[&[&[usize]]],
    ) -> Result<Self> {
        // Reject malformed certificates before enqueuing any builders.
        for levels in layouts {
            validate_frozen_layout(levels, Row::WIDTH)?;
        }
        let mut execution = runtime.execution();
        let table = execution.bind_columns(columns)?;
        let mut recorded = Vec::<Vec<Vec<usize>>>::new();
        let mut runs = Vec::new();
        for &levels in layouts {
            if recorded.iter().any(|existing| {
                existing.len() == levels.len()
                    && existing.iter().zip(levels).all(|(a, b)| a.as_slice() == *b)
            }) {
                continue;
            }
            let run = if levels.len() == 1 && levels[0].len() == 1 {
                execution.prepare_run(table, levels[0][0])?
            } else {
                let keys = levels
                    .iter()
                    .flat_map(|level| level.iter().copied())
                    .collect::<Vec<_>>();
                execution.prepare_prefix_run(table, &keys)?
            };
            recorded.push(levels.iter().map(|level| level.to_vec()).collect());
            runs.push(run);
        }
        Ok(Self {
            execution,
            table,
            layouts: recorded,
            runs,
            marker: PhantomData,
        })
    }
}

impl<Row, C, Layout> FrozenDeviceIndex<'_, '_, Row, C, Layout> {
    /// Borrow exactly the recorded layout; never rebuild or substitute a prefix.
    /// The returned pointers are valid only while this index remains alive.
    pub fn resident_run(&self, levels: &[&[usize]]) -> Result<RunView> {
        let position = frozen_layout_position(&self.layouts, levels)?;
        self.execution.run_view(self.runs[position])
    }
}

impl<'data, 'source: 'data, Row: RowCodec<'data>, C, Layout> HostColumns<'data>
    for &FrozenDeviceIndex<'_, 'source, Row, C, Layout>
{
    type Row = Row;
    fn pool(self) -> Option<Arc<Pool>> {
        Some(self.execution.runtime.pool.clone())
    }
    fn seed_dictionary(self, dictionary: &mut Dictionary<'data>) -> Result<()> {
        dictionary.merge(self.execution.dictionary())
    }
    fn bind(
        self,
        execution: &mut Execution<'_, 'data>,
        _: &'static str,
        width: usize,
    ) -> Result<TableId> {
        if !Arc::ptr_eq(&execution.runtime.pool, &self.execution.runtime.pool) {
            return Err(failure(
                "CQ_GPU_CONTEXT",
                "resident index must share the query pool and stream",
            ));
        }
        let table = self.execution.table(self.table)?;
        if table.columns.len() != width {
            return Err(failure(
                "CQ_GPU_SCHEMA",
                "resident index width differs from recorded schema",
            ));
        }
        self.seed_dictionary(execution.dictionary_mut())?;
        Ok(execution.add_table(table.clone()))
    }
}

/// Exact projected-key membership built once and retained for a stratum.
///
/// The carrier owns the source table, dictionary, dependency statuses and the
/// exact membership allocation through its resident execution. Query holes
/// borrow views on the same pool stream; construction performs no readback.
pub struct FrozenDeviceMembership<'runtime, 'data, Row, C = Plain> {
    index: FrozenDeviceIndex<'runtime, 'data, Row, C>,
    membership: ExactMembershipId,
}

impl<'runtime, 'data, Row: RowCodec<'data>, C> FrozenDeviceMembership<'runtime, 'data, Row, C> {
    /// Index every column of an already projected key relation.
    pub fn new(runtime: &'runtime Runtime, columns: &DeviceColumns<'data, Row, C>) -> Result<Self> {
        let keys = (0..Row::WIDTH).collect::<Vec<_>>();
        Self::build(runtime, columns, &keys)
    }

    fn build(
        runtime: &'runtime Runtime,
        columns: &DeviceColumns<'data, Row, C>,
        keys: &[usize],
    ) -> Result<Self> {
        let mut index = FrozenDeviceIndex::new(runtime, columns, &[])?;
        let membership = index
            .execution
            .prepare_exact_membership(index.table, keys)?;
        Ok(Self { index, membership })
    }
}

impl<'runtime, 'data, C> FrozenDeviceMembership<'runtime, 'data, (u8,), C> {
    /// Build nullary membership from the bridge's canonical witness rows.
    pub fn new_exists(
        runtime: &'runtime Runtime,
        columns: &DeviceColumns<'data, (u8,), C>,
    ) -> Result<Self> {
        Self::build(runtime, columns, &[])
    }
}

impl<Row, C> FrozenDeviceMembership<'_, '_, Row, C> {
    /// Borrow an exact membership view with the recorded logical key width.
    /// Unlike witness-row width, the logical width of nullary membership is zero.
    pub fn resident_membership(&self, expected_width: usize) -> Result<ExactMembershipView> {
        let view = self
            .index
            .execution
            .exact_membership_view(self.membership)?;
        if view.width != expected_width {
            return Err(failure(
                "CQ_GPU_FROZEN_MEMBERSHIP_SCHEMA",
                format!(
                    "recorded key width {expected_width} differs from resident width {}",
                    view.width
                ),
            ));
        }
        Ok(view)
    }
}

impl<'data, 'source: 'data, Row: RowCodec<'data>, C> HostColumns<'data>
    for &FrozenDeviceMembership<'_, 'source, Row, C>
{
    type Row = Row;
    fn pool(self) -> Option<Arc<Pool>> {
        HostColumns::pool(&self.index)
    }
    fn seed_dictionary(self, dictionary: &mut Dictionary<'data>) -> Result<()> {
        HostColumns::seed_dictionary(&self.index, dictionary)
    }
    fn bind(
        self,
        execution: &mut Execution<'_, 'data>,
        schema: &'static str,
        width: usize,
    ) -> Result<TableId> {
        HostColumns::bind(&self.index, execution, schema, width)
    }
}

pub struct Runtime {
    kernels: kernels::runtime_kernels::LoadedModule,
    pool: Arc<Pool>,
    dedup_scratch: RefCell<DedupScratch>,
    descriptor_staging: RefCell<DescriptorStaging>,
    descriptor_cache: RefCell<HashMap<Vec<usize>, CachedViews>>,
    pending_timings: RefCell<Vec<PendingTiming>>,
    fused_retries: RefCell<fused_retry::Records>,
    materialization_history: RefCell<fused_retry::MaterializationHistory>,
    hashed_run_bucket_limit: usize,
}
impl Runtime {
    pub fn new() -> Result<Self> {
        Self::with_pool(0, 0, 0)
    }
    pub fn with_pool(device: usize, initial_bytes: usize, maximum_bytes: usize) -> Result<Self> {
        Self::from_pool(Pool::new(device, initial_bytes, maximum_bytes)?)
    }
    pub fn from_pool(pool: Arc<Pool>) -> Result<Self> {
        let kernels = unsafe { kernels::runtime_kernels::load(pool.context())? };
        gpu_blocked_run::prepare_kernels(pool.context())?;
        nvtx::name_stream(pool.stream().cu_stream() as *mut std::ffi::c_void);
        Ok(Self {
            kernels,
            pool,
            dedup_scratch: RefCell::new(DedupScratch::default()),
            descriptor_staging: RefCell::new(DescriptorStaging::default()),
            descriptor_cache: RefCell::new(HashMap::new()),
            pending_timings: RefCell::new(Vec::new()),
            fused_retries: RefCell::new(fused_retry::Records::default()),
            materialization_history: RefCell::new(fused_retry::MaterializationHistory::default()),
            hashed_run_bucket_limit: 1 << 24,
        })
    }
    /// Limit hashed-run directories (also useful for collision stress tests).
    /// A smaller directory changes performance, never equality semantics.
    pub fn with_hashed_run_bucket_limit(mut self, buckets: usize) -> Result<Self> {
        if !buckets.is_power_of_two() || buckets as u128 > gpu_blocked_run::MAX_DOMAIN_KEYS as u128
        {
            return Err(failure(
                "CQ_GPU_HASH_BUCKETS",
                "bucket limit must be a power of two within the run domain",
            ));
        }
        self.hashed_run_bucket_limit = buckets;
        Ok(self)
    }
    pub fn context(&self) -> &Arc<CudaContext> {
        self.pool.context()
    }
    pub fn stream(&self) -> &CudaStream {
        self.pool.stream()
    }
    pub fn pool(&self) -> &Arc<Pool> {
        &self.pool
    }
    /// Snapshot cumulative fused reports, sorted by `(hole, site, tuple width)`.
    /// Does not synchronize the device; completed attempts are accounted at
    /// their mandatory metadata readback. Estimates survive Execution drops.
    pub fn fused_retry_reports(&self) -> Vec<FusedRetryReport> {
        self.fused_retries.borrow().reports()
    }
    fn retry_budget(&self) -> Result<fused_retry::Budget> {
        self.context().bind_to_thread()?;
        let mut free = 0usize;
        let mut total = 0usize;
        unsafe {
            cuda_core::sys::cuMemGetInfo_v2(&mut free, &mut total).result()?;
        }
        let pool = self.pool.resource().upstream();
        let stats = pool.stats();
        // Same-stream frees are immediately reusable by this custom pool.
        // Do not flush its cache or synchronize merely to inspect a budget.
        Ok(fused_retry::Budget::new(
            free,
            total,
            stats.pool_size,
            stats.allocated_bytes,
            stats.free_bytes,
            pool.maximum_size(),
        ))
    }
    pub fn execution<'runtime, 'data>(&'runtime self) -> Execution<'runtime, 'data> {
        LAST_TIMINGS.with(|last| last.set(None));
        Execution {
            runtime: self,
            dictionary: Dictionary::default(),
            tables: Vec::new(),
            runs: Vec::new(),
            uploads: HashMap::new(),
            run_cache: HashMap::new(),
            memberships: Vec::new(),
            membership_cache: HashMap::new(),
            exact_memberships: Vec::new(),
            timings: Timings::default(),
            pending_timings: Vec::new(),
            started: Instant::now(),
        }
    }
    pub fn upload_rows<'data, T: RowCodec<'data>>(
        &self,
        rows: &'data [T],
        dictionary: &mut Dictionary<'data>,
        codecs: &[Codec],
    ) -> Result<DeviceColumns<'data, T>> {
        check_rows(rows.len())?;
        if codecs.len() != T::WIDTH {
            return Err(failure("CQ_GPU_SCHEMA", "one codec is required per column"));
        }
        let mut host: Vec<Vec<i64>> = (0..T::WIDTH)
            .map(|_| Vec::with_capacity(rows.len()))
            .collect();
        let mut values = vec![0; T::WIDTH];
        for row in rows {
            row.encode(dictionary, &mut values)?;
            for (column, &value) in host.iter_mut().zip(&values) {
                column.push(value);
            }
        }
        self.upload_encoded_rows(rows.len(), host, dictionary, codecs)
    }
    /// Loader for owned Rust values. No source tuple/string borrow escapes.
    pub fn upload_owned_rows<T: OwnedRowCodec>(
        &self,
        rows: &[T],
        dictionary: &mut Dictionary<'static>,
        codecs: &[Codec],
    ) -> Result<DeviceColumns<'static, T>> {
        DeviceColumns::from_rows(self.pool.clone(), rows, dictionary, codecs)
    }
    fn upload_encoded_rows<'data, T>(
        &self,
        rows: usize,
        host: Vec<Vec<i64>>,
        dictionary: &Dictionary<'data>,
        codecs: &[Codec],
    ) -> Result<DeviceColumns<'data, T>> {
        let columns = host
            .iter()
            .zip(codecs)
            .map(|(values, &codec)| {
                EncodedColumn::from_host(self.pool.clone(), values, codec).map(Arc::new)
            })
            .collect::<Result<Vec<_>>>()?;
        let bounds = columns.iter().map(|column| column.bounds()).collect();
        let metadata = self.metadata(rows)?;
        let views = self.column_views(&columns)?;
        self.stream().synchronize()?;
        self.reclaim_descriptors();
        // The loader has initialized every column and its matching metadata on
        // this pool's stream; the table retains every allocation owner.
        Ok(unsafe {
            DeviceColumns::from_parts(
                Table {
                    columns,
                    views,
                    metadata,
                    capacity: rows,
                    bounds,
                    statuses: Vec::new(),
                },
                Arc::new(dictionary.clone()),
                self.pool.clone(),
            )
        })
    }
    fn metadata(&self, rows: usize) -> Result<Arc<Buffer<OutputCount>>> {
        Ok(Arc::new(self.upload_descriptors(std::iter::once(
            OutputCount {
                rows: rows as u64,
                error: 0,
                padding: 0,
            },
        ))?))
    }
    fn column_views(&self, columns: &[Arc<EncodedColumn>]) -> Result<Arc<Buffer<ColumnView>>> {
        let key: Vec<_> = columns
            .iter()
            .map(|column| Arc::as_ptr(column) as usize)
            .collect();
        let mut cache = self.descriptor_cache.borrow_mut();
        if let Some(entry) = cache.get(&key) {
            if entry
                .columns
                .iter()
                .all(|column| column.strong_count() != 0)
            {
                return Ok(entry.views.clone());
            }
        }
        let views = Arc::new(
            self.upload_descriptors(columns.iter().map(|column| unsafe { column.view() }))?,
        );
        cache.insert(
            key,
            CachedViews {
                columns: columns.iter().map(Arc::downgrade).collect(),
                views: views.clone(),
            },
        );
        Ok(views)
    }
    /// Copy the complete host-known table once. Pinned source storage stays
    /// immutable until an existing checked readback completes this stream.
    fn upload_descriptors<T: DeviceCopy>(
        &self,
        values: impl ExactSizeIterator<Item = T>,
    ) -> Result<Buffer<T>> {
        let buffer = unsafe { Buffer::uninitialized(self.pool.clone(), values.len())? };
        if buffer.is_empty() {
            return Ok(buffer);
        }
        self.descriptor_staging
            .borrow_mut()
            .upload(self, &buffer, values)?;
        Ok(buffer)
    }
    /// Call only after a successful synchronization of this runtime's stream.
    fn reclaim_descriptors(&self) {
        self.descriptor_staging.borrow_mut().rewind();
        self.descriptor_cache.borrow_mut().retain(|_, entry| {
            entry
                .columns
                .iter()
                .all(|column| column.strong_count() != 0)
        });
    }
}

struct CachedViews {
    columns: Vec<Weak<EncodedColumn>>,
    views: Arc<Buffer<ColumnView>>,
}

#[derive(Default)]
struct DescriptorStaging {
    chunks: Vec<PinnedHostBuffer<u64>>,
    chunk: usize,
    used: usize,
}
impl DescriptorStaging {
    fn upload<T: DeviceCopy>(
        &mut self,
        runtime: &Runtime,
        destination: &Buffer<T>,
        values: impl ExactSizeIterator<Item = T>,
    ) -> Result<()> {
        assert!(std::mem::align_of::<T>() <= std::mem::align_of::<u64>());
        let words = destination.bytes().div_ceil(std::mem::size_of::<u64>());
        if self
            .chunks
            .get(self.chunk)
            .is_some_and(|chunk| self.used + words > chunk.len())
        {
            self.chunk += 1;
            self.used = 0;
        }
        if self.chunk == self.chunks.len() {
            self.chunks.push(PinnedHostBuffer::zeroed(
                runtime.context(),
                words.max(8192),
            )?);
        } else if self.chunks[self.chunk].len() < words {
            // This chunk has not been used since the last completed readback.
            self.chunks[self.chunk] = PinnedHostBuffer::zeroed(runtime.context(), words.max(8192))?;
        }
        let source = unsafe {
            self.chunks[self.chunk]
                .as_mut_slice()
                .as_mut_ptr()
                .add(self.used)
        };
        for (index, value) in values.enumerate() {
            unsafe { source.cast::<T>().add(index).write(value) };
        }
        // Reserve before submission, including failure paths: a failed copy
        // must never let a subsequent call overwrite a possibly live source.
        self.used += words;
        unsafe {
            cuda_core::memory::memcpy_htod_async(
                destination.cu_deviceptr(),
                source.cast::<T>(),
                destination.bytes(),
                runtime.stream().cu_stream(),
            )?;
        }
        Ok(())
    }
    fn pending(&self) -> bool {
        // Advancing to another chunk can fail before its first upload. Earlier
        // chunks still contain live DMA sources even when `used` is now zero.
        self.chunk != 0 || self.used != 0
    }
    fn rewind(&mut self) {
        self.chunk = 0;
        self.used = 0;
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        // Error/early-exit paths may not reach another checked readback.
        // Keep pinned upload sources alive until all outstanding DMA completes.
        if self.descriptor_staging.get_mut().pending() {
            self.context().record_err(self.stream().synchronize());
        }
    }
}

impl Runtime {
    // Called only after a checked readback or explicit finish has completed the
    // stream. Query holes and publication operations only enqueue their events.
    fn collect_timings(&self) -> Result<Timings> {
        let mut timings = Timings::default();
        for timing in self.pending_timings.borrow_mut().drain(..) {
            let elapsed = timing.start.elapsed_ms(&timing.end)? as f64;
            match timing.stage {
                TimingStage::Upload => timings.upload_ms += elapsed,
                TimingStage::Build => timings.build_ms += elapsed,
                TimingStage::Kernel => timings.kernel_ms += elapsed,
                TimingStage::Dedup => timings.dedup_ms += elapsed,
            }
        }
        timings.total_ms = timings.build_ms + timings.kernel_ms + timings.dedup_ms;
        LAST_TIMINGS.with(|last| last.set(Some(timings)));
        Ok(timings)
    }
}
thread_local! { static SHARED_RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) }; }
thread_local! { static RESIDENT_RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) }; }
pub fn with_runtime<T>(execute: impl FnOnce(&Runtime) -> Result<T>) -> Result<T> {
    SHARED_RUNTIME.with(|runtime| {
        if runtime.borrow().is_none() {
            *runtime.borrow_mut() = Some(Runtime::new()?);
        }
        execute(runtime.borrow().as_ref().unwrap())
    })
}
pub fn with_runtime_for<T>(
    pool: Option<Arc<Pool>>,
    execute: impl FnOnce(&Runtime) -> Result<T>,
) -> Result<T> {
    if let Some(pool) = pool {
        RESIDENT_RUNTIME.with(|cached| {
            if cached
                .borrow()
                .as_ref()
                .is_none_or(|runtime| !Arc::ptr_eq(&runtime.pool, &pool))
            {
                *cached.borrow_mut() = Some(Runtime::from_pool(pool)?);
            }
            execute(cached.borrow().as_ref().unwrap())
        })
    } else {
        with_runtime(execute)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct TableId(usize);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct RunId(usize);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct MembershipId(usize);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ExactMembershipId(usize);
struct ExactMembership {
    packed: Buffer<u64>,
    slots: Buffer<u64>,
    keys: TableId,
    width: usize,
}
#[derive(Eq, PartialEq, Hash)]
struct SliceKey {
    pointer: usize,
    len: usize,
    rust_type: &'static str,
    schema: &'static str,
    width: usize,
}
struct BuiltRun<'runtime> {
    run: Run<'runtime>,
    pointers: Buffer<u64>,
    len: u64,
    bucket_mask: u64,
}
struct Membership {
    bits: Buffer<u64>,
    bounds: Buffer<u64>,
}
#[derive(Default)]
struct DedupScratch {
    packed: Option<Buffer<u64>>,
    slots: Option<Buffer<u64>>,
}
impl DedupScratch {
    fn reserve(&mut self, pool: &Arc<Pool>, packed: usize, slots: usize) -> Result<()> {
        for (buffer, required) in [(&mut self.packed, packed), (&mut self.slots, slots)] {
            if buffer.as_ref().is_none_or(|buffer| buffer.len() < required) {
                // Kernels initialize the live prefix before it is consumed.
                *buffer = Some(unsafe { Buffer::uninitialized(pool.clone(), required)? });
            }
        }
        Ok(())
    }
}
enum TimingStage {
    Upload,
    Build,
    Kernel,
    Dedup,
}
struct PendingTiming {
    stage: TimingStage,
    start: TimingEvent,
    end: TimingEvent,
}
struct TimingEvent(Option<cuda_core::CudaEvent>);
impl TimingEvent {
    fn record(stream: &CudaStream) -> std::result::Result<Self, cuda_core::DriverError> {
        #[cfg(feature = "timings")]
        return stream
            .record_event(Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT))
            .map(|event| Self(Some(event)));
        #[cfg(not(feature = "timings"))]
        {
            let _ = stream;
            Ok(Self(None))
        }
    }
    fn elapsed_ms(&self, end: &Self) -> std::result::Result<f32, cuda_core::DriverError> {
        match (&self.0, &end.0) {
            (Some(start), Some(end)) => start.elapsed_ms(end),
            _ => Ok(0.0),
        }
    }
}

struct Output {
    columns: Vec<Buffer<i64>>,
    pointers: Buffer<u64>,
    metadata: Arc<Buffer<OutputCount>>,
    capacity: usize,
}
impl Output {
    fn view(&self) -> OutputView {
        OutputView {
            columns: self.pointers.ptr().cast(),
            capacity: self.capacity as u64,
            count: self.metadata.ptr().cast(),
            error: unsafe { self.metadata.ptr().cast::<u8>().add(8).cast() },
        }
    }
}

/// All storage is attempt-owned; dropping a failed attempt precedes budgeting
/// its replacement. The immutable source status owners remain in Execution.
struct FusedRetryAttempt {
    output: Output,
    keys: Output,
    slots: Buffer<u64>,
    candidates: Buffer<u64>,
    overflow: Buffer<u32>,
    metadata: Buffer<FusedRetryMetadata>,
    dependencies: Buffer<u64>,
    #[cfg(feature = "dedup-diagnostics")]
    counters: Buffer<u64>,
}
impl FusedRetryAttempt {
    fn new(
        runtime: &Runtime,
        width: usize,
        capacity: usize,
        statuses: &[Arc<Buffer<OutputCount>>],
    ) -> Result<Self> {
        let output = allocate_output(runtime, width, capacity)?;
        let keys = allocate_output(runtime, width, capacity)?;
        let slots = Buffer::zeroed(runtime.pool.clone(), fused_slot_capacity(capacity)?)?;
        let candidates = Buffer::zeroed(runtime.pool.clone(), 1)?;
        let overflow = Buffer::zeroed(runtime.pool.clone(), 1)?;
        let metadata = unsafe { Buffer::uninitialized(runtime.pool.clone(), 1)? };
        let dependencies =
            runtime.upload_descriptors(statuses.iter().map(|status| status.ptr() as u64))?;
        Ok(Self {
            output,
            keys,
            slots,
            candidates,
            overflow,
            metadata,
            dependencies,
            #[cfg(feature = "dedup-diagnostics")]
            counters: Buffer::zeroed(runtime.pool.clone(), 568 * 6)?,
        })
    }
    fn view(&self) -> FusedRetryView {
        FusedRetryView {
            set: FusedSetView {
                slots: self.slots.ptr(),
                capacity: self.slots.len(),
                #[cfg(feature = "dedup-diagnostics")]
                counters: self.counters.ptr(),
            },
            keys: self.keys.view(),
            candidates: self.candidates.ptr(),
            overflow: self.overflow.ptr(),
        }
    }
}
pub struct Execution<'runtime, 'data> {
    runtime: &'runtime Runtime,
    dictionary: Dictionary<'data>,
    tables: Vec<Table>,
    runs: Vec<BuiltRun<'runtime>>,
    uploads: HashMap<SliceKey, TableId>,
    run_cache: HashMap<(TableId, usize), RunId>,
    memberships: Vec<Membership>,
    membership_cache: HashMap<(TableId, usize, usize), MembershipId>,
    exact_memberships: Vec<ExactMembership>,
    timings: Timings,
    pending_timings: Vec<PendingTiming>,
    started: Instant,
}

impl Drop for Execution<'_, '_> {
    fn drop(&mut self) {
        self.runtime
            .pending_timings
            .borrow_mut()
            .append(&mut self.pending_timings);
    }
}
impl<'runtime, 'data> Execution<'runtime, 'data> {
    pub fn dictionary(&self) -> &Dictionary<'data> {
        &self.dictionary
    }
    pub fn dictionary_mut(&mut self) -> &mut Dictionary<'data> {
        &mut self.dictionary
    }
    pub fn bind_host<H: HostColumns<'data>>(
        &mut self,
        host: H,
        schema: &'static str,
        width: usize,
    ) -> Result<TableId> {
        host.bind(self, schema, width)
    }
    pub fn bind_columns<T, C>(&mut self, host: &DeviceColumns<'data, T, C>) -> Result<TableId> {
        if !Arc::ptr_eq(&self.runtime.pool, DeviceColumns::pool(host)) {
            return Err(failure(
                "CQ_GPU_CONTEXT",
                "resident inputs must share a pool, stream and CUDA context",
            ));
        }
        self.dictionary.merge(host.dictionary())?;
        Ok(self.add_table(host.storage().clone()))
    }
    pub fn encode_slice<T, F>(
        &mut self,
        rows: &'data [T],
        schema: &'static str,
        width: usize,
        mut encode: F,
    ) -> Result<TableId>
    where
        F: FnMut(&'data T, &mut Dictionary<'data>, &mut [i64]) -> Result<()>,
    {
        check_rows(rows.len())?;
        let key = SliceKey {
            pointer: rows.as_ptr() as usize,
            len: rows.len(),
            rust_type: std::any::type_name::<T>(),
            schema,
            width,
        };
        if let Some(&id) = self.uploads.get(&key) {
            return Ok(id);
        }
        let mut host: Vec<Vec<i64>> = (0..width).map(|_| Vec::with_capacity(rows.len())).collect();
        let mut values = vec![0; width];
        for row in rows {
            encode(row, &mut self.dictionary, &mut values)?;
            for (column, &value) in host.iter_mut().zip(&values) {
                column.push(value);
            }
        }
        let start = TimingEvent::record(self.runtime.stream())?;
        let columns = host
            .iter()
            .map(|values| {
                EncodedColumn::from_host(self.runtime.pool.clone(), values, Codec::Plain)
                    .map(Arc::new)
            })
            .collect::<Result<Vec<_>>>()?;
        let bounds = columns.iter().map(|column| column.bounds()).collect();
        let metadata = self.runtime.metadata(rows.len())?;
        let views = self.runtime.column_views(&columns)?;
        self.record_timing(TimingStage::Upload, start)?;
        let id = self.add_table(Table {
            columns,
            views,
            metadata,
            capacity: rows.len(),
            bounds,
            statuses: Vec::new(),
        });
        self.uploads.insert(key, id);
        Ok(id)
    }
    pub fn project_table(&mut self, source: TableId, columns: &[usize]) -> Result<TableId> {
        let input = self.table(source)?;
        let projected =
            columns
                .iter()
                .map(|&column| {
                    input.columns.get(column).cloned().ok_or_else(|| {
                        failure("CQ_GPU_KEY_COLUMN", "projection outside source schema")
                    })
                })
                .collect::<Result<Vec<_>>>()?;
        let bounds = columns.iter().map(|&column| input.bounds[column]).collect();
        let table = Table {
            views: self.runtime.column_views(&projected)?,
            columns: projected,
            metadata: input.metadata.clone(),
            capacity: input.capacity,
            bounds,
            statuses: input.statuses.clone(),
        };
        Ok(self.add_table(table))
    }
    pub fn table_view(&self, id: TableId) -> Result<TableView> {
        Ok(self.table(id)?.view())
    }
    /// Immutable descriptor facts are already host-owned; the live count stays on device.
    ///
    /// # Safety
    /// The descriptor may only be used on this runtime's stream while the
    /// execution retains its source column. It owns no device allocations.
    pub unsafe fn column_view(&self, id: TableId, index: usize) -> Result<ColumnView> {
        self.table(id)?
            .columns
            .get(index)
            .map(|column| unsafe { column.view() })
            .ok_or_else(|| {
                failure(
                    "CQ_GPU_KEY_COLUMN",
                    "column descriptor outside source schema",
                )
            })
    }
    pub fn column_bounds(&self, id: TableId, index: usize) -> Result<Bounds> {
        self.table(id)?
            .bounds
            .get(index)
            .copied()
            .ok_or_else(|| failure("CQ_GPU_KEY_COLUMN", "column bounds outside source schema"))
    }
    pub fn table_width(&self, id: TableId) -> Result<usize> {
        Ok(self.table(id)?.columns.len())
    }
    pub fn run_view(&self, id: RunId) -> Result<RunView> {
        let built = self
            .runs
            .get(id.0)
            .ok_or_else(|| failure("CQ_GPU_RUN_ID", "unknown run handle"))?;
        Ok(RunView {
            rank: built.run.view(),
            columns: built.pointers.ptr().cast(),
            len: built.len,
            count: built.run.stats.cu_deviceptr() as *const u32,
            bucket_mask: built.bucket_mask,
        })
    }
    pub fn membership_view(&self, id: MembershipId) -> Result<MembershipView> {
        let value = self
            .memberships
            .get(id.0)
            .ok_or_else(|| failure("CQ_GPU_MEMBERSHIP_ID", "unknown membership handle"))?;
        Ok(MembershipView {
            bits: value.bits.ptr(),
            bounds: value.bounds.ptr(),
        })
    }
    pub fn prepare_membership(
        &mut self,
        table: TableId,
        key_column: usize,
        budget_bytes: usize,
    ) -> Result<MembershipId> {
        let key = (table, key_column, budget_bytes);
        if let Some(&id) = self.membership_cache.get(&key) {
            return Ok(id);
        }
        let _nvtx = nvtx::range(c"membership/build");
        let source = self.table(table)?;
        if key_column >= source.columns.len() {
            return Err(failure(
                "CQ_GPU_KEY_COLUMN",
                "membership key outside schema",
            ));
        }
        let start = TimingEvent::record(self.runtime.stream())?;
        let bounds = unsafe { Buffer::uninitialized(self.runtime.pool.clone(), 2)? };
        let domain = source.bounds[key_column];
        let domain_words = domain.cardinality().div_ceil(64);
        let words = domain_words.min((budget_bytes / 8) as u128) as usize;
        let reduce = domain_words > words as u128;
        let status = self.runtime.metadata(0)?;
        if reduce {
            let initialize = self
                .runtime
                .kernels
                .prepare_membership_extrema_initialize(config(1, 0))?;
            let reduce = self
                .runtime
                .kernels
                .prepare_membership_extrema(config(source.capacity, 0))?;
            unsafe {
                self.runtime.kernels.membership_extrema_initialize(
                    self.runtime.stream(),
                    &initialize,
                    bounds.ptr(),
                )?;
                self.runtime.kernels.membership_extrema(
                    self.runtime.stream(),
                    &reduce,
                    source.view(),
                    key_column,
                    bounds.ptr(),
                )?;
            }
        }
        let (min, max) = match domain {
            Bounds::Empty => (0, -1),
            Bounds::Inclusive { min, max } => (min, max),
        };
        let launch = self
            .runtime
            .kernels
            .prepare_membership_bounds(config(1, 0))?;
        unsafe {
            self.runtime.kernels.membership_bounds(
                self.runtime.stream(),
                &launch,
                source.view(),
                bounds.ptr(),
                words as u64 * 64,
                status.ptr(),
                min,
                max,
                reduce,
            )?;
        }
        let bits = Buffer::zeroed(self.runtime.pool.clone(), words)?;
        let launch = self
            .runtime
            .kernels
            .prepare_membership_insert(config(source.capacity, 0))?;
        unsafe {
            self.runtime.kernels.membership_insert(
                self.runtime.stream(),
                &launch,
                source.view(),
                key_column,
                bounds.ptr(),
                bits.ptr(),
            )?;
        }
        self.record_timing(TimingStage::Build, start)?;
        self.tables[table.0].statuses.push(status);
        let id = MembershipId(self.memberships.len());
        self.memberships.push(Membership { bits, bounds });
        self.membership_cache.insert(key, id);
        Ok(id)
    }
    pub fn prepare_run(&mut self, table: TableId, key_column: usize) -> Result<RunId> {
        if let Some(&id) = self.run_cache.get(&(table, key_column)) {
            return Ok(id);
        }
        let id = self.prepare_run_inner(table, Some(key_column), &[])?;
        self.run_cache.insert((table, key_column), id);
        Ok(id)
    }
    /// Build a lexicographic run from key levels flattened in recorded order.
    /// Columns in the returned RunView retain original schema indexes, and its
    /// rank directory addresses `keys[0]`. Equal tuples retain input-row order.
    pub fn prepare_prefix_run(&mut self, table: TableId, keys: &[usize]) -> Result<RunId> {
        let first = keys
            .first()
            .copied()
            .ok_or_else(|| failure("CQ_GPU_PREFIX_KEYS", "prefix runs require at least one key"))?;
        self.prepare_run_inner(table, Some(first), keys)
    }
    pub fn prepare_hashed_run(&mut self, table: TableId, keys: &[usize]) -> Result<RunId> {
        if keys.len() < 2 {
            return Err(failure(
                "CQ_GPU_HASH_KEYS",
                "hashed runs require composite keys",
            ));
        }
        self.prepare_run_inner(table, None, keys)
    }
    fn prepare_run_inner(
        &mut self,
        table: TableId,
        key_column: Option<usize>,
        keys: &[usize],
    ) -> Result<RunId> {
        let _nvtx = nvtx::range(c"index/run/build");
        let source = self.table(table)?;
        if keys.iter().any(|&key| key >= source.columns.len()) {
            return Err(failure("CQ_GPU_KEY_COLUMN", "run key outside schema"));
        }
        let buckets = source
            .capacity
            .max(1)
            .checked_next_power_of_two()
            .unwrap_or(self.runtime.hashed_run_bucket_limit)
            .min(self.runtime.hashed_run_bucket_limit);
        let bucket_mask = (buckets - 1) as u64;
        let domain = if let Some(key) = key_column {
            *source
                .bounds
                .get(key)
                .ok_or_else(|| failure("CQ_GPU_KEY_COLUMN", "run key outside schema"))?
        } else {
            Bounds::Inclusive {
                min: 0,
                max: bucket_mask as i64,
            }
        };
        if domain.cardinality() > gpu_blocked_run::MAX_DOMAIN_KEYS as u128 {
            return Err(failure(
                "CQ_GPU_RUN_DOMAIN",
                "dense rank domain exceeds 2^32 key positions",
            ));
        }
        if key_column.is_some()
            && keys.iter().any(|&key| {
                source.bounds[key].cardinality() > gpu_blocked_run::MAX_DOMAIN_KEYS as u128
            })
        {
            return Err(failure(
                "CQ_GPU_RUN_DOMAIN",
                "dense rank domain exceeds 2^32 key positions",
            ));
        }
        let runtime = self.runtime;
        let start = TimingEvent::record(runtime.stream())?;
        let decoded = source
            .columns
            .iter()
            .enumerate()
            .map(|(index, column)| {
                if unsafe { column.plain_buffer() }.is_some() {
                    Ok(None)
                } else {
                    decode_column(runtime, source, index).map(Some)
                }
            })
            .collect::<Result<Vec<_>>>()?;
        // Run::build gates all input reads with the device count supplied below.
        let columns: Vec<&Buffer<i64>> = source
            .columns
            .iter()
            .zip(&decoded)
            .map(|(column, decoded)| {
                unsafe { column.plain_buffer() }
                    .or(decoded.as_ref())
                    .unwrap()
            })
            .collect();
        let count = unsafe {
            rccl_memory::DeviceUninitBuffer::<u32, _>::new_in(
                runtime.pool.resource(),
                runtime.stream(),
                1,
            )?
            .assume_init()
        };
        let launch = runtime.kernels.prepare_count_u32(config(1, 0))?;
        unsafe {
            runtime.kernels.count_u32(
                runtime.stream(),
                &launch,
                source.view(),
                count.cu_deviceptr() as *mut u32,
            )?;
        }
        let hashed_keys = if key_column.is_none() {
            let slots = runtime.upload_descriptors(keys.iter().map(|&key| key as u64))?;
            let hashes =
                unsafe { Buffer::<i64>::uninitialized(runtime.pool.clone(), source.capacity)? };
            let launch = runtime
                .kernels
                .prepare_hash_run_keys(config(source.capacity, 0))?;
            unsafe {
                runtime.kernels.hash_run_keys(
                    runtime.stream(),
                    &launch,
                    source.view(),
                    slots.ptr(),
                    keys.len(),
                    bucket_mask,
                    hashes.ptr(),
                )?;
            }
            Some(hashes)
        } else {
            None
        };
        let payloads: Vec<&dyn ContiguousDeviceRange<i64>> = columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| {
                (Some(index) != key_column).then_some(*column as &dyn ContiguousDeviceRange<i64>)
            })
            .collect();
        let run = if key_column.is_some() && !keys.is_empty() {
            let prefix_keys: Vec<&dyn ContiguousDeviceRange<i64>> = keys
                .iter()
                .map(|&key| columns[key] as &dyn ContiguousDeviceRange<i64>)
                .collect();
            let domains: Vec<_> = keys
                .iter()
                .map(|&key| match source.bounds[key] {
                    Bounds::Empty => (0, -1),
                    Bounds::Inclusive { min, max } => (min, max),
                })
                .collect();
            Run::build_prefix_stream_ordered(
                runtime.context(),
                runtime.stream(),
                runtime.pool.resource(),
                &prefix_keys,
                &payloads,
                &domains,
                Some(&count),
            )
        } else {
            Run::build(
                runtime.context(),
                runtime.stream(),
                runtime.pool.resource(),
                key_column
                    .map(|key| columns[key])
                    .or(hashed_keys.as_ref())
                    .expect("run key buffer"),
                &payloads,
                match domain {
                    Bounds::Empty => (0, -1),
                    Bounds::Inclusive { min, max } => (min, max),
                },
                Some(&count),
            )
        }
        .map_err(|e| failure("CQ_GPU_RUN_BUILD", e.to_string()))?;
        let mut payload = 0;
        let pointers = runtime.upload_descriptors((0..columns.len()).map(|index| {
            if Some(index) == key_column {
                run.keys.cu_deviceptr()
            } else {
                let value = run.payloads[payload].cu_deviceptr();
                payload += 1;
                value
            }
        }))?;
        let launch = runtime.kernels.prepare_run_status(config(1, 0))?;
        let status = runtime.metadata(0)?;
        unsafe {
            runtime.kernels.run_status(
                runtime.stream(),
                &launch,
                run.stats.cu_deviceptr() as *const gpu_blocked_run::RunStats,
                status.ptr(),
            )?;
        }
        let len = source.capacity as u64;
        self.record_timing(TimingStage::Build, start)?;
        self.tables[table.0].statuses.push(status);
        let id = RunId(self.runs.len());
        self.runs.push(BuiltRun {
            run,
            pointers,
            len,
            bucket_mask,
        });
        Ok(id)
    }
    pub fn execute_selection<F>(
        &mut self,
        source: TableId,
        columns: &[usize],
        mut launch: F,
    ) -> Result<TableId>
    where
        F: FnMut(&CudaStream, OutputView, *mut u32, bool) -> Result<()>,
    {
        let input = self.table(source)?;
        let rows = input.capacity;
        let bounds =
            columns
                .iter()
                .map(|&column| {
                    input.bounds.get(column).copied().ok_or_else(|| {
                        failure("CQ_GPU_KEY_COLUMN", "selection outside source schema")
                    })
                })
                .collect::<Result<Vec<_>>>()?;
        let runtime = self.runtime;
        let start = TimingEvent::record(runtime.stream())?;
        let output = allocate_output(runtime, columns.len(), rows)?;
        let counts = Scratch::<u32>::zeroed_in(
            runtime.pool.resource(),
            runtime.stream(),
            rows.div_ceil(256).max(1),
        )?;
        let _scratch = if rows != 0 {
            launch(
                runtime.stream(),
                output.view(),
                counts.cu_deviceptr() as *mut u32,
                false,
            )?;
            let scratch = gpu_blocked_run::exclusive_scan(
                runtime.stream(),
                runtime.pool.resource(),
                &counts,
            )?;
            launch(
                runtime.stream(),
                output.view(),
                counts.cu_deviceptr() as *mut u32,
                true,
            )?;
            scratch
        } else {
            Vec::new()
        };
        let table = self.complete_output(output, bounds)?;
        self.record_timing(TimingStage::Build, start)?;
        Ok(self.add_table(table))
    }
    pub fn execute_bounded<F>(
        &mut self,
        bounds: &[Bounds],
        capacity: usize,
        mut launch: F,
    ) -> Result<TableId>
    where
        F: FnMut(&CudaStream, OutputView) -> Result<()>,
    {
        check_rows(capacity)?;
        let start = TimingEvent::record(self.runtime.stream())?;
        let output = allocate_output(self.runtime, bounds.len(), capacity)?;
        launch(self.runtime.stream(), output.view())?;
        let table = self.complete_output(output, bounds.to_vec())?;
        self.record_timing(TimingStage::Kernel, start)?;
        Ok(self.add_table(table))
    }
    /// Run a pure bounded query, retrying only its own capacity overflow once.
    ///
    /// Inputs must remain immutable and the callback must restart the complete
    /// query, including resetting its frontier, on each invocation. Every
    /// candidate must be counted even after writes exceed capacity. The second
    /// attempt allocates exactly that total with fresh count/error metadata;
    /// no partial rows are copied. Canonicalization belongs after this policy.
    /// Successful candidate counts seed the next call at this hole/site/width.
    /// The validated count bounds retained table work without shrinking buffers.
    ///
    /// Each attempt downloads one 16-byte summary of its count/error and all
    /// retained dependency statuses, never rows. Dependency errors, other
    /// output errors, and any second-attempt error/count change are terminal.
    pub fn execute_retry<F>(
        &mut self,
        site: &'static str,
        bounds: &[Bounds],
        initial_capacity: usize,
        mut launch: F,
    ) -> Result<TableId>
    where
        F: FnMut(&CudaStream, OutputView) -> Result<()>,
    {
        check_rows(initial_capacity)?;
        let runtime = self.runtime;
        let key = fused_retry::Key::current(site, bounds.len());
        let initial_capacity = runtime
            .materialization_history
            .borrow()
            .capacity(key, initial_capacity);
        let start = TimingEvent::record(runtime.stream())?;
        let statuses = self.output_statuses();
        let dependencies =
            runtime.upload_descriptors(statuses.iter().map(|status| status.ptr() as u64))?;
        let metadata = unsafe { Buffer::<RetryMetadata>::uninitialized(runtime.pool.clone(), 1)? };
        let check = runtime.kernels.prepare_retry_metadata(config(1, 0))?;
        let read_metadata = |output: &Output| -> Result<RetryMetadata> {
            unsafe {
                runtime.kernels.retry_metadata(
                    runtime.stream(),
                    &check,
                    output.view(),
                    dependencies.ptr(),
                    dependencies.len(),
                    metadata.ptr(),
                )?;
            }
            record_host_wait(true);
            RETRY_READBACKS.with(|count| count.set(count.get().saturating_add(1)));
            let summary = metadata.to_host()?[0];
            runtime.reclaim_descriptors();
            Ok(summary)
        };

        let initial_capacity = runtime
            .retry_budget()?
            .materialization_capacity(bounds.len(), initial_capacity)?;
        let mut output = allocate_output(runtime, bounds.len(), initial_capacity)?;
        launch(runtime.stream(), output.view())?;
        let mut summary = read_metadata(&output)?;
        if let Some(capacity) = retry_capacity(summary, initial_capacity, None)? {
            drop(output);
            let budget = runtime.retry_budget()?;
            if budget.materialization_capacity(bounds.len(), capacity)? != capacity {
                return Err(failure(
                    "CQ_GPU_RETRY_RESOURCE",
                    format!(
                        "exact bounded output cannot fit: width={}, rows={capacity}, budget_bytes={}, reserved_headroom={}",
                        bounds.len(),
                        budget.materialization_bytes,
                        budget.headroom,
                    ),
                ));
            }
            output = allocate_output(runtime, bounds.len(), capacity)?;
            launch(runtime.stream(), output.view())?;
            summary = read_metadata(&output)?;
            retry_capacity(summary, capacity, Some(capacity as u64))?;
        }
        // The successful readback has already proved this count fits storage.
        // Keep owning the allocation, but never launch downstream work over slack.
        output.capacity = summary.rows as usize;
        // The synchronized summary checked every immutable ancestor status.
        // They cannot fail later; retaining them would only recheck old work.
        let table = self.retain_output(output, bounds.to_vec(), Vec::new())?;
        self.record_timing(TimingStage::Kernel, start)?;
        runtime
            .materialization_history
            .borrow_mut()
            .success(key, summary.rows);
        Ok(self.add_table(table))
    }
    /// Run pure bounded exact admission with at most twelve fresh attempts.
    ///
    /// Estimates persist in Runtime per `(fused_retry_scope label, site, width)`.
    /// Initial capacity is `max(1024, 2 * last_nonempty_distinct + hint)` (just
    /// `max(1024, hint)` before a nonempty success), capped by u32 and the budget.
    /// Empty successful calls retain the estimate but report their actual zero.
    /// Overflow proposes four times completed arena occupancy, bounded by the
    /// immutable candidate total, u32, available VRAM and the pool allowance.
    /// Candidate totals may exceed u32; distinct output may not.
    ///
    /// Every attempt has fresh output, ticketed arena, rounded slots, counters
    /// and metadata. The callback must reset its frontier on every invocation,
    /// read immutable input snapshots, count all candidates even after overflow,
    /// and compact only new admissions. Failed partial storage is dropped before
    /// measuring the next budget. Allocation/fatal errors never authorize replay.
    /// The 40-byte checked summary (88 with diagnostics) must preserve candidate
    /// count across attempts; failure to grow or exhausting the bound is terminal.
    /// Successful output is retained without another deduplication pass.
    ///
    /// With `timings`, report CUDA events bracket the callback, including its
    /// frontier reset and compaction, excluding allocation and metadata work.
    pub fn execute_retry_fused<F>(
        &mut self,
        site: &'static str,
        bounds: &[Bounds],
        capacity_hint: Option<usize>,
        mut launch: F,
    ) -> Result<TableId>
    where
        F: FnMut(&CudaStream, OutputView, FusedRetryView) -> Result<()>,
    {
        let runtime = self.runtime;
        let key = fused_retry::Key::current(site, bounds.len());
        let (call, mut target) = runtime.fused_retries.borrow_mut().begin(key, capacity_hint);
        let start = TimingEvent::record(runtime.stream())?;
        let statuses = self.output_statuses();
        let check = runtime.kernels.prepare_fused_retry_metadata(config(1, 0))?;
        let mut previous_capacity = None;
        let mut expected_candidates = None;
        for attempt_number in 1..=fused_retry::MAX_ATTEMPTS {
            let budget = runtime.retry_budget()?;
            let capacity =
                budget.capacity(bounds.len(), statuses.len(), target, previous_capacity)?;
            let bytes = fused_retry::attempt_bytes(bounds.len(), capacity, statuses.len())?;
            let attempt = FusedRetryAttempt::new(runtime, bounds.len(), capacity, &statuses)
                .map_err(|error| failure("CQ_GPU_FUSED_RETRY_ALLOCATION", format!(
                    "{key:?}, attempt={attempt_number}, capacity={capacity}, requested_bytes={}, pooled_bytes={}, budget_bytes={}: {error}",
                    bytes.requested, bytes.pooled, budget.bytes,
                )))?;
            let launch_start = TimingEvent::record(runtime.stream())?;
            runtime
                .fused_retries
                .borrow_mut()
                .launch(key, call, capacity, bytes.requested);
            let launch_result = launch(runtime.stream(), attempt.output.view(), attempt.view());
            let launch_end = TimingEvent::record(runtime.stream())
                .map_err(|error| -> Error {
                    match &launch_result {
                        Ok(()) => Box::new(error),
                        Err(callback_error) => failure("CQ_GPU_FUSED_RETRY_TIMING", format!(
                            "callback failed: {callback_error}; recording its end event also failed: {error}",
                        )),
                    }
                })?;
            if let Err(error) = launch_result {
                if launch_start.0.is_some() {
                    record_host_wait(false);
                }
                let kernel_ms = launch_start.elapsed_ms(&launch_end)
                    .map_err(|timing_error| failure("CQ_GPU_FUSED_RETRY_TIMING", format!(
                        "callback failed: {error}; timing its discarded work also failed: {timing_error}",
                    )))? as f64;
                runtime
                    .fused_retries
                    .borrow_mut()
                    .timed(key, kernel_ms, true);
                return Err(error);
            }
            unsafe {
                runtime.kernels.fused_retry_metadata(
                    runtime.stream(),
                    &check,
                    attempt.output.view(),
                    attempt.view(),
                    attempt.dependencies.ptr(),
                    attempt.dependencies.len(),
                    attempt.metadata.ptr(),
                )?;
            }
            record_host_wait(true);
            FUSED_RETRY_READBACKS.with(|count| count.set(count.get().saturating_add(1)));
            let summary = attempt.metadata.to_host()?[0];
            runtime.reclaim_descriptors();
            let kernel_ms = launch_start.elapsed_ms(&launch_end)? as f64;
            #[cfg(feature = "dedup-diagnostics")]
            eprintln!(
                "CQ_GPU_FUSED_RETRY_DIAGNOSTIC site={site} width={} attempt={attempt_number} capacity={capacity} slots={} rows={} distinct={} candidates={} overflow={} inserted={} duplicates={} busy={} full={} probes={} max_probes={}",
                bounds.len(),
                attempt.slots.len(),
                summary.rows,
                summary.distinct,
                summary.candidates,
                summary.overflow,
                summary.diagnostics[0],
                summary.diagnostics[1],
                summary.diagnostics[2],
                summary.diagnostics[3],
                summary.diagnostics[4],
                summary.diagnostics[5],
            );
            let next =
                fused_retry::next_capacity(summary, capacity, expected_candidates, attempt_number);
            match next {
                Ok(Some(grown)) => {
                    runtime
                        .fused_retries
                        .borrow_mut()
                        .timed(key, kernel_ms, true);
                    // Drop arena/output/slots/all counters before the next CUDA
                    // free-memory query and custom-pool statistics snapshot.
                    drop(attempt);
                    target = grown;
                    previous_capacity = Some(capacity);
                    expected_candidates = Some(summary.candidates);
                }
                Ok(None) => {
                    let mut output = attempt.output;
                    output.capacity = summary.rows as usize;
                    // This successful synchronized summary discharged ancestors.
                    let table = match self.retain_output(output, bounds.to_vec(), Vec::new()) {
                        Ok(table) => table,
                        Err(error) => {
                            runtime
                                .fused_retries
                                .borrow_mut()
                                .timed(key, kernel_ms, true);
                            return Err(error);
                        }
                    };
                    if let Err(error) = self.record_timing(TimingStage::Kernel, start) {
                        runtime
                            .fused_retries
                            .borrow_mut()
                            .timed(key, kernel_ms, true);
                        return Err(error);
                    }
                    let mut reports = runtime.fused_retries.borrow_mut();
                    reports.timed(key, kernel_ms, false);
                    reports.success(key, summary.distinct);
                    return Ok(self.add_table(table));
                }
                Err(error) => {
                    runtime
                        .fused_retries
                        .borrow_mut()
                        .timed(key, kernel_ms, true);
                    return Err(error);
                }
            }
        }
        unreachable!("the final failed attempt returns the bounded-attempt error")
    }
    /// Resume unfinished work; only one u32 control word crosses per attempt.
    pub fn execute_resumable<F>(
        &mut self,
        bounds: &[Bounds],
        initial_capacity: usize,
        state_words: usize,
        mut launch: F,
    ) -> Result<TableId>
    where
        F: FnMut(&CudaStream, OutputView, ResumeView) -> Result<()>,
    {
        self.execute_resumable_with_policy(
            bounds,
            initial_capacity,
            state_words,
            None,
            false,
            |stream, output, resume, _| launch(stream, output, resume),
        )
    }
    /// Compact completed output while preserving pending kernel reservations.
    /// `max_reservation` bounds one append (one row for serial emission, or
    /// threads times items for cooperative emission), not merely block size.
    pub fn execute_resumable_compacting<F>(
        &mut self,
        bounds: &[Bounds],
        initial_capacity: usize,
        state_words: usize,
        max_reservation: usize,
        mut launch: F,
    ) -> Result<TableId>
    where
        F: FnMut(&CudaStream, OutputView, ResumeView) -> Result<()>,
    {
        self.execute_resumable_with_policy(
            bounds,
            initial_capacity,
            state_words,
            Some(max_reservation),
            false,
            |stream, output, resume, _| launch(stream, output, resume),
        )
    }
    pub fn execute_resumable_fused<F>(
        &mut self,
        bounds: &[Bounds],
        initial_capacity: usize,
        state_words: usize,
        launch: F,
    ) -> Result<TableId>
    where
        F: FnMut(&CudaStream, OutputView, ResumeView, FusedSetView) -> Result<()>,
    {
        self.execute_resumable_with_policy(
            bounds,
            initial_capacity,
            state_words,
            None,
            true,
            launch,
        )
    }
    fn execute_resumable_with_policy<F>(
        &mut self,
        bounds: &[Bounds],
        initial_capacity: usize,
        state_words: usize,
        compacting_reservation: Option<usize>,
        fused: bool,
        mut launch: F,
    ) -> Result<TableId>
    where
        F: FnMut(&CudaStream, OutputView, ResumeView, FusedSetView) -> Result<()>,
    {
        check_rows(initial_capacity)?;
        let width = bounds.len();
        let runtime = self.runtime;
        let start = TimingEvent::record(runtime.stream())?;
        let state = Buffer::<u64>::zeroed(runtime.pool.clone(), state_words)?;
        let control = Buffer::<u32>::zeroed(runtime.pool.clone(), 1)?;
        let resume = ResumeView {
            state: state.ptr(),
            control: control.ptr(),
        };
        let mut output = allocate_output(runtime, width, initial_capacity.max(1))?;
        let allocate_set = |capacity: usize| -> Result<Buffer<u64>> {
            let slots = fused_slot_capacity(capacity)?;
            Ok(Buffer::<u64>::zeroed(runtime.pool.clone(), slots)?)
        };
        let mut set = if fused {
            Some(allocate_set(output.capacity)?)
        } else {
            None
        };
        #[cfg(feature = "dedup-diagnostics")]
        let counters = if fused {
            Some(Buffer::<u64>::zeroed(runtime.pool.clone(), 568 * 6)?)
        } else {
            None
        };
        #[cfg(feature = "dedup-diagnostics")]
        let mut attempts = 0usize;
        loop {
            #[cfg(feature = "dedup-diagnostics")]
            {
                attempts += 1;
            }
            let _nvtx = nvtx::range(c"resumable/attempt");
            unsafe {
                cuda_core::memory::memset_d8_async(
                    control.ptr() as u64,
                    0,
                    4,
                    runtime.stream().cu_stream(),
                )?;
            }
            let fused_view = FusedSetView {
                slots: set.as_ref().map_or(std::ptr::null_mut(), |set| set.ptr()),
                capacity: if fused {
                    (output.capacity * 2).next_power_of_two()
                } else {
                    0
                },
                #[cfg(feature = "dedup-diagnostics")]
                counters: counters
                    .as_ref()
                    .map_or(std::ptr::null_mut(), |buffer| buffer.ptr()),
            };
            launch(runtime.stream(), output.view(), resume, fused_view)?;
            record_host_wait(true);
            RESUME_READBACKS.with(|count| count.set(count.get().saturating_add(1)));
            let suspended = control.to_host()?[0] != 0;
            runtime.reclaim_descriptors();
            if !suspended {
                break;
            }
            if fused {
                let clamp = runtime.kernels.prepare_fused_clamp(config(1, 0))?;
                unsafe {
                    runtime
                        .kernels
                        .fused_clamp(runtime.stream(), &clamp, output.view())?;
                }
            }
            if let Some(max_reservation) = compacting_reservation {
                let _nvtx = nvtx::range(c"resumable/compact");
                let old_capacity = output.capacity;
                // Checkpoints contain only query cursors/registers, never
                // output addresses. Exact prefix replacement cannot replay or
                // skip a candidate; the suspended append is still pending.
                let mut work = runtime.execution();
                let table = work.complete_output(output, bounds.to_vec())?;
                let source = work.add_table(table);
                let distinct = work.dedup(source)?;
                COMPACTION_READBACKS.with(|count| count.set(count.get().saturating_add(1)));
                let rows = work.checked_rows(distinct)?;
                // A pending append can contain every item of every lane. Leave
                // room for the printer's complete reservation, not just a warp
                // or block of scalar rows, so exact compaction makes progress.
                let capacity = compacting_capacity(rows, old_capacity, max_reservation)?;
                output = allocate_output(runtime, width, capacity)?;
                let source = work.table(distinct)?;
                let append = runtime.kernels.prepare_append(config(rows, 0))?;
                unsafe {
                    runtime.kernels.append(
                        runtime.stream(),
                        &append,
                        source.view(),
                        width,
                        output.view(),
                    )?;
                }
                continue;
            }
            let capacity = output
                .capacity
                .checked_mul(2)
                .unwrap_or(u32::MAX as usize)
                .min(u32::MAX as usize);
            if capacity <= output.capacity {
                return Err(failure(
                    "CQ_GPU_OUTPUT_ROWS",
                    "resumable output exceeds u32::MAX",
                ));
            }
            let mut grown = allocate_output(runtime, width, capacity).map_err(|error| {
                failure(
                    "CQ_GPU_RESUMABLE_ALLOCATION",
                    format!("width={width}, capacity={capacity}: {error}"),
                )
            })?;
            for (source, destination) in output.columns.iter().zip(&grown.columns) {
                unsafe {
                    cuda_core::memory::memcpy_dtod_async(
                        destination.ptr() as u64,
                        source.ptr() as u64,
                        output.capacity * 8,
                        runtime.stream().cu_stream(),
                    )?;
                }
            }
            grown.metadata = output.metadata.clone();
            output = grown;
            if fused {
                let slots = allocate_set(capacity)?;
                let view = FusedSetView {
                    slots: slots.ptr(),
                    capacity: (capacity * 2).next_power_of_two(),
                    #[cfg(feature = "dedup-diagnostics")]
                    counters: counters
                        .as_ref()
                        .map_or(std::ptr::null_mut(), |buffer| buffer.ptr()),
                };
                let rehash = runtime.kernels.prepare_fused_rehash(config(capacity, 0))?;
                unsafe {
                    runtime.kernels.fused_rehash(
                        runtime.stream(),
                        &rehash,
                        output.view(),
                        width,
                        view,
                    )?;
                }
                set = Some(slots);
            }
        }
        #[cfg(feature = "dedup-diagnostics")]
        if let Some(counters) = counters {
            let mut totals = [0u64; 6];
            for block in counters.to_host()?.chunks_exact(6) {
                for index in 0..5 {
                    totals[index] += block[index];
                }
                totals[5] = totals[5].max(block[5]);
            }
            let rows = output.metadata.to_host()?[0].rows;
            eprintln!(
                "CQ_GPU_FUSED_DIAGNOSTIC width={width} initial_capacity={initial_capacity} capacity={} slots={} rows={rows} attempts={attempts} rehashes={} inserted={} duplicates={} busy={} full={} probes={} max_probes={}",
                output.capacity,
                (output.capacity * 2).next_power_of_two(),
                attempts - 1,
                totals[0],
                totals[1],
                totals[2],
                totals[3],
                totals[4],
                totals[5]
            );
        }
        let table = self.complete_output(output, bounds.to_vec())?;
        self.record_timing(TimingStage::Kernel, start)?;
        Ok(self.add_table(table))
    }
    fn complete_output(&self, output: Output, bounds: Vec<Bounds>) -> Result<Table> {
        let runtime = self.runtime;
        let check = runtime.kernels.prepare_validate_output(config(1, 0))?;
        unsafe {
            runtime
                .kernels
                .validate_output(runtime.stream(), &check, output.view())?;
        }
        self.retain_output(output, bounds, self.output_statuses())
    }
    fn output_statuses(&self) -> Vec<Arc<Buffer<OutputCount>>> {
        let mut statuses = Vec::new();
        for table in &self.tables {
            for status in table
                .statuses
                .iter()
                .chain(std::iter::once(&table.metadata))
            {
                if !statuses.iter().any(|other| Arc::ptr_eq(status, other)) {
                    statuses.push(status.clone());
                }
            }
        }
        statuses
    }
    /// Retain output and its dependencies. Callers must validate before any
    /// dependent read, or keep this table private until the round check succeeds.
    fn retain_output(
        &self,
        output: Output,
        bounds: Vec<Bounds>,
        mut statuses: Vec<Arc<Buffer<OutputCount>>>,
    ) -> Result<Table> {
        let columns = output
            .columns
            .into_iter()
            .zip(&bounds)
            .map(|(column, &bound)| {
                EncodedColumn::from_plain_buffer(column, output.metadata.clone(), 0, bound)
                    .map(Arc::new)
            })
            .collect::<Result<Vec<_>>>()?;
        let views = self.runtime.column_views(&columns)?;
        statuses.push(output.metadata.clone());
        Ok(Table {
            columns,
            views,
            metadata: output.metadata,
            capacity: output.capacity,
            bounds,
            statuses,
        })
    }
    pub fn dedup(&mut self, table: TableId) -> Result<TableId> {
        let _nvtx = nvtx::range(c"admission/dedup");
        let source = self.table(table)?;
        if source.capacity < 2 {
            return Ok(table);
        }
        let runtime = self.runtime;
        let start = TimingEvent::record(runtime.stream())?;
        let width = source.columns.len();
        let words = width.max(1);
        let packed_len = words
            .checked_mul(source.capacity)
            .ok_or_else(|| failure("CQ_GPU_PACKED_KEY_SIZE", "packed rows exceed address space"))?;
        let capacity = source
            .capacity
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or_else(|| failure("CQ_GPU_SET_CAPACITY", "set capacity overflow"))?;
        let mut scratch = runtime.dedup_scratch.borrow_mut();
        scratch.reserve(&runtime.pool, packed_len, capacity)?;
        let packed = scratch.packed.as_ref().unwrap();
        let slots = scratch.slots.as_ref().unwrap();
        let output = allocate_output(runtime, width, source.capacity)?;
        let pack = runtime.kernels.prepare_pack(config(source.capacity, 0))?;
        let insert = runtime
            .kernels
            .prepare_set_insert(config(source.capacity, 0))?;
        let clear = runtime.kernels.prepare_clear_set(config(capacity, 0))?;
        unsafe {
            runtime.kernels.clear_set(
                runtime.stream(),
                &clear,
                source.view(),
                slots.ptr(),
                capacity,
            )?;
            runtime.kernels.pack(
                runtime.stream(),
                &pack,
                source.view(),
                width,
                packed.ptr(),
                words,
            )?;
            runtime.kernels.set_insert(
                runtime.stream(),
                &insert,
                source.view(),
                width,
                packed.ptr(),
                words,
                slots.ptr(),
                capacity,
                output.view(),
                true,
            )?;
        }
        let bounds = source.bounds.clone();
        let result = self.complete_output(output, bounds)?;
        self.record_timing(TimingStage::Dedup, start)?;
        Ok(self.add_table(result))
    }
    /// Only a recorded derived provider selects this codec.
    pub fn reencode_table(&mut self, source: TableId, codec: Codec) -> Result<TableId> {
        let table = self.table(source)?;
        if table.columns.iter().all(|column| column.codec() == codec) {
            return Ok(source);
        }
        let start = TimingEvent::record(self.runtime.stream())?;
        let columns = table
            .columns
            .iter()
            .enumerate()
            .zip(&table.bounds)
            .map(|((index, column), &bound)| {
                if column.codec() == codec {
                    return Ok(column.clone());
                }
                let decoded;
                let input = match unsafe { column.plain_buffer() } {
                    Some(buffer) => buffer,
                    None => {
                        decoded = decode_column(self.runtime, table, index)?;
                        if codec == Codec::Plain {
                            return EncodedColumn::from_plain_buffer(
                                decoded,
                                table.metadata.clone(),
                                0,
                                bound,
                            )
                            .map(Arc::new);
                        }
                        &decoded
                    }
                };
                unsafe {
                    EncodedColumn::from_device(
                        self.runtime.pool.clone(),
                        input.ptr(),
                        table.capacity,
                        table.metadata.clone(),
                        0,
                        codec,
                        bound,
                    )
                }
                .map(Arc::new)
            })
            .collect::<Result<Vec<_>>>()?;
        let table = Table {
            views: self.runtime.column_views(&columns)?,
            columns,
            ..table.clone()
        };
        self.record_timing(TimingStage::Build, start)?;
        Ok(self.add_table(table))
    }
    pub fn into_columns<Row, C: Representation>(
        &mut self,
        table: TableId,
        columns: &[usize],
    ) -> Result<DeviceColumns<'data, Row, C>> {
        let table = self.project_table(table, columns)?;
        let table = self.reencode_table(table, C::CODEC)?;
        let table = self.table(table)?.clone();
        // Completed tables retain their views, count owners and ancestor
        // statuses; the representation and dictionary match the typed result.
        Ok(unsafe {
            DeviceColumns::from_parts(
                table,
                Arc::new(self.dictionary.clone()),
                self.runtime.pool.clone(),
            )
        })
    }
    pub fn frontier(&self, slots: u32, descriptor_words: u32) -> Result<Frontier<'runtime>> {
        Frontier::new(self.runtime, slots, descriptor_words)
    }
    fn record_timing(&mut self, stage: TimingStage, start: TimingEvent) -> Result<()> {
        if start.0.is_none() {
            return Ok(());
        }
        let end = TimingEvent::record(self.runtime.stream())?;
        self.pending_timings
            .push(PendingTiming { stage, start, end });
        Ok(())
    }
    /// Explicit timing-collection boundary. One synchronization covers every
    /// preceding operation, including asynchronous errors and ordered frees.
    pub fn finish(&mut self) -> Result<Timings> {
        record_host_wait(false);
        self.runtime.stream().synchronize()?;
        self.runtime.reclaim_descriptors();
        self.runtime
            .pending_timings
            .borrow_mut()
            .append(&mut self.pending_timings);
        self.timings = self.runtime.collect_timings()?;
        self.timings.total_ms =
            self.timings.build_ms + self.timings.kernel_ms + self.timings.dedup_ms;
        self.timings.wall_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        LAST_TIMINGS.with(|last| last.set(Some(self.timings)));
        Ok(self.timings)
    }
    fn table(&self, id: TableId) -> Result<&Table> {
        self.tables
            .get(id.0)
            .ok_or_else(|| failure("CQ_GPU_TABLE_ID", "unknown table handle"))
    }
    fn add_table(&mut self, table: Table) -> TableId {
        let id = TableId(self.tables.len());
        self.tables.push(table);
        id
    }
}

pub struct Frontier<'runtime> {
    runtime: &'runtime Runtime,
    pool: Buffer<u32>,
    counters: Buffer<u64>,
    reset_once: Cell<bool>,
    slots: u32,
    descriptor_words: u32,
}
impl<'runtime> Frontier<'runtime> {
    fn new(runtime: &'runtime Runtime, slots: u32, descriptor_words: u32) -> Result<Self> {
        let words = (descriptor_words as usize)
            .checked_add(3)
            .and_then(|words| words.checked_mul(slots as usize))
            .ok_or_else(|| failure("CQ_GPU_FRONTIER_SIZE", "descriptor pool size overflow"))?;
        Ok(Self {
            runtime,
            pool: Buffer::zeroed(runtime.pool.clone(), words)?,
            counters: runtime.upload_descriptors([0u64; 2].into_iter())?,
            reset_once: Cell::new(false),
            slots,
            descriptor_words,
        })
    }
    pub fn view(&self) -> FrontierView {
        FrontierView {
            pool: self.pool.ptr(),
            slots: self.slots,
            descriptor_words: self.descriptor_words,
            cursor: self.counters.ptr(),
            remaining: unsafe { self.counters.ptr().add(1) },
        }
    }
    pub fn reset(&self, root_tiles: u64) -> Result<()> {
        if self.slots == 0 {
            return Ok(());
        }
        if self.reset_once.replace(true) {
            unsafe {
                cuda_core::memory::memset_d8_async(
                    self.pool.ptr() as u64,
                    0,
                    self.pool.bytes(),
                    self.runtime.stream().cu_stream(),
                )?;
            }
        }
        self.runtime.descriptor_staging.borrow_mut().upload(
            self.runtime,
            &self.counters,
            [0, root_tiles].into_iter(),
        )?;
        Ok(())
    }
}
/// Only a first-attempt capacity error with an exact, supported total can grow.
fn retry_capacity(
    metadata: RetryMetadata,
    capacity: usize,
    expected_rows: Option<u64>,
) -> Result<Option<usize>> {
    if metadata.dependency_error != 0 {
        return Err(device_status_failure(metadata.dependency_error));
    }
    if metadata.error != 0 && (metadata.error != 4 || expected_rows.is_some()) {
        return Err(device_status_failure(metadata.error));
    }
    if metadata.rows > u32::MAX as u64 {
        return Err(device_status_failure(5));
    }
    if metadata.error == 4 {
        if metadata.rows <= capacity as u64 {
            return Err(device_status_failure(4));
        }
        return Ok(Some(metadata.rows as usize));
    }
    if metadata.rows > capacity as u64 {
        return Err(device_status_failure(4));
    }
    if let Some(expected) = expected_rows {
        if metadata.rows != expected {
            return Err(failure(
                "CQ_GPU_RETRY_COUNT",
                format!(
                    "pure retry changed candidate count from {expected} to {}",
                    metadata.rows,
                ),
            ));
        }
    }
    Ok(None)
}

fn compacting_capacity(rows: usize, old_capacity: usize, max_reservation: usize) -> Result<usize> {
    if max_reservation == 0 {
        return Err(failure(
            "CQ_GPU_RESERVATION_BOUND",
            "compacting execution requires a nonzero reservation bound",
        ));
    }
    let required = rows
        .checked_add(max_reservation)
        .filter(|&capacity| capacity <= u32::MAX as usize)
        .ok_or_else(|| {
            failure(
                "CQ_GPU_OUTPUT_ROWS",
                "canonical output plus a pending reservation exceeds u32::MAX",
            )
        })?;
    Ok(rows
        .saturating_mul(2)
        .min(u32::MAX as usize)
        .max(required)
        .max(old_capacity))
}

fn fused_slot_capacity(capacity: usize) -> Result<usize> {
    capacity
        .max(1)
        .checked_mul(2)
        .and_then(usize::checked_next_power_of_two)
        .ok_or_else(|| failure("CQ_GPU_SET_CAPACITY", "fused set capacity overflow"))
}

fn device_status_failure(error: u32) -> Error {
    // Keep these diagnostics identical to device-columns' checked_count.
    let (code, detail) = match error {
        1 => (
            "CQ_GPU_OUTPUT_OVERFLOW",
            "logical output count exceeded u64",
        ),
        2 => (
            "CQ_GPU_EXPANSION_OVERFLOW",
            "tile expansion exceeded u32 offset domain",
        ),
        3 => (
            "CQ_GPU_SET_FULL",
            "exact device set exhausted its bounded probe table",
        ),
        4 => (
            "CQ_GPU_OUTPUT_CAPACITY",
            "logical output exceeds allocated capacity",
        ),
        5 => ("CQ_GPU_OUTPUT_ROWS", "output cardinality exceeds u32::MAX"),
        6 => (
            "CQ_GPU_MEMBERSHIP_DOMAIN",
            "recorded membership bitmap budget exceeded",
        ),
        7 => ("CQ_GPU_RUN_BUILD", "invalid device run count or key domain"),
        _ => ("CQ_GPU_KERNEL_ERROR", "unknown device error code"),
    };
    failure(code, detail)
}

fn check_rows(rows: usize) -> Result<()> {
    if rows > u32::MAX as usize {
        Err(failure(
            "CQ_GPU_OUTPUT_ROWS",
            "relation cardinality exceeds u32::MAX",
        ))
    } else {
        Ok(())
    }
}
fn config(rows: usize, shared: u32) -> LaunchConfig1D {
    LaunchConfig1D::new(rows.div_ceil(256).clamp(1, 568) as u32, 256, shared)
}
fn allocate_output(runtime: &Runtime, width: usize, capacity: usize) -> Result<Output> {
    let columns = (0..width)
        .map(|_| unsafe { Buffer::uninitialized(runtime.pool.clone(), capacity) })
        .collect::<Result<Vec<_>>>()?;
    let pointers = runtime.upload_descriptors(columns.iter().map(|column| column.ptr() as u64))?;
    Ok(Output {
        columns,
        pointers,
        metadata: runtime.metadata(0)?,
        capacity,
    })
}
fn decode_column(runtime: &Runtime, table: &Table, index: usize) -> Result<Buffer<i64>> {
    let launch = runtime
        .kernels
        .prepare_decode_column(config(table.capacity, 0))?;
    // decode_column writes every live row; consumers use table.metadata.
    let output = unsafe { Buffer::uninitialized(runtime.pool.clone(), table.capacity)? };
    unsafe {
        runtime.kernels.decode_column(
            runtime.stream(),
            &launch,
            table.view(),
            index,
            output.ptr(),
        )?;
    }
    Ok(output)
}

#[cfg(test)]
mod frozen_tests {
    use super::{RuntimeError, frozen_layout_position, validate_frozen_layout};

    #[test]
    fn frozen_layouts_preserve_composite_and_prefix_certificates() {
        assert!(validate_frozen_layout(&[&[1]], 3).is_ok());
        assert!(validate_frozen_layout(&[&[0, 1]], 3).is_ok());
        assert!(validate_frozen_layout(&[&[1], &[0]], 3).is_ok());
        assert!(validate_frozen_layout(&[&[1, 2], &[0]], 3).is_ok());
        assert!(validate_frozen_layout(&[&[1, 2], &[0, 3]], 4).is_ok());
        let layouts = vec![vec![vec![0, 1]], vec![vec![1], vec![0]]];
        assert_eq!(frozen_layout_position(&layouts, &[&[0, 1]]).unwrap(), 0);
        assert_eq!(frozen_layout_position(&layouts, &[&[1], &[0]]).unwrap(), 1);
        for levels in [&[&[0][..]][..], &[&[0][..], &[1][..]][..], &[][..]] {
            let error = frozen_layout_position(&layouts, levels).unwrap_err();
            assert_eq!(
                error.downcast::<RuntimeError>().unwrap().code,
                "CQ_GPU_FROZEN_LAYOUT_MISSING"
            );
        }
    }

    #[test]
    fn frozen_layouts_reject_invalid_keys_before_building() {
        for levels in [
            &[][..],
            &[&[][..]][..],
            &[&[3][..]][..],
            &[&[0, 0][..]][..],
            &[&[0, 1][..], &[1][..]][..],
        ] {
            let error = validate_frozen_layout(levels, 3).unwrap_err();
            assert_eq!(
                error.downcast::<RuntimeError>().unwrap().code,
                "CQ_GPU_FROZEN_LEVELS"
            );
        }
    }
}

#[cfg(test)]
mod retry_tests {
    use super::{Result, RetryMetadata, RuntimeError, compacting_capacity, retry_capacity};

    #[test]
    fn failed_staging_growth_keeps_previous_uploads_pending() {
        let mut staging = super::DescriptorStaging::default();
        assert!(!staging.pending());
        staging.used = 1;
        assert!(staging.pending());
        staging.chunk = 1;
        staging.used = 0;
        assert!(staging.pending());
        staging.rewind();
        assert!(!staging.pending());
    }

    #[test]
    fn compaction_reserves_every_item_of_a_pending_append() {
        for reservation in [1, 256, 1024] {
            for rows in [0, 1, 127, 512, 1024] {
                let capacity = compacting_capacity(rows, 1024, reservation).unwrap();
                assert!(capacity >= rows + reservation);
                assert!(capacity >= rows * 2);
                assert!(capacity >= 1024);
            }
        }
        assert_eq!(compacting_capacity(0, 256, 1024).unwrap(), 1024);
        assert_eq!(compacting_capacity(64, 1024, 1024).unwrap(), 1088);
        assert_eq!(compacting_capacity(1000, 1024, 1024).unwrap(), 2024);
        assert_eq!(compacting_capacity(0, 64, 256).unwrap(), 256);
    }

    #[test]
    fn compaction_rejects_reservations_that_cannot_make_progress() {
        assert_eq!(
            error_code(compacting_capacity(0, 1024, 0).map(Some)),
            "CQ_GPU_RESERVATION_BOUND"
        );
        assert_eq!(
            compacting_capacity(u32::MAX as usize - 1024, 1024, 1024).unwrap(),
            u32::MAX as usize
        );
        for rows in [u32::MAX as usize - 1023, usize::MAX] {
            assert_eq!(
                error_code(compacting_capacity(rows, 1024, 1024).map(Some)),
                "CQ_GPU_OUTPUT_ROWS"
            );
        }
    }

    fn error_code(result: Result<Option<usize>>) -> &'static str {
        result.unwrap_err().downcast::<RuntimeError>().unwrap().code
    }

    #[test]
    fn retry_metadata_is_one_small_readback() {
        assert_eq!(std::mem::size_of::<RetryMetadata>(), 16);
        assert_eq!(std::mem::align_of::<RetryMetadata>(), 8);
    }

    #[test]
    fn retry_accepts_empty_underfull_and_exact_capacity_outputs() {
        for (rows, capacity) in [(0, 0), (0, 1024), (17, 1024), (1024, 1024)] {
            let metadata = RetryMetadata {
                rows,
                ..RetryMetadata::default()
            };
            assert_eq!(retry_capacity(metadata, capacity, None).unwrap(), None);
        }
    }

    #[test]
    fn retry_allocates_exact_total_including_zero_initial_capacity() {
        for (rows, capacity) in [(1, 0), (1025, 1024), (65_537, 1024), (u32::MAX as u64, 0)] {
            let metadata = RetryMetadata {
                rows,
                error: 4,
                dependency_error: 0,
            };
            assert_eq!(
                retry_capacity(metadata, capacity, None).unwrap(),
                Some(rows as usize),
            );
        }
    }

    #[test]
    fn retry_requires_a_consistent_current_capacity_error() {
        for (rows, error) in [(2048, 0), (1024, 4), (0, 4)] {
            let metadata = RetryMetadata {
                rows,
                error,
                dependency_error: 0,
            };
            assert_eq!(
                error_code(retry_capacity(metadata, 1024, None)),
                "CQ_GPU_OUTPUT_CAPACITY",
            );
        }
    }

    #[test]
    fn retry_preserves_every_non_capacity_output_error() {
        for (error, expected_code) in [
            (1, "CQ_GPU_OUTPUT_OVERFLOW"),
            (2, "CQ_GPU_EXPANSION_OVERFLOW"),
            (3, "CQ_GPU_SET_FULL"),
            (5, "CQ_GPU_OUTPUT_ROWS"),
            (6, "CQ_GPU_MEMBERSHIP_DOMAIN"),
            (7, "CQ_GPU_RUN_BUILD"),
            (99, "CQ_GPU_KERNEL_ERROR"),
        ] {
            for expected_rows in [None, Some(2048)] {
                let metadata = RetryMetadata {
                    rows: 2048,
                    error,
                    dependency_error: 0,
                };
                assert_eq!(
                    error_code(retry_capacity(metadata, 1024, expected_rows)),
                    expected_code,
                );
            }
        }
    }

    #[test]
    fn retry_rejects_every_dependency_error_before_output_capacity() {
        for (dependency_error, expected_code) in [
            (1, "CQ_GPU_OUTPUT_OVERFLOW"),
            (2, "CQ_GPU_EXPANSION_OVERFLOW"),
            (3, "CQ_GPU_SET_FULL"),
            (4, "CQ_GPU_OUTPUT_CAPACITY"),
            (5, "CQ_GPU_OUTPUT_ROWS"),
            (6, "CQ_GPU_MEMBERSHIP_DOMAIN"),
            (7, "CQ_GPU_RUN_BUILD"),
            (99, "CQ_GPU_KERNEL_ERROR"),
        ] {
            for error in [0, 4] {
                for expected_rows in [None, Some(2048)] {
                    let metadata = RetryMetadata {
                        rows: 2048,
                        error,
                        dependency_error,
                    };
                    assert_eq!(
                        error_code(retry_capacity(metadata, 1024, expected_rows)),
                        expected_code,
                    );
                }
            }
        }
    }

    #[test]
    fn retry_rejects_wrapped_counts_and_unsupported_row_totals() {
        let wrapped = RetryMetadata {
            rows: 0,
            error: 1,
            dependency_error: 0,
        };
        assert_eq!(
            error_code(retry_capacity(wrapped, 1024, None)),
            "CQ_GPU_OUTPUT_OVERFLOW",
        );
        for error in [0, 4, 5] {
            let metadata = RetryMetadata {
                rows: u32::MAX as u64 + 1,
                error,
                dependency_error: 0,
            };
            assert_eq!(
                error_code(retry_capacity(metadata, 1024, None)),
                "CQ_GPU_OUTPUT_ROWS",
            );
        }
        let maximum = RetryMetadata {
            rows: u32::MAX as u64,
            ..RetryMetadata::default()
        };
        assert_eq!(
            retry_capacity(maximum, u32::MAX as usize, None).unwrap(),
            None,
        );
    }

    #[test]
    fn retry_second_attempt_must_match_the_first_total() {
        let exact = RetryMetadata {
            rows: 2048,
            ..RetryMetadata::default()
        };
        assert_eq!(retry_capacity(exact, 2048, Some(2048)).unwrap(), None);
        for rows in [0, 1024, 2047] {
            let changed = RetryMetadata {
                rows,
                ..RetryMetadata::default()
            };
            assert_eq!(
                error_code(retry_capacity(changed, 2048, Some(2048))),
                "CQ_GPU_RETRY_COUNT",
            );
        }
    }

    #[test]
    fn retry_second_capacity_failure_never_retries_again() {
        let overflow = RetryMetadata {
            rows: 4096,
            error: 4,
            dependency_error: 0,
        };
        assert_eq!(
            error_code(retry_capacity(overflow, 2048, Some(2048))),
            "CQ_GPU_OUTPUT_CAPACITY",
        );
    }
}
