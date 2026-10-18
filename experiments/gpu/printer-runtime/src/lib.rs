//! Host/runtime contract for the Structures GPU printer.
//!
//! Each execution owns its slice/schema upload cache and string dictionary.
//! All device storage uses rccl-memory's pooled allocator; query algorithms
//! come from blocked-run, tiles and this crate, never RCCL device algorithms.
//! Generated launchers must be pure: an overflowing bounded output is counted
//! exactly, resized, and relaunched. Views are valid only while their execution
//! and its runtime remain alive, and all uses must be ordered on its stream.
mod device;
mod encoding;
mod kernels;

pub use device::*;
pub use encoding::{DecodeValue, Dictionary, EncodeValue, decode_value, encode_value};
pub use gpu_blocked_run as blocked_run;
pub use gpu_tiles as tiles;

use cuda_core::{CudaContext, CudaStream, DeviceCopy, LaunchConfig1D};
use gpu_blocked_run::{Buffer, MemoryResource, Run};
use rccl_memory::{
    ContiguousDeviceRange, CudaMemoryResource, PoolMemoryResource, StatisticsMemoryResource,
};
use std::{cell::Cell, collections::HashMap, fmt, marker::PhantomData, sync::Arc, time::Instant};

pub type Error = Box<dyn std::error::Error>;
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A concrete Structures canon provider. The printer, not a host collection,
/// interprets this recorded choice and invokes the exact packed device set.
pub struct DeviceSet<T>(PhantomData<T>);

/// Recorded prefix-run provider. Its abstraction law is a stable multiset:
/// full-key lookup equals the first integral key's blocked-run range filtered
/// by equality on every remaining recorded key. Multiplicity and source order
/// within that range are preserved. Generated residual checks are mandatory;
/// this marker never grants an unrecorded lookup or an implicit host fallback.
pub struct PrefixRun<K, V, C>(PhantomData<(K, V, C)>);

#[derive(Debug)]
pub struct RuntimeError {
    pub code: &'static str,
    pub detail: String,
}
impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
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
    /// GPU build + query kernels + exact device dedup; excludes transfers.
    pub total_ms: f64,
    /// Complete host execution interval, including encoding and transfers.
    pub wall_ms: f64,
}
thread_local! { static LAST_TIMINGS: Cell<Option<Timings>> = const { Cell::new(None) }; }
pub fn last_run_timings() -> Option<Timings> {
    LAST_TIMINGS.with(Cell::get)
}
pub fn take_last_timings() -> Option<Timings> {
    LAST_TIMINGS.with(Cell::take)
}

/// Resource owner, separate from Execution to avoid self-referential buffers.
/// Drop order releases pooled allocations before their stream and context.
pub struct Runtime {
    memory: MemoryResource,
    stream: Arc<CudaStream>,
    context: Arc<CudaContext>,
}
impl Runtime {
    pub fn new() -> Result<Self> {
        Self::with_pool(0, 0, 0)
    }

    /// A zero maximum permits pool growth; nonzero maximum bounds pool bytes.
    pub fn with_pool(device: usize, initial_bytes: usize, maximum_bytes: usize) -> Result<Self> {
        let context = CudaContext::new(device)?;
        let stream = context.default_stream();
        let memory = StatisticsMemoryResource::new(PoolMemoryResource::new(
            CudaMemoryResource::new(context.clone()),
            initial_bytes,
            maximum_bytes,
        )?);
        Ok(Self {
            memory,
            stream,
            context,
        })
    }
    pub fn context(&self) -> &Arc<CudaContext> {
        &self.context
    }
    pub fn stream(&self) -> &CudaStream {
        &self.stream
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
            timings: Timings::default(),
            started: Instant::now(),
            memberships: Vec::new(),
            membership_cache: HashMap::new(),
        }
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
struct Table<'runtime> {
    columns: Vec<Buffer<'runtime, i64>>,
    pointers: Buffer<'runtime, u64>,
    len: usize,
    bounds: Option<Vec<(i64, i64)>>,
}
impl Table<'_> {
    fn view(&self) -> TableView {
        TableView {
            columns: ptr(&self.pointers) as *const *const i64,
            len: self.len as u64,
        }
    }
}
struct BuiltRun<'runtime> {
    run: Run<'runtime>,
    pointers: Buffer<'runtime, u64>,
    len: u64,
}
struct Membership<'runtime> {
    bits: Buffer<'runtime, u64>,
    min_key: i64,
    max_key: i64,
}
struct Output<'runtime> {
    table: Table<'runtime>,
    count: Buffer<'runtime, u64>,
    error: Buffer<'runtime, u32>,
    capacity: usize,
}
impl Output<'_> {
    fn view(&self) -> OutputView {
        OutputView {
            columns: ptr(&self.table.pointers) as *const *mut i64,
            capacity: self.capacity as u64,
            count: ptr(&self.count),
            error: ptr(&self.error),
        }
    }
    fn count(&self) -> Result<usize> {
        let code = self.error.to_host_vec()?[0];
        let named = match code {
            0 => None,
            1 => Some((
                "CQ_GPU_OUTPUT_OVERFLOW",
                "logical output count exceeded u64",
            )),
            2 => Some((
                "CQ_GPU_EXPANSION_OVERFLOW",
                "tile expansion exceeded the u32 offset domain",
            )),
            3 => Some((
                "CQ_GPU_SET_FULL",
                "exact device set exhausted its bounded probe table",
            )),
            _ => Some((
                "CQ_GPU_KERNEL_ERROR",
                "generated kernel reported an unknown error code",
            )),
        };
        if let Some((name, detail)) = named {
            return Err(failure(name, detail));
        }
        let count = self.count.to_host_vec()?[0];
        if count > u32::MAX as u64 {
            return Err(failure(
                "CQ_GPU_OUTPUT_ROWS",
                "output cardinality exceeds u32::MAX",
            ));
        }
        Ok(count as usize)
    }
}

pub struct Execution<'runtime, 'data> {
    runtime: &'runtime Runtime,
    dictionary: Dictionary<'data>,
    tables: Vec<Table<'runtime>>,
    runs: Vec<BuiltRun<'runtime>>,
    uploads: HashMap<SliceKey, TableId>,
    run_cache: HashMap<(TableId, usize), RunId>,
    memberships: Vec<Membership<'runtime>>,
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
        let mut columns: Vec<Vec<i64>> =
            (0..width).map(|_| Vec::with_capacity(rows.len())).collect();
        let mut bounds = vec![(i64::MAX, i64::MIN); width];
        let mut encoded = vec![0; width];
        for row in rows {
            encode(row, &mut self.dictionary, &mut encoded)?;
            for (index, &value) in encoded.iter().enumerate() {
                columns[index].push(value);
                bounds[index].0 = bounds[index].0.min(value);
                bounds[index].1 = bounds[index].1.max(value);
            }
        }
        if rows.is_empty() {
            bounds.fill((0, -1));
        }
        let runtime = self.runtime;
        let start = runtime.stream.record_event(timing_flags())?;
        let device_columns = columns
            .iter()
            .map(|column| Buffer::from_host_in(&runtime.memory, &runtime.stream, column))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let pointers = pointer_array(runtime, &device_columns)?;
        let end = runtime.stream.record_event(timing_flags())?;
        end.synchronize()?;
        self.timings.upload_ms += start.elapsed_ms(&end)? as f64;
        let id = self.add_table(Table {
            columns: device_columns,
            pointers,
            len: rows.len(),
            bounds: Some(bounds),
        });
        self.uploads.insert(key, id);
        Ok(id)
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
            columns: ptr(&built.pointers) as *const *const i64,
            len: built.len,
        })
    }

    pub fn membership_view(&self, id: MembershipId) -> Result<MembershipView> {
        let membership = self
            .memberships
            .get(id.0)
            .ok_or_else(|| failure("CQ_GPU_MEMBERSHIP_ID", "unknown membership handle"))?;
        Ok(MembershipView {
            bits: ptr(&membership.bits),
            min_key: membership.min_key,
            max_key: membership.max_key,
        })
    }

    /// A recorded integral-key membership provider, not a payload-bearing run.
    /// The recorded provider's bitmap budget is explicit; no host/hash fallback.
    pub fn prepare_membership(
        &mut self,
        table: TableId,
        key_column: usize,
        budget_bytes: usize,
    ) -> Result<MembershipId> {
        let cache_key = (table, key_column, budget_bytes);
        if let Some(&id) = self.membership_cache.get(&cache_key) {
            return Ok(id);
        }
        let runtime = self.runtime;
        let start = runtime.stream.record_event(timing_flags())?;
        self.ensure_bounds(table)?;
        let source = self.table(table)?;
        let &(min_key, max_key) =
            source
                .bounds
                .as_ref()
                .unwrap()
                .get(key_column)
                .ok_or_else(|| {
                    failure(
                        "CQ_GPU_KEY_COLUMN",
                        "membership key is outside table schema",
                    )
                })?;
        let span = (max_key as i128 - min_key as i128 + 1).max(0) as u128;
        let words = span.div_ceil(64);
        if words * 8 > budget_bytes as u128 {
            return Err(failure(
                "CQ_GPU_MEMBERSHIP_DOMAIN",
                "recorded membership bitmap budget exceeded",
            ));
        }
        let bits = Buffer::<u64>::zeroed_in(&runtime.memory, &runtime.stream, words as usize)?;
        if source.len != 0 {
            let module = unsafe { kernels::runtime_kernels::load(&runtime.context)? };
            let launch = module.prepare_membership_insert(config(source.len, 0))?;
            unsafe {
                module.membership_insert(
                    &runtime.stream,
                    &launch,
                    source.view(),
                    key_column,
                    min_key,
                    ptr(&bits),
                )?;
            }
        }
        let end = runtime.stream.record_event(timing_flags())?;
        end.synchronize()?;
        self.timings.build_ms += start.elapsed_ms(&end)? as f64;
        let id = MembershipId(self.memberships.len());
        self.memberships.push(Membership {
            bits,
            min_key,
            max_key,
        });
        self.membership_cache.insert(cache_key, id);
        Ok(id)
    }

    /// Build exactly one recorded integral-key level. Payload order is preserved
    /// in RunView, even when the key is not the first source column.
    pub fn prepare_run(&mut self, table: TableId, key_column: usize) -> Result<RunId> {
        if let Some(&id) = self.run_cache.get(&(table, key_column)) {
            return Ok(id);
        }
        let runtime = self.runtime;
        let start = runtime.stream.record_event(timing_flags())?;
        self.ensure_bounds(table)?;
        let source = self.table(table)?;
        let key = source
            .columns
            .get(key_column)
            .ok_or_else(|| failure("CQ_GPU_KEY_COLUMN", "run key is outside the table schema"))?;
        let domain = source.bounds.as_ref().unwrap()[key_column];
        let span = (domain.1 as i128 - domain.0 as i128 + 1).max(0);
        if span > gpu_blocked_run::MAX_DOMAIN_KEYS as i128 {
            return Err(failure(
                "CQ_GPU_RUN_DOMAIN",
                "dense rank domain exceeds 2^32 key positions",
            ));
        }
        let payloads: Vec<&dyn ContiguousDeviceRange<i64>> = source
            .columns
            .iter()
            .enumerate()
            .filter_map(|(index, column)| {
                (index != key_column).then_some(column as &dyn ContiguousDeviceRange<i64>)
            })
            .collect();
        let active = if key.len() != source.len {
            Some(Buffer::from_host_in(
                &runtime.memory,
                &runtime.stream,
                &[source.len as u32],
            )?)
        } else {
            None
        };
        let run = Run::build(
            &runtime.context,
            &runtime.stream,
            &runtime.memory,
            key,
            &payloads,
            domain,
            active.as_ref(),
        )
        .map_err(|error| failure("CQ_GPU_RUN_BUILD", error.to_string()))?;
        let stats = gpu_blocked_run::read_stats(&runtime.memory, &runtime.stream, &[&run])
            .map_err(|error| failure("CQ_GPU_RUN_BUILD", error.to_string()))?;
        let mut pointers = Vec::with_capacity(source.columns.len());
        let mut payload = 0;
        for index in 0..source.columns.len() {
            if index == key_column {
                pointers.push(run.keys.cu_deviceptr());
            } else {
                pointers.push(run.payloads[payload].cu_deviceptr());
                payload += 1;
            }
        }
        let pointers = Buffer::from_host_in(&runtime.memory, &runtime.stream, &pointers)?;
        let end = runtime.stream.record_event(timing_flags())?;
        end.synchronize()?;
        self.timings.build_ms += start.elapsed_ms(&end)? as f64;
        let id = RunId(self.runs.len());
        self.runs.push(BuiltRun {
            run,
            pointers,
            len: stats[0].rows as u64,
        });
        self.run_cache.insert((table, key_column), id);
        Ok(id)
    }

    /// Every attempted row must increment OutputView.count, including rows
    /// beyond capacity. The callback is rerun only when the first allocation
    /// overflows; a different count on replay is a named determinism error.
    pub fn execute<F>(
        &mut self,
        width: usize,
        initial_capacity: usize,
        mut launch: F,
    ) -> Result<TableId>
    where
        F: FnMut(&CudaStream, OutputView) -> Result<()>,
    {
        check_rows(initial_capacity)?;
        let runtime = self.runtime;
        let mut capacity = initial_capacity;
        let mut expected = None;
        loop {
            let mut output = allocate_output(runtime, width, capacity)?;
            let start = runtime.stream.record_event(timing_flags())?;
            launch(&runtime.stream, output.view())?;
            let end = runtime.stream.record_event(timing_flags())?;
            end.synchronize()?;
            self.timings.kernel_ms += start.elapsed_ms(&end)? as f64;
            let count = output.count()?;
            if let Some(previous) = expected {
                if count != previous {
                    return Err(failure(
                        "CQ_GPU_NONDETERMINISTIC_OUTPUT",
                        "relaunch changed logical output cardinality",
                    ));
                }
            }
            if count <= capacity {
                output.table.len = count;
                return Ok(self.add_table(output.table));
            }
            expected = Some(count);
            capacity = count;
        }
    }

    /// Exact multiword packed keys. Field widths derive from checked extrema;
    /// a field may use all 64 bits and a tuple may use arbitrarily many words.
    /// CAS slots store immutable row IDs; equality compares every packed word,
    /// not hash fingerprints, so collisions and all-zero keys are harmless.
    pub fn dedup(&mut self, table: TableId) -> Result<TableId> {
        let runtime = self.runtime;
        let start = runtime.stream.record_event(timing_flags())?;
        self.ensure_bounds(table)?;
        let source = self.table(table)?;
        if source.len < 2 {
            return Ok(table);
        }
        let mut fields = Vec::with_capacity(source.columns.len());
        let mut bits = 0u64;
        for &(min, max) in source.bounds.as_ref().unwrap() {
            let range = (max as u64).wrapping_sub(min as u64);
            let width = 64 - range.leading_zeros();
            fields.push(kernels::PackedField {
                min,
                bit_offset: bits,
                width,
                padding: 0,
            });
            bits = bits.checked_add(width as u64).ok_or_else(|| {
                failure("CQ_GPU_PACKED_KEY_SIZE", "packed key bit count overflow")
            })?;
        }
        let words = usize::try_from(bits.div_ceil(64).max(1))
            .map_err(|_| failure("CQ_GPU_PACKED_KEY_SIZE", "packed key exceeds address space"))?;
        let packed_len = words
            .checked_mul(source.len)
            .ok_or_else(|| failure("CQ_GPU_PACKED_KEY_SIZE", "packed rows exceed address space"))?;
        let capacity = source
            .len
            .checked_mul(2)
            .and_then(usize::checked_next_power_of_two)
            .ok_or_else(|| failure("CQ_GPU_SET_CAPACITY", "device set capacity overflow"))?;
        let packed = Buffer::<u64>::zeroed_in(&runtime.memory, &runtime.stream, packed_len)?;
        let slots = Buffer::<u64>::zeroed_in(&runtime.memory, &runtime.stream, capacity)?;
        let fields_device = Buffer::from_host_in(&runtime.memory, &runtime.stream, &fields)?;
        let mut output = allocate_output(runtime, source.columns.len(), source.len)?;
        let module = unsafe { kernels::runtime_kernels::load(&runtime.context)? };
        let pack = module.prepare_pack(config(source.len, 0))?;
        let insert = module.prepare_set_insert(config(source.len, 0))?;
        unsafe {
            module.pack(
                &runtime.stream,
                &pack,
                source.view(),
                ptr(&fields_device),
                fields.len(),
                ptr(&packed),
                words,
            )?;
            module.set_insert(
                &runtime.stream,
                &insert,
                source.view(),
                source.columns.len(),
                ptr(&packed),
                words,
                ptr(&slots),
                capacity,
                output.view(),
            )?;
        }
        let end = runtime.stream.record_event(timing_flags())?;
        end.synchronize()?;
        output.table.len = output.count()?;
        output.table.bounds = source.bounds.clone();
        self.timings.dedup_ms += start.elapsed_ms(&end)? as f64;
        Ok(self.add_table(output.table))
    }

    /// Download initialized logical prefixes only, with no device-side copies.
    pub fn download(&mut self, table: TableId) -> Result<Vec<Vec<i64>>> {
        let runtime = self.runtime;
        let source = self.table(table)?;
        let start = runtime.stream.record_event(timing_flags())?;
        let mut columns = Vec::with_capacity(source.columns.len());
        for column in &source.columns {
            let mut host: Vec<i64> = Vec::with_capacity(source.len);
            if source.len != 0 {
                unsafe {
                    cuda_core::memory::memcpy_dtoh_async(
                        host.as_mut_ptr(),
                        column.cu_deviceptr(),
                        source.len * core::mem::size_of::<i64>(),
                        runtime.stream.cu_stream(),
                    )?;
                }
                runtime.stream.synchronize()?;
                // SAFETY: the completed copy initialized precisely this prefix.
                unsafe {
                    host.set_len(source.len);
                }
            }
            columns.push(host);
        }
        let end = runtime.stream.record_event(timing_flags())?;
        end.synchronize()?;
        self.timings.download_ms += start.elapsed_ms(&end)? as f64;
        Ok(columns)
    }

    pub fn frontier(&self, slots: u32, descriptor_words: u32) -> Result<Frontier<'runtime>> {
        Frontier::new(self.runtime, slots, descriptor_words)
    }

    pub fn finish(&mut self) -> Result<Timings> {
        self.runtime.stream.synchronize()?;
        self.timings.total_ms =
            self.timings.build_ms + self.timings.kernel_ms + self.timings.dedup_ms;
        self.timings.wall_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        LAST_TIMINGS.with(|last| last.set(Some(self.timings)));
        Ok(self.timings)
    }

    fn table(&self, id: TableId) -> Result<&Table<'runtime>> {
        self.tables
            .get(id.0)
            .ok_or_else(|| failure("CQ_GPU_TABLE_ID", "unknown table handle"))
    }
    fn add_table(&mut self, table: Table<'runtime>) -> TableId {
        let id = TableId(self.tables.len());
        self.tables.push(table);
        id
    }
    fn ensure_bounds(&mut self, table: TableId) -> Result<()> {
        let source = self.table(table)?;
        if source.bounds.is_some() {
            return Ok(());
        }
        let width = source.columns.len();
        if source.len == 0 || width == 0 {
            self.tables[table.0].bounds = Some(vec![(0, -1); width]);
            return Ok(());
        }
        let runtime = self.runtime;
        let initial: Vec<u64> = (0..width).flat_map(|_| [u64::MAX, 0]).collect();
        let extrema = Buffer::from_host_in(&runtime.memory, &runtime.stream, &initial)?;
        let module = unsafe { kernels::runtime_kernels::load(&runtime.context)? };
        let launch = module.prepare_extrema(config(source.len, 4096))?;
        unsafe {
            module.extrema(
                &runtime.stream,
                &launch,
                source.view(),
                width,
                ptr(&extrema),
            )?;
        }
        let extrema = extrema.to_host_vec()?;
        self.tables[table.0].bounds = Some(
            extrema
                .chunks_exact(2)
                .map(|pair| {
                    (
                        (pair[0] ^ (1u64 << 63)) as i64,
                        (pair[1] ^ (1u64 << 63)) as i64,
                    )
                })
                .collect(),
        );
        Ok(())
    }
}

/// Bounded continuation storage using the triangles descriptor lock/copy
/// protocol. Each callback must reset it, including output-growth replays.
pub struct Frontier<'runtime> {
    runtime: &'runtime Runtime,
    pool: Buffer<'runtime, u32>,
    counters: Buffer<'runtime, u64>,
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
            pool: Buffer::zeroed_in(&runtime.memory, &runtime.stream, words)?,
            counters: Buffer::zeroed_in(&runtime.memory, &runtime.stream, 2)?,
            slots,
            descriptor_words,
        })
    }
    pub fn view(&self) -> FrontierView {
        FrontierView {
            pool: ptr(&self.pool),
            slots: self.slots,
            descriptor_words: self.descriptor_words,
            cursor: ptr(&self.counters),
            remaining: unsafe { ptr(&self.counters).add(1) },
        }
    }
    pub fn reset(&self, root_tiles: u64) -> Result<()> {
        let stream = &self.runtime.stream;
        if !self.pool.is_empty() {
            unsafe {
                cuda_core::memory::memset_d8_async(
                    self.pool.cu_deviceptr(),
                    0,
                    self.pool.num_bytes(),
                    stream.cu_stream(),
                )?;
            }
        }
        let module = unsafe { kernels::runtime_kernels::load(&self.runtime.context)? };
        let launch = module.prepare_frontier_initialize(LaunchConfig1D::new(1, 256, 0))?;
        unsafe {
            module.frontier_initialize(stream, &launch, self.view(), root_tiles)?;
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
fn ptr<T: DeviceCopy>(buffer: &Buffer<'_, T>) -> *mut T {
    buffer.cu_deviceptr() as *mut T
}
fn timing_flags() -> Option<cuda_core::sys::CUevent_flags> {
    Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT)
}
fn config(rows: usize, shared: u32) -> LaunchConfig1D {
    LaunchConfig1D::new(rows.div_ceil(256).clamp(1, 568) as u32, 256, shared)
}
fn pointer_array<'runtime>(
    runtime: &'runtime Runtime,
    columns: &[Buffer<'runtime, i64>],
) -> Result<Buffer<'runtime, u64>> {
    let pointers: Vec<u64> = columns
        .iter()
        .map(ContiguousDeviceRange::cu_deviceptr)
        .collect();
    Ok(Buffer::from_host_in(
        &runtime.memory,
        &runtime.stream,
        &pointers,
    )?)
}
fn allocate_output<'runtime>(
    runtime: &'runtime Runtime,
    width: usize,
    capacity: usize,
) -> Result<Output<'runtime>> {
    let columns = (0..width)
        .map(|_| Buffer::<i64>::zeroed_in(&runtime.memory, &runtime.stream, capacity))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let pointers = pointer_array(runtime, &columns)?;
    Ok(Output {
        table: Table {
            columns,
            pointers,
            len: 0,
            bounds: None,
        },
        count: Buffer::zeroed_in(&runtime.memory, &runtime.stream, 1)?,
        error: Buffer::zeroed_in(&runtime.memory, &runtime.stream, 1)?,
        capacity,
    })
}
