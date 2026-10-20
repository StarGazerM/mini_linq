//! General hand-written H05 (TPC-H Q5 projection, SELECT DISTINCT) reference.
//!
//! Compression (load, untimed): columns read at their declared types; every
//! join key dictionary-encoded to a dense u32 ID per domain (order-preserving);
//! every base column FOR bit-packed per 1024-row vector and made resident.
//! Timed, per iteration, from compressed resident base columns to the resident
//! distinct answer: build the five CSR indexes of plan R N C O L S on the GPU
//! with RCCL (stable radix sort by key, batched lower_bound for offsets), run
//! the query, deduplicate with RCCL (radix sort, unique). The reported figure is
//! host wall clock over that span; GPU events give the breakdown.
//!
//! Usage: h05-reference DATA_DIR [RESULTS_JSON] [--capacity N] [--repeat N]
mod kernels;

use cuda_core::{CudaStream, DeviceBuffer, DeviceCopy, LaunchConfig1D};
use gpu_device_columns::{Bounds, Buffer, Codec, EncodedColumn, Lane, MemoryResource, Pool};
use kernels::*;
use rccl_thrust::prelude::*;
use rccl_thrust::{arithmetic_range, lower_bound, sequence, unique_copy_by};
use serde_json::{json, Value};
use std::{collections::BTreeMap, error::Error, fs, path::Path, sync::Arc, time::Instant};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
type Exec<'a> = rccl_thrust::cuda::Execution<'a, 'a, &'a MemoryResource>;

const EXPAND_GRID: u32 = 568;
const STRIDED_GRID: usize = 568 * 4;

/// Source columns at their declared manifest types. Host-only, load-time.
struct Source {
    columns: BTreeMap<String, Vec<i64>>,
    declared_bytes: usize,
    constants: Value,
    sf: Value,
}

impl Source {
    fn load(dir: &Path) -> Result<Self> {
        let manifest: Value = serde_json::from_slice(&fs::read(dir.join("manifest.json"))?)?;
        let mut columns = BTreeMap::new();
        let mut declared_bytes = 0;
        for (relation, meta) in manifest["relations"].as_object().ok_or("relations missing")? {
            let rows = meta["rows"].as_u64().ok_or("row count missing")? as usize;
            if rows >= u32::MAX as usize {
                return Err(format!("{relation}: row count exceeds the u32 row-ID domain").into());
            }
            for column in meta["columns"].as_array().ok_or("columns missing")? {
                let name = column["name"].as_str().ok_or("column name missing")?;
                let bytes = fs::read(dir.join(column["file"].as_str().ok_or("column file missing")?))?;
                let width = match column["type"].as_str().ok_or("column type missing")? {
                    "i64" => 8,
                    "i32" => 4,
                    other => return Err(format!("{relation}.{name}: unsupported declared type {other}").into()),
                };
                if bytes.len() != rows * width {
                    return Err(format!("{relation}.{name}: byte length does not match its declared type").into());
                }
                let values = bytes
                    .chunks_exact(width)
                    .map(|b| {
                        if width == 8 {
                            i64::from_le_bytes(b.try_into().unwrap())
                        } else {
                            i32::from_le_bytes(b.try_into().unwrap()) as i64
                        }
                    })
                    .collect();
                declared_bytes += bytes.len();
                columns.insert(format!("{relation}.{name}"), values);
            }
        }
        Ok(Self { columns, declared_bytes, constants: manifest["constants"].clone(), sf: manifest["sf"].clone() })
    }
    fn col(&self, name: &str) -> Result<&[i64]> {
        self.columns.get(name).map(Vec::as_slice).ok_or_else(|| format!("missing column {name}").into())
    }
    fn constant(&self, name: &str) -> Result<i32> {
        let value = self.constants[name].as_i64().ok_or_else(|| format!("missing constant {name}"))?;
        Ok(i32::try_from(value)?)
    }
    /// An INTEGER/DATE column at its declared 32-bit type.
    fn i32_column(&self, name: &str) -> Result<Vec<i32>> {
        self.col(name)?.iter().map(|&v| Ok(i32::try_from(v)?)).collect()
    }
}

/// Order-preserving dictionary of one join domain: dense IDs in [0, len).
struct Dictionary {
    values: Vec<i64>,
}

impl Dictionary {
    fn new(columns: &[&[i64]]) -> Result<Self> {
        let mut values: Vec<i64> = columns.iter().flat_map(|c| c.iter().copied()).collect();
        values.sort_unstable();
        values.dedup();
        if values.len() >= u32::MAX as usize {
            return Err("dictionary exceeds the u32 ID domain".into());
        }
        Ok(Self { values })
    }
    fn ids(&self, column: &[i64]) -> Vec<u32> {
        column.iter().map(|v| self.values.binary_search(v).expect("value in its domain") as u32).collect()
    }
    fn len(&self) -> usize {
        self.values.len()
    }
    /// Bits of an ID: ceil(log2(len)).
    fn bits(&self) -> u32 {
        if self.len() <= 1 { 0 } else { 64 - ((self.len() - 1) as u64).leading_zeros() }
    }
    fn bounds(&self) -> Bounds {
        if self.len() == 0 { Bounds::Empty } else { Bounds::Inclusive { min: 0, max: self.len() as i64 - 1 } }
    }
    fn value(&self, id: u32) -> i64 {
        self.values[id as usize]
    }
}

/// Compressed resident base tables (join keys as dictionary IDs).
struct Base {
    columns: BTreeMap<&'static str, EncodedColumn>,
    rows: BTreeMap<&'static str, usize>,
    date_bounds: Bounds,
    orders: Dictionary,
    customers: Dictionary,
    suppliers: Dictionary,
    nations: Dictionary,
    regions: Dictionary,
}

impl Base {
    fn compress(pool: &Arc<Pool>, source: &Source) -> Result<Self> {
        let orders = Dictionary::new(&[source.col("orders.o_orderkey")?, source.col("lineitem.l_orderkey")?])?;
        let customers = Dictionary::new(&[source.col("customer.c_custkey")?, source.col("orders.o_custkey")?])?;
        let suppliers = Dictionary::new(&[source.col("supplier.s_suppkey")?, source.col("lineitem.l_suppkey")?])?;
        let nations = Dictionary::new(&[
            source.col("nation.n_nationkey")?,
            source.col("customer.c_nationkey")?,
            source.col("supplier.s_nationkey")?,
        ])?;
        let regions = Dictionary::new(&[source.col("region.r_regionkey")?, source.col("nation.n_regionkey")?])?;
        let mut columns = BTreeMap::new();
        let mut rows = BTreeMap::new();
        fn pack<L: Lane>(
            pool: &Arc<Pool>,
            columns: &mut BTreeMap<&'static str, EncodedColumn>,
            rows: &mut BTreeMap<&'static str, usize>,
            name: &'static str,
            values: &[L],
        ) -> Result<()> {
            columns.insert(name, EncodedColumn::from_host(pool.clone(), values, Codec::For)?);
            rows.insert(name, values.len());
            Ok(())
        }
        let id_columns: [(&'static str, &Dictionary, &str); 11] = [
            ("region.id", &regions, "region.r_regionkey"),
            ("nation.id", &nations, "nation.n_nationkey"),
            ("nation.region", &regions, "nation.n_regionkey"),
            ("customer.id", &customers, "customer.c_custkey"),
            ("customer.nation", &nations, "customer.c_nationkey"),
            ("orders.id", &orders, "orders.o_orderkey"),
            ("orders.customer", &customers, "orders.o_custkey"),
            ("lineitem.order", &orders, "lineitem.l_orderkey"),
            ("lineitem.supplier", &suppliers, "lineitem.l_suppkey"),
            ("supplier.id", &suppliers, "supplier.s_suppkey"),
            ("supplier.nation", &nations, "supplier.s_nationkey"),
        ];
        for (name, dictionary, column) in &id_columns {
            pack(pool, &mut columns, &mut rows, name, &dictionary.ids(source.col(column)?))?;
        }
        pack(pool, &mut columns, &mut rows, "region.name", &source.i32_column("region.r_name")?)?;
        let dates = source.i32_column("orders.o_orderdate")?;
        let date_bounds = match (dates.iter().min(), dates.iter().max()) {
            (Some(&min), Some(&max)) => Bounds::Inclusive { min: min as i64, max: max as i64 },
            _ => Bounds::Empty,
        };
        pack(pool, &mut columns, &mut rows, "orders.date", &dates)?;
        Ok(Self { columns, rows, date_bounds, orders, customers, suppliers, nations, regions })
    }
    fn view(&self, name: &str) -> gpu_device_columns::ColumnView {
        unsafe { self.columns[name].view() }
    }
    fn bytes(&self) -> usize {
        self.columns.values().map(EncodedColumn::bytes).sum()
    }
}

/// One CSR index to build: key column over a dense domain, payload columns.
struct IndexSpec {
    name: &'static str,
    key: &'static str,
    domain: usize,
    payload: Vec<(&'static str, bool, Bounds)>,
}

/// Row-count owners for device packing, allocated once outside timing.
struct Counts {
    rows: Arc<Buffer<u64>>,
    offsets: Arc<Buffer<u64>>,
}

struct BuiltIndex {
    offsets: EncodedColumn,
    payload: Vec<EncodedColumn>,
}

/// Stream-ordered temporary; released with `release`, never by implicit drop
/// (which synchronizes the whole context).
fn temporary<T: DeviceCopy>(stream: &Arc<CudaStream>, len: usize) -> Result<DeviceBuffer<T>> {
    Ok(unsafe { DeviceBuffer::uninitialized_async(stream, len)? })
}

fn release<T: DeviceCopy>(stream: &CudaStream, buffer: DeviceBuffer<T>) -> Result<()> {
    unsafe { buffer.drop_async(stream)? };
    Ok(())
}

fn pointer<T: DeviceCopy>(buffer: &DeviceBuffer<T>) -> *mut T {
    buffer.cu_deviceptr() as *mut T
}

/// Build one CSR index from compressed base columns:
/// decode keys; RCCL sequence + stable radix sort of (key, row);
/// RCCL lower_bound of every key ID for the offsets; gather payload by row;
/// FOR-pack offsets and payload.
fn build_index(
    module: &reference::LoadedModule,
    exec: &Exec<'_>,
    pool: &Arc<Pool>,
    base: &Base,
    spec: &IndexSpec,
    counts: &Counts,
) -> Result<BuiltIndex> {
    let stream = pool.stream();
    let rows = base.rows[spec.key];
    let entries = spec.domain + 1;
    let strided = LaunchConfig1D::new(rows.div_ceil(256).clamp(1, STRIDED_GRID) as u32, 256, 0);

    let mut keys = temporary::<u32>(stream, rows)?;
    let mut order = temporary::<u32>(stream, rows)?;
    module.decode_keys(stream, &module.prepare_decode_keys(strided)?, base.view(spec.key), rows as u32, pointer(&keys))?;
    sequence(exec, &mut order)?;
    keys.view_mut().with_values(order.view_mut())?.sort(exec)?;
    let mut offsets = temporary::<u64>(stream, entries)?;
    lower_bound(exec, keys.view(), arithmetic_range(0_u32, 1, entries), offsets.view_mut())?;

    let outputs: Vec<DeviceBuffer<u32>> =
        spec.payload.iter().map(|_| temporary::<u32>(stream, rows.max(1))).collect::<Result<_>>()?;
    let unused = base.view(spec.key);
    let payload = Payload {
        columns: [
            spec.payload.first().map_or(unused, |p| base.view(p.0)),
            spec.payload.get(1).map_or(unused, |p| base.view(p.0)),
        ],
        outputs: [
            outputs.first().map_or(std::ptr::null_mut(), pointer),
            outputs.get(1).map_or(std::ptr::null_mut(), pointer),
        ],
        signed: [
            spec.payload.first().map_or(0, |p| p.1 as u32),
            spec.payload.get(1).map_or(0, |p| p.1 as u32),
        ],
        count: spec.payload.len() as u32,
    };
    module.gather(stream, &module.prepare_gather(strided)?, pointer(&order) as *const u32, rows as u32, payload)?;

    let offsets_column = unsafe {
        EncodedColumn::from_device(
            pool.clone(),
            pointer(&offsets) as *const i64,
            entries,
            counts.offsets.clone(),
            0,
            Codec::For,
            Bounds::Inclusive { min: 0, max: rows as i64 },
        )?
    };
    let payload = spec
        .payload
        .iter()
        .zip(&outputs)
        .map(|((_, signed, bounds), output)| unsafe {
            if *signed {
                EncodedColumn::from_device(pool.clone(), pointer(output) as *const i32, rows, counts.rows.clone(), 0, Codec::For, *bounds)
            } else {
                EncodedColumn::from_device(pool.clone(), pointer(output) as *const u32, rows, counts.rows.clone(), 0, Codec::For, *bounds)
            }
        })
        .collect::<Result<Vec<_>>>()?;
    release(stream, keys)?;
    release(stream, order)?;
    release(stream, offsets)?;
    for output in outputs {
        release(stream, output)?;
    }
    Ok(BuiltIndex { offsets: offsets_column, payload })
}

/// Query-owned device buffers for one output capacity.
struct Workspace {
    meta: Buffer<u32>,
    segment_begin: Buffer<u32>,
    segment_nation: Buffer<u32>,
    segment_offsets: Buffer<u32>,
    segment_capacity: u32,
    answers: Buffer<u64>,
    capacity: usize,
}

impl Workspace {
    fn new(pool: &Arc<Pool>, segment_capacity: usize, capacity: usize) -> Result<Self> {
        let capacity = capacity.max(1);
        if capacity > u32::MAX as usize {
            return Err("output capacity exceeds the u32 bound".into());
        }
        let column = |n: usize| Buffer::<u32>::zeroed(pool.clone(), n);
        Ok(Self {
            meta: Buffer::zeroed(pool.clone(), META_WORDS)?,
            segment_begin: column(segment_capacity)?,
            segment_nation: column(segment_capacity)?,
            segment_offsets: column(segment_capacity + 1)?,
            segment_capacity: segment_capacity as u32,
            answers: Buffer::zeroed(pool.clone(), capacity)?,
            capacity,
        })
    }
}

struct Sample {
    wall_ms: f64,
    gpu_ms: f32,
    build_ms: f32,
    index_ms: Vec<f32>,
    expand_ms: f32,
    dedup_ms: f32,
    emitted: u32,
    unique: u64,
    index_bytes: usize,
}

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut option = |flag: &str| -> Result<Option<usize>> {
        match args.iter().position(|a| a == flag) {
            Some(i) => {
                let value = args.get(i + 1).ok_or_else(|| format!("{flag} needs a value"))?.parse()?;
                args.drain(i..i + 2);
                Ok(Some(value))
            }
            None => Ok(None),
        }
    };
    let mut capacity = option("--capacity")?.unwrap_or(1024);
    let repeat = option("--repeat")?.unwrap_or(5);
    let dir = Path::new(args.first().ok_or("usage: h05-reference DATA_DIR [RESULTS_JSON] [--capacity N] [--repeat N]")?);
    let results_path = args.get(1).cloned();

    let read_started = Instant::now();
    let source = Source::load(dir)?;
    let read_ms = read_started.elapsed().as_secs_f64() * 1000.0;
    let constants = Constants {
        asia: source.constant("asia_id")?,
        date_start: source.constant("date_start_days")?,
        date_end: source.constant("date_end_days")?,
    };
    let pool = Pool::new(0, 2 << 30, 16 << 30)?;
    // Setup: keep the stream-ordered allocator's memory across synchronizations.
    unsafe {
        let mut memory_pool = std::ptr::null_mut();
        let status = cuda_core::sys::cuDeviceGetDefaultMemPool(&mut memory_pool, 0);
        if status != 0 {
            return Err(format!("cuDeviceGetDefaultMemPool: {status}").into());
        }
        let mut threshold = u64::MAX;
        let status = cuda_core::sys::cuMemPoolSetAttribute(
            memory_pool,
            cuda_core::sys::CUmemPool_attribute_enum_CU_MEMPOOL_ATTR_RELEASE_THRESHOLD,
            &mut threshold as *mut u64 as *mut std::ffi::c_void,
        );
        if status != 0 {
            return Err(format!("cuMemPoolSetAttribute: {status}").into());
        }
    }
    let compress_started = Instant::now();
    let base = Base::compress(&pool, &source)?;
    let compress_ms = compress_started.elapsed().as_secs_f64() * 1000.0;
    let declared_bytes = source.declared_bytes;
    let sf = source.sf.clone();
    let nation_rows = base.rows["nation.id"];
    let region_rows = base.rows["region.id"];
    drop(source);

    let answer: Vec<(i64, i64, i64)> = {
        let mut rows = fs::read_to_string(dir.join("answer.tsv"))?
            .lines()
            .map(|line| {
                let f: Vec<i64> = line.split('\t').map(str::parse).collect::<std::result::Result<_, _>>()?;
                if f.len() != 3 {
                    return Err::<_, Box<dyn Error>>("answer.tsv rows need three columns".into());
                }
                Ok((f[0], f[1], f[2]))
            })
            .collect::<Result<Vec<_>>>()?;
        rows.sort_unstable();
        rows.dedup();
        rows
    };

    let (order_bits, supplier_bits, nation_bits) = (base.orders.bits(), base.suppliers.bits(), base.nations.bits());
    if order_bits + supplier_bits + nation_bits > 64 {
        return Err("packed answer key exceeds 64 bits for these dictionary sizes".into());
    }
    let specs = [
        IndexSpec { name: "nation_by_region", key: "nation.region", domain: base.regions.len(), payload: vec![("nation.id", false, base.nations.bounds())] },
        IndexSpec { name: "customer_by_nation", key: "customer.nation", domain: base.nations.len(), payload: vec![("customer.id", false, base.customers.bounds())] },
        IndexSpec {
            name: "orders_by_customer",
            key: "orders.customer",
            domain: base.customers.len(),
            payload: vec![("orders.id", false, base.orders.bounds()), ("orders.date", true, base.date_bounds)],
        },
        IndexSpec { name: "lineitem_by_order", key: "lineitem.order", domain: base.orders.len(), payload: vec![("lineitem.supplier", false, base.suppliers.bounds())] },
        IndexSpec { name: "supplier_by_supplier", key: "supplier.id", domain: base.suppliers.len(), payload: vec![("supplier.nation", false, base.nations.bounds())] },
    ];
    let counts: Vec<Counts> = specs
        .iter()
        .map(|spec| {
            Ok(Counts {
                rows: Arc::new(Buffer::from_host(pool.clone(), &[base.rows[spec.key] as u64])?),
                offsets: Arc::new(Buffer::from_host(pool.clone(), &[(spec.domain + 1) as u64])?),
            })
        })
        .collect::<Result<_>>()?;

    let module = unsafe { reference::load(pool.context())? };
    let thrust = rccl_thrust::cuda::Program::load(pool.context())?;
    let stream = pool.stream();
    let exec: Exec<'_> = thrust.par_in(stream, pool.resource())?;
    let segments_launch = module.prepare_segments(LaunchConfig1D::new(1, 256, 0))?;
    let expand_launch = module.prepare_expand(LaunchConfig1D::new(EXPAND_GRID, 256, EXPAND_SHARED_BYTES))?;
    let timing = Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
    let segment_capacity = (region_rows * nation_rows).max(1);

    let mut workspace = Workspace::new(&pool, segment_capacity, capacity)?;
    let mut retries = 0;
    let mut samples: Vec<Sample> = Vec::new();
    let mut checked = false;
    while samples.len() < repeat + 1 {
        let w = &workspace;
        stream.synchronize()?;
        let wall = Instant::now();
        let start = stream.record_event(timing)?;
        let mut built = Vec::new();
        let mut index_events = Vec::new();
        for (spec, count) in specs.iter().zip(&counts) {
            built.push(build_index(&module, &exec, &pool, &base, spec, count)?);
            index_events.push(stream.record_event(timing)?);
        }
        let view = |index: usize, column: usize| unsafe {
            if column == 0 { built[index].offsets.view() } else { built[index].payload[column - 1].view() }
        };
        let ix = Indexes {
            region_id: base.view("region.id"),
            region_name: base.view("region.name"),
            region_rows: region_rows as u32,
            nation_offsets: view(0, 0),
            nation_id: view(0, 1),
            customer_offsets: view(1, 0),
            customer_id: view(1, 1),
            order_offsets: view(2, 0),
            order_id: view(2, 1),
            order_date: view(2, 2),
            line_offsets: view(3, 0),
            line_supplier: view(3, 1),
            supplier_offsets: view(4, 0),
            supplier_nation: view(4, 1),
        };
        let segments = Segments {
            begin: w.segment_begin.ptr(),
            nation: w.segment_nation.ptr(),
            offsets: w.segment_offsets.ptr(),
            capacity: w.segment_capacity,
        };
        let out = Answers {
            keys: w.answers.ptr(),
            capacity: w.capacity as u32,
            order_shift: supplier_bits + nation_bits,
            supplier_shift: nation_bits,
        };
        let meta = w.meta.ptr();
        unsafe {
            cuda_core::memory::memset_d8_async(w.meta.cu_deviceptr(), 0, w.meta.bytes(), stream.cu_stream())?;
        }
        module.segments(stream, &segments_launch, ix, constants, segments, meta)?;
        module.expand(stream, &expand_launch, ix, constants, segments, out, meta)?;
        let expanded = stream.record_event(timing)?;
        // The emitted count sizes the dedup; it is the query's control readback.
        let words = w.meta.to_host()?;
        if words[META_OVERFLOW] != 0 {
            return Err("query exceeded a u32 count/offset bound".into());
        }
        let emitted = words[META_EMITTED];
        if emitted as usize > w.capacity {
            capacity = emitted as usize;
            retries += 1;
            workspace = Workspace::new(&pool, segment_capacity, capacity)?;
            continue;
        }
        let rows = emitted as usize;
        let mut keys = temporary::<u64>(stream, rows)?;
        if rows > 0 {
            unsafe {
                cuda_core::memory::memcpy_dtod_async(keys.cu_deviceptr(), w.answers.cu_deviceptr(), rows * 8, stream.cu_stream())?;
            }
        }
        keys.sort_unstable(&exec)?;
        let mut unique = temporary::<u64>(stream, rows)?;
        let mut selected = temporary::<u64>(stream, 1)?;
        unique_copy_by(&exec, keys.view(), unique.view_mut(), &mut selected, |a: u64, b: u64| a == b)?;
        let finished = stream.record_event(timing)?;
        let unique_count = selected.to_host_vec(stream)?[0];
        let wall_ms = wall.elapsed().as_secs_f64() * 1000.0;

        let built_event = index_events.last().unwrap();
        let mut index_ms = Vec::new();
        let mut previous = &start;
        for event in &index_events {
            index_ms.push(previous.elapsed_ms(event)?);
            previous = event;
        }
        let sample = Sample {
            wall_ms,
            gpu_ms: start.elapsed_ms(&finished)?,
            build_ms: start.elapsed_ms(built_event)?,
            index_ms,
            expand_ms: built_event.elapsed_ms(&expanded)?,
            dedup_ms: expanded.elapsed_ms(&finished)?,
            emitted,
            unique: unique_count,
            index_bytes: built.iter().map(|b| b.offsets.bytes() + b.payload.iter().map(EncodedColumn::bytes).sum::<usize>()).sum(),
        };
        if !checked {
            let words = unique.to_host_vec(stream)?;
            let supplier_mask = (1u64 << supplier_bits) - 1;
            let nation_mask = (1u64 << nation_bits) - 1;
            let order_shift = supplier_bits + nation_bits;
            let mut actual: Vec<(i64, i64, i64)> = words[..unique_count as usize]
                .iter()
                .map(|&key| {
                    (
                        base.orders.value((key >> order_shift) as u32),
                        base.suppliers.value(((key >> nation_bits) & supplier_mask) as u32),
                        base.nations.value((key & nation_mask) as u32),
                    )
                })
                .collect();
            actual.sort_unstable();
            if actual != answer {
                return Err(format!(
                    "exact mismatch: {} rows vs DuckDB {}; first difference {:?}",
                    actual.len(),
                    answer.len(),
                    actual.iter().zip(&answer).find(|(a, b)| a != b)
                )
                .into());
            }
            checked = true;
        }
        release(stream, keys)?;
        release(stream, unique)?;
        release(stream, selected)?;
        println!(
            "sf={sf} iteration={} wall_ms={:.4} gpu_ms={:.4} build_ms={:.4} expand_ms={:.4} dedup_ms={:.4} emitted={} unique={} exact=true",
            samples.len(),
            sample.wall_ms,
            sample.gpu_ms,
            sample.build_ms,
            sample.expand_ms,
            sample.dedup_ms,
            sample.emitted,
            sample.unique
        );
        samples.push(sample);
    }

    let measured = &samples[1..];
    let metric = |values: Vec<f64>| {
        let mut v = values;
        v.sort_by(f64::total_cmp);
        json!({"median": v[v.len() / 2], "min": v[0], "samples": v})
    };
    let index_ms: BTreeMap<&str, Value> = specs
        .iter()
        .enumerate()
        .map(|(i, spec)| (spec.name, metric(measured.iter().map(|s| s.index_ms[i] as f64).collect())))
        .collect();
    let base_columns: Vec<Value> = base
        .columns
        .iter()
        .map(|(name, c)| json!({"name": name, "rows": c.capacity(), "encoded_bytes": c.bytes()}))
        .collect();
    let report = json!({
        "sf": sf,
        "plan": "R N C O L S",
        "timed": "compressed resident base -> GPU index build (RCCL) + query + dedup (RCCL) -> resident distinct answer",
        "wall_ms": metric(measured.iter().map(|s| s.wall_ms).collect()),
        "gpu_ms": metric(measured.iter().map(|s| s.gpu_ms as f64).collect()),
        "build_ms": metric(measured.iter().map(|s| s.build_ms as f64).collect()),
        "index_ms": index_ms,
        "expand_ms": metric(measured.iter().map(|s| s.expand_ms as f64).collect()),
        "dedup_ms": metric(measured.iter().map(|s| s.dedup_ms as f64).collect()),
        "untimed_load": {"read_ms": read_ms, "compress_ms": compress_ms},
        "representation": {
            "declared_source_bytes": declared_bytes,
            "compressed_base_bytes": base.bytes(),
            "index_bytes": samples[0].index_bytes,
            "dictionary_ids": {
                "orderkey": base.orders.len(), "custkey": base.customers.len(), "suppkey": base.suppliers.len(),
                "nationkey": base.nations.len(), "regionkey": base.regions.len(),
            },
            "id_bits": {"orderkey": order_bits, "suppkey": supplier_bits, "nationkey": nation_bits},
            "base_columns": base_columns,
        },
        "emitted_bindings": samples[0].emitted,
        "unique_answers": samples[0].unique,
        "duckdb_answers": answer.len(),
        "exact": true,
        "capacity_retries": retries,
        "warmup_excluded": 1,
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    if let Some(path) = results_path {
        fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}
