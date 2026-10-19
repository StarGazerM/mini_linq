//! Resident execution for the Structures GPU printer.
//!
//! Rust values retain pooled allocations, dictionaries and device status. Fixed
//! capacity execution transfers no rows, counts, bounds or pointer arrays.
//! General growth reads one four-byte suspension control word per attempt and
//! resumes saved work without replay. Explicit `to_host` validates all retained
//! ancestor statuses before decoding; capacities are never logical row counts.
//! Dense runs consume conservative loader/projection bounds and reject domains
//! outside their recorded representation rather than downloading extrema.
mod device;
mod kernels;

use cuda_core::{CudaContext, CudaStream, LaunchConfig1D};
pub use device::*;
pub use gpu_blocked_run as blocked_run;
use gpu_blocked_run::{Buffer as Scratch, Run};
pub use gpu_device_columns::{
    decode_value, detect_integer_codec, encode_value, Buffer, Codec, ColumnView, DecodeValue,
    Delta, DeviceColumns, Dictionary, DictionaryCodec, EncodeValue, EncodedColumn, For,
    OwnedRowCodec, Plain, Pool, Representation, RowCodec,
};
use gpu_device_columns::{OutputCount, Table};
pub use gpu_tiles as tiles;
use rccl_memory::ContiguousDeviceRange;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fmt,
    marker::PhantomData,
    sync::Arc,
    time::Instant,
};

pub type Error = Box<dyn std::error::Error>;
pub type Result<T, E = Error> = std::result::Result<T, E>;
pub struct DeviceSet<T, C = Plain>(PhantomData<(T, C)>);
pub struct PrefixRun<K, V, C, const PREFIX: usize>(PhantomData<(K, V, C)>);

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
#[derive(Clone, Copy, Debug, Default)]
pub struct Timings {
    pub upload_ms: f64,
    pub build_ms: f64,
    pub kernel_ms: f64,
    pub dedup_ms: f64,
    pub download_ms: f64,
    pub total_ms: f64,
    pub wall_ms: f64,
}
thread_local! { static LAST_TIMINGS: Cell<Option<Timings>> = const { Cell::new(None) }; }
pub fn last_run_timings() -> Option<Timings> {
    LAST_TIMINGS.with(Cell::get)
}
pub fn take_last_timings() -> Option<Timings> {
    LAST_TIMINGS.with(Cell::take)
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

pub struct Runtime {
    kernels: kernels::runtime_kernels::LoadedModule,
    pool: Arc<Pool>,
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
        Ok(Self { kernels, pool })
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
            timings: Timings::default(),
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
        let bounds = host
            .iter()
            .map(|values| {
                (
                    *values.iter().min().unwrap_or(&0),
                    *values.iter().max().unwrap_or(&-1),
                )
            })
            .collect();
        let metadata = self.metadata(rows)?;
        let views = self.column_views(&columns)?;
        self.stream().synchronize()?;
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
        let metadata = Arc::new(Buffer::zeroed(self.pool.clone(), 1)?);
        let launch = self.kernels.prepare_initialize_count(config(1, 0))?;
        unsafe {
            self.kernels
                .initialize_count(self.stream(), &launch, metadata.ptr(), rows as u64)?;
        }
        Ok(metadata)
    }
    fn column_views(&self, columns: &[Arc<EncodedColumn>]) -> Result<Arc<Buffer<ColumnView>>> {
        let views = Arc::new(Buffer::zeroed(self.pool.clone(), columns.len())?);
        let launch = self.kernels.prepare_initialize_view(config(1, 0))?;
        for (index, column) in columns.iter().enumerate() {
            unsafe {
                self.kernels.initialize_view(
                    self.stream(),
                    &launch,
                    views.ptr(),
                    index,
                    column.view(),
                )?;
            }
        }
        Ok(views)
    }
}
thread_local! { static SHARED_RUNTIME: RefCell<Option<Runtime>> = const { RefCell::new(None) }; }
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
        execute(&Runtime::from_pool(pool)?)
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
}
struct Membership {
    bits: Buffer<u64>,
    bounds: Buffer<u64>,
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
pub struct Execution<'runtime, 'data> {
    runtime: &'runtime Runtime,
    dictionary: Dictionary<'data>,
    tables: Vec<Table>,
    runs: Vec<BuiltRun<'runtime>>,
    uploads: HashMap<SliceKey, TableId>,
    run_cache: HashMap<(TableId, usize), RunId>,
    memberships: Vec<Membership>,
    membership_cache: HashMap<(TableId, usize, usize), MembershipId>,
    timings: Timings,
    started: Instant,
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
        let start = self.runtime.stream().record_event(timing_flags())?;
        let columns = host
            .iter()
            .map(|values| {
                EncodedColumn::from_host(self.runtime.pool.clone(), values, Codec::Plain)
                    .map(Arc::new)
            })
            .collect::<Result<Vec<_>>>()?;
        let bounds = host
            .iter()
            .map(|values| {
                (
                    *values.iter().min().unwrap_or(&0),
                    *values.iter().max().unwrap_or(&-1),
                )
            })
            .collect();
        let metadata = self.runtime.metadata(rows.len())?;
        let views = self.runtime.column_views(&columns)?;
        self.timings.upload_ms += elapsed(self.runtime, start)?;
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
        let source = self.table(table)?;
        if key_column >= source.columns.len() {
            return Err(failure(
                "CQ_GPU_KEY_COLUMN",
                "membership key outside schema",
            ));
        }
        let start = self.runtime.stream().record_event(timing_flags())?;
        let bounds = Buffer::zeroed(self.runtime.pool.clone(), 2)?;
        let status = self.runtime.metadata(0)?;
        let launch = self
            .runtime
            .kernels
            .prepare_membership_bounds(config(1, 0))?;
        unsafe {
            self.runtime.kernels.membership_bounds(
                self.runtime.stream(),
                &launch,
                source.view(),
                key_column,
                bounds.ptr(),
                (budget_bytes / 8) as u64 * 64,
                status.ptr(),
            )?;
        }
        let bits = Buffer::zeroed(self.runtime.pool.clone(), budget_bytes / 8)?;
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
        self.timings.build_ms += elapsed(self.runtime, start)?;
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
        let source = self.table(table)?;
        let domain = *source
            .bounds
            .get(key_column)
            .ok_or_else(|| failure("CQ_GPU_KEY_COLUMN", "run key outside schema"))?;
        if (domain.1 as i128 - domain.0 as i128 + 1).max(0)
            > gpu_blocked_run::MAX_DOMAIN_KEYS as i128
        {
            return Err(failure(
                "CQ_GPU_RUN_DOMAIN",
                "dense rank domain exceeds 2^32 key positions",
            ));
        }
        let runtime = self.runtime;
        let start = runtime.stream().record_event(timing_flags())?;
        let columns = decode_columns(runtime, source)?;
        let count = Scratch::<u32>::zeroed_in(runtime.pool.resource(), runtime.stream(), 1)?;
        let launch = runtime.kernels.prepare_count_u32(config(1, 0))?;
        unsafe {
            runtime.kernels.count_u32(
                runtime.stream(),
                &launch,
                source.view(),
                count.cu_deviceptr() as *mut u32,
            )?;
        }
        let payloads: Vec<&dyn ContiguousDeviceRange<i64>> = columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| {
                (index != key_column).then_some(column as &dyn ContiguousDeviceRange<i64>)
            })
            .collect();
        let run = Run::build(
            runtime.context(),
            runtime.stream(),
            runtime.pool.resource(),
            &columns[key_column],
            &payloads,
            domain,
            Some(&count),
        )
        .map_err(|e| failure("CQ_GPU_RUN_BUILD", e.to_string()))?;
        let pointers = Buffer::zeroed(runtime.pool.clone(), columns.len())?;
        let launch = runtime.kernels.prepare_initialize_pointer(config(1, 0))?;
        let mut payload = 0;
        for index in 0..columns.len() {
            let pointer = if index == key_column {
                run.keys.cu_deviceptr()
            } else {
                let value = run.payloads[payload].cu_deviceptr();
                payload += 1;
                value
            };
            unsafe {
                runtime.kernels.initialize_pointer(
                    runtime.stream(),
                    &launch,
                    pointers.ptr(),
                    index,
                    pointer,
                )?;
            }
        }
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
        self.timings.build_ms += elapsed(runtime, start)?;
        self.tables[table.0].statuses.push(status);
        let id = RunId(self.runs.len());
        self.runs.push(BuiltRun { run, pointers, len });
        self.run_cache.insert((table, key_column), id);
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
        let start = runtime.stream().record_event(timing_flags())?;
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
        let table = self.complete_output(output, bounds, Codec::Plain)?;
        self.timings.build_ms += elapsed(runtime, start)?;
        Ok(self.add_table(table))
    }
    pub fn execute_bounded<F>(
        &mut self,
        width: usize,
        capacity: usize,
        mut launch: F,
    ) -> Result<TableId>
    where
        F: FnMut(&CudaStream, OutputView) -> Result<()>,
    {
        check_rows(capacity)?;
        let start = self.runtime.stream().record_event(timing_flags())?;
        let output = allocate_output(self.runtime, width, capacity)?;
        launch(self.runtime.stream(), output.view())?;
        let table =
            self.complete_output(output, vec![(i64::MIN, i64::MAX); width], Codec::Plain)?;
        self.timings.kernel_ms += elapsed(self.runtime, start)?;
        Ok(self.add_table(table))
    }
    /// Resume unfinished work; only one u32 control word crosses per attempt.
    pub fn execute_resumable<F>(
        &mut self,
        width: usize,
        initial_capacity: usize,
        state_words: usize,
        mut launch: F,
    ) -> Result<TableId>
    where
        F: FnMut(&CudaStream, OutputView, ResumeView) -> Result<()>,
    {
        check_rows(initial_capacity)?;
        let runtime = self.runtime;
        let start = runtime.stream().record_event(timing_flags())?;
        let state = Buffer::<u64>::zeroed(runtime.pool.clone(), state_words)?;
        let control = Buffer::<u32>::zeroed(runtime.pool.clone(), 1)?;
        let resume = ResumeView {
            state: state.ptr(),
            control: control.ptr(),
        };
        let mut output = allocate_output(runtime, width, initial_capacity.max(1))?;
        loop {
            unsafe {
                cuda_core::memory::memset_d8_async(
                    control.ptr() as u64,
                    0,
                    4,
                    runtime.stream().cu_stream(),
                )?;
            }
            launch(runtime.stream(), output.view(), resume)?;
            if control.to_host()?[0] == 0 {
                break;
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
            let mut grown = allocate_output(runtime, width, capacity)?;
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
        }
        let table =
            self.complete_output(output, vec![(i64::MIN, i64::MAX); width], Codec::Plain)?;
        self.timings.kernel_ms += elapsed(runtime, start)?;
        Ok(self.add_table(table))
    }
    fn complete_output(
        &self,
        output: Output,
        bounds: Vec<(i64, i64)>,
        codec: Codec,
    ) -> Result<Table> {
        let runtime = self.runtime;
        let check = runtime.kernels.prepare_validate_output(config(1, 0))?;
        unsafe {
            runtime
                .kernels
                .validate_output(runtime.stream(), &check, output.view())?;
        }
        let columns = output
            .columns
            .iter()
            .zip(&bounds)
            .map(|(column, &bound)| unsafe {
                EncodedColumn::from_device(
                    runtime.pool.clone(),
                    column.ptr(),
                    output.capacity,
                    output.metadata.ptr().cast(),
                    codec,
                    bound,
                )
                .map(Arc::new)
            })
            .collect::<Result<Vec<_>>>()?;
        let views = runtime.column_views(&columns)?;
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
        let source = self.table(table)?;
        if source.capacity < 2 {
            return Ok(table);
        }
        let runtime = self.runtime;
        let start = runtime.stream().record_event(timing_flags())?;
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
        let packed = Buffer::zeroed(runtime.pool.clone(), packed_len)?;
        let slots = Buffer::zeroed(runtime.pool.clone(), capacity)?;
        let output = allocate_output(runtime, width, source.capacity)?;
        let pack = runtime.kernels.prepare_pack(config(source.capacity, 0))?;
        let insert = runtime
            .kernels
            .prepare_set_insert(config(source.capacity, 0))?;
        unsafe {
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
            )?;
        }
        let bounds = source.bounds.clone();
        let result = self.complete_output(output, bounds, Codec::Plain)?;
        self.timings.dedup_ms += elapsed(runtime, start)?;
        Ok(self.add_table(result))
    }
    /// Only a recorded derived provider selects this codec.
    pub fn reencode_table(&mut self, source: TableId, codec: Codec) -> Result<TableId> {
        let table = self.table(source)?;
        if table.columns.iter().all(|column| column.codec() == codec) {
            return Ok(source);
        }
        let start = self.runtime.stream().record_event(timing_flags())?;
        let decoded = decode_columns(self.runtime, table)?;
        let columns = decoded
            .iter()
            .zip(&table.bounds)
            .map(|(column, &bound)| unsafe {
                EncodedColumn::from_device(
                    self.runtime.pool.clone(),
                    column.ptr(),
                    table.capacity,
                    table.metadata.ptr().cast(),
                    codec,
                    bound,
                )
                .map(Arc::new)
            })
            .collect::<Result<Vec<_>>>()?;
        let table = Table {
            views: self.runtime.column_views(&columns)?,
            columns,
            ..table.clone()
        };
        self.timings.build_ms += elapsed(self.runtime, start)?;
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
    pub fn finish(&mut self) -> Result<Timings> {
        self.runtime.stream().synchronize()?;
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
            counters: Buffer::zeroed(runtime.pool.clone(), 2)?,
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
        unsafe {
            cuda_core::memory::memset_d8_async(
                self.pool.ptr() as u64,
                0,
                self.pool.len() * 4,
                self.runtime.stream().cu_stream(),
            )?;
        }
        let launch = self
            .runtime
            .kernels
            .prepare_frontier_initialize(config(1, 0))?;
        unsafe {
            self.runtime.kernels.frontier_initialize(
                self.runtime.stream(),
                &launch,
                self.view(),
                root_tiles,
            )?;
        }
        Ok(())
    }
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
fn timing_flags() -> Option<cuda_core::sys::CUevent_flags> {
    Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT)
}
fn config(rows: usize, shared: u32) -> LaunchConfig1D {
    LaunchConfig1D::new(rows.div_ceil(256).clamp(1, 568) as u32, 256, shared)
}
fn elapsed(runtime: &Runtime, start: cuda_core::CudaEvent) -> Result<f64> {
    let end = runtime.stream().record_event(timing_flags())?;
    end.synchronize()?;
    Ok(start.elapsed_ms(&end)? as f64)
}
fn allocate_output(runtime: &Runtime, width: usize, capacity: usize) -> Result<Output> {
    let columns = (0..width)
        .map(|_| Buffer::zeroed(runtime.pool.clone(), capacity))
        .collect::<Result<Vec<_>>>()?;
    let pointers = Buffer::zeroed(runtime.pool.clone(), width)?;
    let launch = runtime.kernels.prepare_initialize_pointer(config(1, 0))?;
    for (index, column) in columns.iter().enumerate() {
        unsafe {
            runtime.kernels.initialize_pointer(
                runtime.stream(),
                &launch,
                pointers.ptr(),
                index,
                column.ptr() as u64,
            )?;
        }
    }
    Ok(Output {
        columns,
        pointers,
        metadata: runtime.metadata(0)?,
        capacity,
    })
}
fn decode_columns(runtime: &Runtime, table: &Table) -> Result<Vec<Buffer<i64>>> {
    let launch = runtime
        .kernels
        .prepare_decode_column(config(table.capacity, 0))?;
    table
        .columns
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let output = Buffer::zeroed(runtime.pool.clone(), table.capacity)?;
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
        })
        .collect()
}
