//! Same-input construction controls; no production allocator or scheduler changes.
#[allow(dead_code)]
#[path = "../common.rs"]
mod common;
#[allow(dead_code)]
#[path = "../sort.rs"]
mod sort;

use common::{Gpu, Row, view};
use mccl::relation::{IndexSpec, Payload, Relation};
use metal::objc::{msg_send, sel, sel_impl};
use metal::{
    Buffer, BufferRef, ComputeCommandEncoderRef, ComputePipelineState, ComputePipelineStateRef,
    MTLCommandBufferStatus, MTLDispatchType, MTLSize,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    error::Error,
    ffi::{CStr, c_char, c_int, c_void},
    fs,
    hint::black_box,
    mem::MaybeUninit,
    path::{Path, PathBuf},
    ptr,
    time::Instant,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const DIRECTORY: usize = 4097;
const VARIANTS: [&str; 7] = [
    "raw_three_buffer_per_stage",
    "raw_three_buffer_single_concurrent_barriers",
    "raw_three_buffer_single_serial",
    "mccl_public_shared",
    "mccl_c_shared",
    "mccl_buffer_sort_directory",
    "raw_copy_rows",
];

// Pinned public C ABI from mccl.h and mlx/c/{array,stream}.h; not private MLX internals.
mod ffi {
    use super::*;
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct Handle {
        pub ctx: *mut c_void,
    }
    unsafe extern "C" {
        pub fn mlx_array_new() -> Handle;
        pub fn mlx_array_free(a: Handle) -> c_int;
        pub fn mlx_array_eval(a: Handle) -> c_int;
        pub fn mlx_array_data_uint32(a: Handle) -> *const u32;
        pub fn mlx_default_gpu_stream_new() -> Handle;
        pub fn mlx_stream_free(s: Handle) -> c_int;
        pub fn mccl_allocate_u32(
            out: *mut Handle,
            shape: *const c_int,
            ndim: c_int,
            data: *mut *mut u32,
            s: Handle,
        ) -> c_int;
        pub fn mccl_sort_pairs_u32(out: *mut Handle, input: Handle, s: Handle) -> c_int;
        pub fn mccl_radix_directory_u32(out: *mut Handle, input: Handle, s: Handle) -> c_int;
        pub fn mccl_build_relation_index_u32(
            rows: *mut Handle,
            directory: *mut Handle,
            count: *mut usize,
            relation: Handle,
            key_column: c_int,
            payload_column: c_int,
            selection: Handle,
            stream: Handle,
        ) -> c_int;
        pub fn mccl_last_error() -> *const c_char;
    }
}
struct Array(ffi::Handle);
impl Array {
    fn new() -> Self {
        Self(unsafe { ffi::mlx_array_new() })
    }
    fn eval(&self) -> Result<()> {
        let status = unsafe { ffi::mlx_array_eval(self.0) };
        if status != 0 {
            return Err(format!("mlx_array_eval failed: status {status}").into());
        }
        Ok(())
    }
    fn data<T>(&self, len: usize) -> Result<&[T]> {
        let p = unsafe { ffi::mlx_array_data_uint32(self.0) }.cast::<T>();
        if p.is_null() {
            return Err("MLX returned a null nonempty mapping".into());
        }
        Ok(unsafe { std::slice::from_raw_parts(p, len) })
    }
}
impl Drop for Array {
    fn drop(&mut self) {
        unsafe { ffi::mlx_array_free(self.0) };
    }
}
struct Stream(ffi::Handle);
impl Stream {
    fn new() -> Result<Self> {
        let s = unsafe { ffi::mlx_default_gpu_stream_new() };
        if s.ctx.is_null() {
            return Err("MLX GPU stream unavailable".into());
        }
        Ok(Self(s))
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        unsafe { ffi::mlx_stream_free(self.0) };
    }
}
fn native(status: c_int) -> Result<()> {
    if status == 0 {
        return Ok(());
    }
    let error = unsafe { ffi::mccl_last_error() };
    if error.is_null() {
        return Err(format!("MCCL C ABI status {status}").into());
    }
    Err(unsafe { CStr::from_ptr(error) }
        .to_string_lossy()
        .into_owned()
        .into())
}

struct Stamp {
    wall: Instant,
    usage: libc::rusage,
}
impl Stamp {
    fn start() -> Result<Self> {
        Ok(Self {
            usage: usage()?,
            wall: Instant::now(),
        })
    }
    fn finish(self) -> Result<Value> {
        let wall_ms = self.wall.elapsed().as_secs_f64() * 1000.0;
        let after = usage()?;
        let time = |t: libc::timeval| t.tv_sec as f64 * 1000.0 + t.tv_usec as f64 / 1000.0;
        Ok(json!({"wall_ms":wall_ms,
            "cpu_user_ms":time(after.ru_utime)-time(self.usage.ru_utime),
            "cpu_system_ms":time(after.ru_stime)-time(self.usage.ru_stime),
            "minor_faults":after.ru_minflt-self.usage.ru_minflt,
            "major_faults":after.ru_majflt-self.usage.ru_majflt,
            "voluntary_context_switches":after.ru_nvcsw-self.usage.ru_nvcsw,
            "involuntary_context_switches":after.ru_nivcsw-self.usage.ru_nivcsw}))
    }
}
fn usage() -> Result<libc::rusage> {
    let mut u = MaybeUninit::uninit();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, u.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { u.assume_init() })
}
fn phase<T>(f: impl FnOnce() -> Result<T>) -> Result<(T, Value)> {
    let start = Stamp::start()?;
    let value = f()?;
    Ok((value, start.finish()?))
}
fn memory() -> Result<Value> {
    let s = mccl::memory::stats()?;
    Ok(
        json!({"active_bytes":s.active_bytes,"cached_bytes":s.cached_bytes,"peak_bytes":s.peak_bytes}),
    )
}
fn throughput(phase: &mut Value, bytes: usize) {
    let ms = phase["wall_ms"].as_f64().unwrap();
    if ms > 0.0 {
        phase["payload_gb_s"] = json!(bytes as f64 / (ms * 1e6));
    }
}

fn source(n: usize, distribution: &str) -> Vec<(u32, u32)> {
    let mut state = 0x83a6237cu32;
    (0..n)
        .map(|i| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let key = match distribution {
                "uniform" => state,
                // Exact full-width repeated-key fixture used by the historical
                // workspace and the migrated pool constructor comparisons.
                "workspace" => ((i as u32).wrapping_mul(40503) & 0xffff) * 65537,
                "dense" => ((i as u64 * 13 + 0x83a6237c) % (n / 32).max(1) as u64) as u32 + 1,
                "all_equal" => 0x83a6237c,
                _ => unreachable!(),
            };
            (key, i as u32)
        })
        .collect()
}
// Same tuple-to-packed-row loop as MCCL's public constructor. No tuple layout cast.
unsafe fn populate(dst: *mut Row, source: &[(u32, u32)]) {
    for (i, &(key, value)) in source.iter().enumerate() {
        unsafe { dst.add(i).write(Row { key, value }) };
    }
}
fn first_scan(rows: &[Row], directory: Option<&[u32]>) -> Result<Value> {
    let (checksum, mut measurement) = phase(|| {
        let sum = rows.iter().fold(0u64, |s, r| {
            s.wrapping_add((u64::from(r.key) << 32) | u64::from(r.value))
        });
        let sum = directory.map_or(sum, |d| {
            d.iter().fold(sum, |s, &v| s.wrapping_add(u64::from(v)))
        });
        Ok(black_box(sum))
    })?;
    measurement["checksum"] = json!(checksum);
    throughput(
        &mut measurement,
        std::mem::size_of_val(rows) + directory.map_or(0, std::mem::size_of_val),
    );
    Ok(measurement)
}
fn probes(expected: &[Row]) -> Vec<u32> {
    let mut keys: Vec<u32> = (0..4096).map(|p| p << 20).collect();
    keys.extend([0, u32::MAX]);
    for i in 0..64 {
        let key = expected[(expected.len() - 1) * i / 63].key;
        keys.extend([key, key.wrapping_add(1), key.wrapping_sub(1)]);
    }
    keys.sort_unstable();
    keys.dedup();
    keys
}
fn bounds(rows: &[Row], key: u32) -> std::ops::Range<usize> {
    let start = rows.partition_point(|r| r.key < key);
    start..start + rows[start..].partition_point(|r| r.key == key)
}
fn validate(rows: &[Row], directory: &[u32], expected: &[Row], keys: &[u32]) {
    assert_eq!(rows.len(), expected.len());
    assert!(rows == expected, "complete stable row output mismatch");
    assert_eq!(directory.len(), DIRECTORY);
    for (p, &offset) in directory.iter().enumerate() {
        let want = if p == 4096 {
            rows.len()
        } else {
            expected.partition_point(|r| r.key < (p as u32) << 20)
        };
        assert_eq!(offset as usize, want, "directory prefix {p}");
    }
    for &key in keys {
        let p = (key >> 20) as usize;
        let bucket = &rows[directory[p] as usize..directory[p + 1] as usize];
        assert!(
            bucket[bounds(bucket, key)] == expected[bounds(expected, key)],
            "directory lookup mismatch"
        );
    }
}
fn preserved(rows: &[Row], source: &[(u32, u32)]) {
    assert!(
        rows.iter()
            .zip(source)
            .all(|(r, &(k, v))| r.key == k && r.value == v),
        "immutable input modified"
    );
}

struct RawPool {
    input: Buffer,
    output: Buffer,
    directory: Buffer,
    sort: sort::Sort,
    directory_pipeline: ComputePipelineState,
    copy: ComputePipelineState,
}
impl RawPool {
    fn new(gpu: &Gpu, n: usize) -> Self {
        Self {
            input: gpu.allocate_buffer(n * 8, false),
            output: gpu.allocate_buffer(n * 8, false),
            directory: gpu.allocate_buffer(DIRECTORY * 4, false),
            sort: sort::Sort::with_uninitialized_scratch(gpu, n, 8, 8),
            directory_pipeline: gpu.pipeline("maintain_directory"),
            copy: gpu.pipeline("copy_rows"),
        }
    }
    fn output(&self) -> &BufferRef {
        &self.output
    }
    fn bytes(&self) -> u64 {
        self.input.length()
            + self.output.length()
            + self.directory.length()
            + self.sort.scratch_buffers().map(|b| b.length()).sum::<u64>()
    }
}
fn single_dispatch(
    enc: &ComputeCommandEncoderRef,
    pipeline: &ComputePipelineStateRef,
    buffers: &[&BufferRef],
    constants: &[u32],
    groups: usize,
    threads: usize,
    barriers: bool,
) {
    enc.set_compute_pipeline_state(pipeline);
    for (i, buffer) in buffers.iter().enumerate() {
        enc.set_buffer(i as u64, Some(buffer), 0);
    }
    enc.set_bytes(
        buffers.len() as u64,
        std::mem::size_of_val(constants) as u64,
        constants.as_ptr().cast(),
    );
    enc.dispatch_thread_groups(
        MTLSize::new(groups as u64, 1, 1),
        MTLSize::new(threads as u64, 1, 1),
    );
    if barriers {
        // Public Metal selector; MTLBarrierScopeBuffers = 1 (NSUInteger), as used
        // by pinned MLX device.cpp. metal-rs 0.32 lacks the scope wrapper.
        unsafe {
            let _: () = msg_send![enc, memoryBarrierWithScope: 1u64];
        }
    }
}
fn raw_sample(
    gpu: &Gpu,
    pool: &RawPool,
    source: &[(u32, u32)],
    expected: &[Row],
    keys: &[u32],
    mode: usize,
) -> Result<Value> {
    let n = source.len();
    let total = Stamp::start()?;
    let (cb, encoding) = phase(|| {
        let cb = gpu.queue.new_command_buffer();
        if mode == 6 {
            gpu.dispatch(
                cb,
                &pool.copy,
                &[&pool.input, pool.output()],
                &[n as u32],
                n.div_ceil(2048),
                256,
            );
        } else if matches!(mode, 1 | 2) {
            let concurrent = mode == 1;
            let enc = cb.compute_command_encoder_with_dispatch_type(if concurrent {
                MTLDispatchType::Concurrent
            } else {
                MTLDispatchType::Serial
            });
            pool.sort
                .encode_with_dispatch(&pool.input, pool.output(), n, |p, b, c, g, t| {
                    single_dispatch(enc, p, b, c, g, t, concurrent)
                });
            single_dispatch(
                enc,
                &pool.directory_pipeline,
                &[pool.output(), &pool.directory],
                &[n as u32],
                DIRECTORY.div_ceil(256),
                256,
                false,
            );
            enc.end_encoding();
        } else {
            pool.sort.encode(gpu, cb, &pool.input, pool.output(), n);
            gpu.dispatch(
                cb,
                &pool.directory_pipeline,
                &[pool.output(), &pool.directory],
                &[n as u32],
                DIRECTORY.div_ceil(256),
                256,
            );
        }
        Ok(cb)
    })?;
    let (_, submit) = phase(|| {
        cb.commit();
        Ok(())
    })?;
    let (_, ready_wait) = phase(|| {
        cb.wait_until_completed();
        if cb.status() != MTLCommandBufferStatus::Completed {
            return Err(format!("Metal command failed: {:?}", cb.status()).into());
        }
        Ok(())
    })?;
    let total = total.finish()?;
    let (start, end): (f64, f64) =
        unsafe { (msg_send![cb, GPUStartTime], msg_send![cb, GPUEndTime]) };
    let rows = view::<Row>(pool.output(), n);
    let directory = (mode != 6).then(|| view::<u32>(&pool.directory, DIRECTORY));
    let scan = first_scan(rows, directory)?;
    if let Some(directory) = directory {
        validate(rows, directory, expected, keys);
    } else {
        preserved(rows, source);
    }
    preserved(view::<Row>(&pool.input, n), source);
    let mut result = json!({"total":total,"encode":encoding,"submit":submit,
        "ready_wait":ready_wait,"first_cpu_output_scan":scan,"row_sized_buffers":3,
        "input_preserved":true,"validation":"complete rows, stable duplicate order, all 4097 directory offsets and lookup probes; copy validates complete source instead"});
    if start > 0.0 && end >= start {
        let ms = (end - start) * 1000.0;
        result["gpu_command_buffer_ms"] = json!(ms);
        if mode == 6 && ms > 0.0 {
            result["copy_logical_bytes"] = json!(n * 16);
            result["copy_gpu_logical_gb_s"] = json!(n as f64 * 16.0 / (ms * 1e6));
        }
    } else {
        result["gpu_timestamp_unavailable"] =
            json!("Metal did not supply a valid completed command-buffer timestamp interval");
    }
    if mode != 6 {
        result["sort_modeled_logical_bytes_excluding_directory"] =
            json!(pool.sort.logical_bytes(n));
    }
    Ok(result)
}
fn public_sample(
    gpu: &mccl::Gpu,
    source: &Relation,
    expected: &[Row],
    keys: &[u32],
) -> Result<Value> {
    let spec = IndexSpec {
        key_column: 0,
        payload: Payload::Column(1),
    };
    let (index, total) = phase(|| Ok(gpu.build_index(source, spec, None)?))?;
    // Both are repr(C) with the same two u32 fields, checked at startup. No tuple cast.
    let rows =
        unsafe { std::slice::from_raw_parts(index.rows().as_ptr().cast::<Row>(), index.len()) };
    let ready_memory = memory()?;
    let scan = first_scan(rows, Some(index.directory()))?;
    validate(rows, index.directory(), expected, keys);
    for &key in keys {
        let want = &expected[bounds(expected, key)];
        assert_eq!(index.contains(&key), !want.is_empty());
        assert!(index.lookup(&key).eq(want.iter().map(|r| r.value)));
    }
    let (_, release) = phase(|| {
        drop(index);
        Ok(())
    })?;
    Ok(
        json!({"total":total,"first_cpu_output_scan":scan,"release":release,"mlx_memory_ready":ready_memory,
        "input_preserved":true,"phase_scope":"completed public constructor from retained shared source; no ingestion or CPU population in this interval",
        "gpu_timestamp_unavailable":"no supported pinned mlx-c command-buffer timestamp API",
        "validation":"complete stable rows, exact directory, directory and public lookup/contains probes"}),
    )
}
fn ingest_c(stream: &Stream, source: &[(u32, u32)]) -> Result<Array> {
    let mut input = Array::new();
    let mut data = ptr::null_mut();
    let shape = [source.len() as i32, 2];
    native(unsafe {
        ffi::mccl_allocate_u32(&mut input.0, shape.as_ptr(), 2, &mut data, stream.0)
    })?;
    if data.is_null() {
        return Err("null nonempty relation mapping".into());
    }
    unsafe { populate(data.cast(), source) };
    Ok(input)
}

fn c_sample(
    stream: &Stream,
    input: &Array,
    source: &[(u32, u32)],
    expected: &[Row],
    keys: &[u32],
    full_builder: bool,
) -> Result<Value> {
    let total = Stamp::start()?;
    let mut sorted = Array::new();
    let mut directory = Array::new();
    let mut count = source.len();
    if full_builder {
        native(unsafe {
            ffi::mccl_build_relation_index_u32(
                &mut sorted.0,
                &mut directory.0,
                &mut count,
                input.0,
                0,
                1,
                ffi::Handle {
                    ctx: ptr::null_mut(),
                },
                stream.0,
            )
        })?;
    } else {
        native(unsafe { ffi::mccl_sort_pairs_u32(&mut sorted.0, input.0, stream.0) })?;
        native(unsafe { ffi::mccl_radix_directory_u32(&mut directory.0, sorted.0, stream.0) })?;
        directory.eval()?;
    }
    let rows = sorted.data::<Row>(count)?;
    let offsets = directory.data::<u32>(DIRECTORY)?;
    let total = total.finish()?;
    assert_eq!(count, source.len());
    let ready_memory = memory()?;
    let scan = first_scan(rows, Some(offsets))?;
    validate(rows, offsets, expected, keys);
    preserved(input.data::<Row>(source.len())?, source);
    let (_, release) = phase(|| {
        drop(directory);
        drop(sorted);
        Ok(())
    })?;
    Ok(
        json!({"total":total, "first_cpu_output_scan":scan,"release":release,
        "mlx_memory_ready":ready_memory,"input_preserved":true,
        "input_lifetime":"same immutable C relation retained across all cases, calls and repetitions",
        "gpu_timestamp_unavailable":"use native index_timing observer for actual command-buffer intervals",
        "phase_scope":if full_builder {"ready shared-relation constructor"} else {"buffer-level sort/directory from retained packed relation; no ingestion"},
        "validation":"complete stable rows, exact directory, lookup probes and immutable relation"}),
    )
}

fn numeric_leaves(value: &Value, prefix: &str, out: &mut BTreeMap<String, Vec<f64>>) {
    if let Some(map) = value.as_object() {
        for (key, value) in map {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            numeric_leaves(value, &path, out);
        }
    } else if let Some(n) = value.as_f64() {
        if prefix != "first_cpu_output_scan.checksum" {
            out.entry(prefix.into()).or_default().push(n);
        }
    }
}
fn summary(samples: &[Value]) -> Value {
    let mut metrics = BTreeMap::new();
    for sample in samples {
        numeric_leaves(&sample["measurement"], "", &mut metrics);
    }
    let mut result = serde_json::Map::new();
    for (name, mut values) in metrics {
        values.sort_by(f64::total_cmp);
        result.insert(name, json!({"median":values[values.len()/2],"min":values[0],"max":values[values.len()-1],"samples":values.len()}));
    }
    Value::Object(result)
}
fn save(path: &Path, report: &Value) -> Result<()> {
    let temporary = path.with_extension("json.partial");
    let mut writer = std::io::BufWriter::new(fs::File::create(&temporary)?);
    serde_json::to_writer_pretty(&mut writer, report)?;
    std::io::Write::flush(&mut writer)?;
    fs::rename(temporary, path)?;
    Ok(())
}
fn main() -> Result<()> {
    let mut sizes = vec![65_536usize, 1_048_576, 16_777_216];
    let mut runs = 5usize;
    let mut warmups = 2usize;
    let mut distributions = vec![
        "uniform".to_owned(),
        "workspace".to_owned(),
        "dense".to_owned(),
        "all_equal".to_owned(),
    ];
    let mut out = PathBuf::from("index-compare.json");
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        if flag == "--help" {
            println!(
                "index_compare --sizes N,N --runs ODD --warmups N --distributions uniform,workspace,dense,all_equal --out PATH"
            );
            return Ok(());
        }
        let value = args.next().ok_or("missing option value")?;
        match flag.as_str() {
            "--sizes" => {
                sizes = value
                    .split(',')
                    .map(str::parse)
                    .collect::<std::result::Result<_, _>>()?
            }
            "--runs" => runs = value.parse()?,
            "--warmups" => warmups = value.parse()?,
            "--distributions" => distributions = value.split(',').map(str::to_owned).collect(),
            "--out" => out = value.into(),
            _ => return Err(format!("unknown option {flag}").into()),
        }
    }
    if runs == 0
        || runs % 2 == 0
        || sizes.is_empty()
        || sizes.iter().any(|&n| n == 0 || n > 67_108_864)
        || distributions.is_empty()
        || distributions
            .iter()
            .any(|d| !["uniform", "workspace", "dense", "all_equal"].contains(&d.as_str()))
    {
        return Err(
            "require odd positive runs, 1..=67108864 row sizes and known nonempty distributions"
                .into(),
        );
    }
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    assert_eq!(
        std::mem::size_of::<Row>(),
        std::mem::size_of::<mccl::index::Row>()
    );
    assert_eq!(
        std::mem::align_of::<Row>(),
        std::mem::align_of::<mccl::index::Row>()
    );
    let (gpu, raw_context_setup) = phase(|| Ok(Gpu::new()))?;
    let (mccl, mccl_context_setup) = phase(|| Ok(mccl::Gpu::new()?))?;
    let (stream, c_stream_setup) = phase(Stream::new)?;
    let mut report = json!({"schema":"index_compare_v2_shared_source","device":gpu.device.name(),"sizes":sizes,
        "distributions":distributions,"runs":runs,"warmups":warmups,"variants":VARIANTS,
        "setup":{"raw_context":raw_context_setup,"mccl_context":mccl_context_setup,"c_stream":c_stream_setup},
        "configuration":{"radix_bits":8,"passes":4,"items_per_thread":8,"threads_per_group":256,"directory":"exact radix12, 4097 entries",
            "kernel_source":"experiments/mac/mccl/native/index; raw runtime compile, MCCL offline metallib",
            "mccl_items":"native preferred items8; capability fallback items4 can occur under GPU instrumentation; use uninstrumented launch",
            "environment":std::env::vars().filter(|(k,_)| k.starts_with("MTL_") || k.starts_with("METAL_") || k.starts_with("MLX_") || k.starts_with("MCCL_")).collect::<BTreeMap<_,_>>()},
        "method":{
            "source":"identical immutable packed u32 key/payload relation ingested ONCE per backend per case; all repetitions reuse the source handle",
            "workspace":"exact historical workspace/pool fixture: ((source_row_id*40503)&65535)*65537; full-width repeated keys and source row-ID payloads",
            "initial_touch":"only source ingestion CPU-writes relation data; raw index outputs, directory and scratch are uninitialized until GPU execution. CPU validation reads completed outputs outside timing",
            "allocation":"raw pipelines/output buffers outside samples; one persistent three-buffer pool. MCCL allocates output/scratch per call from its cache. All source handles are retained across variants; no per-build population",
            "order":"all seven variants rotate by case+round; every result fully checked after timing; retained source data never repopulated",
            "timing":"total ends after ready output, before first CPU scan and correctness. Phase instrumentation getrusage and JSON recording lies inside diagnostic totals; public API total has only outer instrumentation. No DRAM counters or GPU faults collected",
            "copy":"raw_copy_rows reads retained source and writes output; logical traffic 16N, not measured DRAM bytes. This is explicitly a copy reference, not index construction",
            "encoder_control":"normal raw uses one serial compute encoder per stage. Controlled three-buffer single concurrent uses explicit buffer-scope barriers after every sort dispatch, including before directory. Single serial uses no explicit barriers. Same Sort stage generator and pools",
            "mlx_gpu_timestamps":"unavailable: pinned public mlx/c/metal.h only offers capture; no safe command-buffer timestamp accessor. eval wall includes CPU scheduling and readiness waits, not GPU execution",
            "validation":"CPU stable reference sorted outside timing; every row and duplicate payload order, every directory offset, prefix-boundary/present/adjacent lookup probes; immutable raw/C input compared after output scan"},
        "cases":[],"complete":false});
    save(&out, &report)?;
    let mut case_number = 0usize;
    for &n in &sizes {
        for distribution in &distributions {
            let ((source, expected, keys), source_setup) = phase(|| {
                let source = source(n, distribution);
                let mut expected: Vec<Row> = source
                    .iter()
                    .map(|&(key, value)| Row { key, value })
                    .collect();
                expected.sort_by_key(|r| r.key);
                let keys = probes(&expected);
                Ok((source, expected, keys))
            })?;
            let (raw, raw_setup) = phase(|| Ok(RawPool::new(&gpu, n)))?;
            let (_, mut raw_ingestion) = phase(|| {
                unsafe { populate(raw.input.contents().cast(), &source) };
                Ok(())
            })?;
            throughput(&mut raw_ingestion, n * 8);
            let (shared, public_ingestion) =
                phase(|| Ok(mccl.relation_u32::<2>(n, |i| [source[i].0, source[i].1])?))?;
            let (c_input, c_ingestion) = phase(|| ingest_c(&stream, &source))?;
            report["cases"].as_array_mut().unwrap().push(json!({"n":n,"distribution":distribution,
                "source_and_reference_setup":source_setup,"raw_pool_setup":raw_setup,
                "ingestion":{"raw_population":raw_ingestion,"mccl_public":public_ingestion,"mccl_c":c_ingestion},
                "source_bytes_per_backend":n * 8,"raw_live_pool_bytes":raw.bytes(),
                "first_calls":[],"warmups":[],"samples":[],"summary":{}}));
            save(&out, &report)?;
            for round in 0..1 + warmups + runs {
                let category = if round == 0 {
                    "first_calls"
                } else if round <= warmups {
                    "warmups"
                } else {
                    "samples"
                };
                for step in 0..VARIANTS.len() {
                    let slot = (case_number + round + step) % VARIANTS.len();
                    let before = memory()?;
                    let measurement = metal::objc::rc::autoreleasepool(|| match slot {
                        0 | 1 | 2 | 6 => raw_sample(&gpu, &raw, &source, &expected, &keys, slot),
                        3 => public_sample(&mccl, &shared, &expected, &keys),
                        4 | 5 => c_sample(&stream, &c_input, &source, &expected, &keys, slot == 4),
                        _ => unreachable!(),
                    })?;
                    assert!(
                        shared
                            .u32_values()?
                            .chunks_exact(2)
                            .zip(&source)
                            .all(|(row, &(key, value))| row == [key, value]),
                        "public shared relation changed after construction"
                    );
                    let after = memory()?;
                    let entry = json!({"variant":VARIANTS[slot],"round":round,"order_in_round":step,
                        "mlx_memory_before":before,"mlx_memory_after_release":after,"measurement":measurement});
                    report["cases"][case_number][category]
                        .as_array_mut()
                        .unwrap()
                        .push(entry);
                }
            }
            for variant in VARIANTS {
                let selected: Vec<Value> = report["cases"][case_number]["samples"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|s| s["variant"].as_str() == Some(variant))
                    .cloned()
                    .collect();
                let stats = summary(&selected);
                println!(
                    "{n}\t{distribution}\t{variant}\tmedian_total_ms={:.3}",
                    stats["total.wall_ms"]["median"].as_f64().unwrap()
                );
                report["cases"][case_number]["summary"][variant] = stats;
            }
            save(&out, &report)?;
            case_number += 1;
        }
    }
    report["complete"] = json!(true);
    save(&out, &report)?;
    println!("Raw samples and medians: {}", out.display());
    Ok(())
}
