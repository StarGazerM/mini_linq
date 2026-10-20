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
use gpu_compress::{Catalog, Domain, Kind, Schema, Source};
use gpu_device_columns::{Bounds, Buffer, Codec, ColumnView, EncodedColumn, MemoryResource, Pool};
use kernels::*;
use rccl_cub::{cuda::Launch, DeviceRadixSortBufferWorkspace, DoubleBuffer};
use rccl_thrust::prelude::*;
use rccl_thrust::{arithmetic_range, lower_bound, unique_copy_by};
use serde_json::{json, Value};
use std::{collections::BTreeMap, error::Error, fs, path::Path, sync::Arc, time::Instant};

type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;
type Exec<'a> = rccl_thrust::cuda::Execution<'a, 'a, &'a MemoryResource>;

const EXPAND_GRID: u32 = 568;
const STRIDED_GRID: usize = 568 * 4;

/// H05's stored schema: five join domains; every join key stored as a domain
/// ID, Region.name and the order date at their declared i32 type.
fn schema() -> Schema {
    let id = |domain: &str| Kind::Id(domain.to_string());
    let columns = |list: &[(&str, Kind)]| list.iter().map(|(c, k)| (c.to_string(), k.clone())).collect();
    let members = |list: &[&str]| list.iter().map(|m| m.to_string()).collect();
    Schema {
        domains: vec![
            ("orderkey".into(), members(&["orders.o_orderkey", "lineitem.l_orderkey"])),
            ("custkey".into(), members(&["customer.c_custkey", "orders.o_custkey"])),
            ("suppkey".into(), members(&["supplier.s_suppkey", "lineitem.l_suppkey"])),
            ("nationkey".into(), members(&["nation.n_nationkey", "customer.c_nationkey", "supplier.s_nationkey"])),
            ("regionkey".into(), members(&["region.r_regionkey", "nation.n_regionkey"])),
        ],
        tables: vec![
            ("region".into(), columns(&[("r_regionkey", id("regionkey")), ("r_name", Kind::I32)])),
            ("nation".into(), columns(&[("n_nationkey", id("nationkey")), ("n_regionkey", id("regionkey"))])),
            ("customer".into(), columns(&[("c_custkey", id("custkey")), ("c_nationkey", id("nationkey"))])),
            (
                "orders".into(),
                columns(&[("o_orderkey", id("orderkey")), ("o_custkey", id("custkey")), ("o_orderdate", Kind::I32)]),
            ),
            ("lineitem".into(), columns(&[("l_orderkey", id("orderkey")), ("l_suppkey", id("suppkey"))])),
            ("supplier".into(), columns(&[("s_suppkey", id("suppkey")), ("s_nationkey", id("nationkey"))])),
        ],
    }
}

/// Stored columns addressed by their source names (`table.column`).
struct Base {
    catalog: Catalog,
}

impl Base {
    fn split(name: &str) -> (&str, &str) {
        name.split_once('.').expect("qualified column name")
    }
    fn view(&self, name: &str) -> ColumnView {
        let (table, column) = Self::split(name);
        self.catalog.view(table, column).expect("stored column")
    }
    fn rows(&self, name: &str) -> usize {
        self.catalog.rows(Self::split(name).0).expect("stored table")
    }
    fn bounds(&self, name: &str) -> Bounds {
        let (table, column) = Self::split(name);
        self.catalog.column(table, column).expect("stored column").column.bounds()
    }
    fn domain(&self, name: &str) -> &Domain {
        self.catalog.domain(name).expect("stored domain")
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

/// Bits needed for IDs in [0, domain): the radix sort's key window.
fn key_bits(domain: usize) -> u32 {
    if domain <= 1 { 0 } else { usize::BITS - (domain - 1).leading_zeros() }
}

/// Build one CSR index from compressed base columns:
/// decode (key, row) pairs; RCCL stable radix sort of the pairs over only the
/// key's ID bits; RCCL lower_bound of every key ID for the offsets; payload
/// gathered by sorted row and FOR-packed in one pass.
#[allow(clippy::too_many_arguments)]
fn build_index(
    module: &reference::LoadedModule,
    exec: &Exec<'_>,
    launch: &Launch<'_, '_>,
    sort: &mut DeviceRadixSortBufferWorkspace,
    pool: &Arc<Pool>,
    base: &Base,
    spec: &IndexSpec,
    counts: &Counts,
) -> Result<BuiltIndex> {
    let stream = pool.stream();
    let rows = base.rows(spec.key);
    let entries = spec.domain + 1;
    let strided = LaunchConfig1D::new(rows.div_ceil(256).clamp(1, STRIDED_GRID) as u32, 256, 0);

    let mut keys = temporary::<u32>(stream, rows)?;
    let mut order = temporary::<u32>(stream, rows)?;
    let mut alternate_keys = temporary::<u32>(stream, rows)?;
    let mut alternate_order = temporary::<u32>(stream, rows)?;
    let mut offsets = temporary::<u64>(stream, entries)?;
    module.keys_and_rows(
        stream,
        &module.prepare_keys_and_rows(strided)?,
        base.view(spec.key),
        rows as u32,
        pointer(&keys),
        pointer(&order),
    )?;
    let payload = {
        let mut pairs = DoubleBuffer::paired((&mut keys, &mut order), (&mut alternate_keys, &mut alternate_order))
            .ok_or("radix double buffer extents differ")?;
        rccl_cub::device_radix_sort_pairs_buffer(launch, &mut pairs, 0..key_bits(spec.domain), sort)?;
        let (sorted_keys, sorted_order) = pairs.current();
        lower_bound(exec, sorted_keys.view(), arithmetic_range(0_u32, 1, entries), offsets.view_mut())?;
        let sorted = sorted_order.cu_deviceptr() as *const u32;
        spec.payload
            .iter()
            .map(|(column, signed, bounds)| unsafe {
                if *signed {
                    EncodedColumn::from_gather::<_, i32>(pool.clone(), base.view(column), sorted, rows, counts.rows.clone(), 0, *bounds)
                } else {
                    EncodedColumn::from_gather::<_, u32>(pool.clone(), base.view(column), sorted, rows, counts.rows.clone(), 0, *bounds)
                }
            })
            .collect::<Result<Vec<_>>>()?
    };
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
    for buffer in [keys, order, alternate_keys, alternate_order] {
        release(stream, buffer)?;
    }
    release(stream, offsets)?;
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
    /// Reusable RCCL radix controls for the answer keys at this capacity.
    sort: DeviceRadixSortBufferWorkspace,
    capacity: usize,
}

impl Workspace {
    fn new(pool: &Arc<Pool>, launch: &Launch<'_, '_>, segment_capacity: usize, capacity: usize) -> Result<Self> {
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
            sort: rccl_cub::device_radix_sort_keys_buffer_workspace::<u64>(launch, capacity)?,
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
    let source = Source::load_manifest(dir)?;
    let read_ms = read_started.elapsed().as_secs_f64() * 1000.0;
    let constant = |name: &str| -> Result<i32> {
        let value = source.manifest()["constants"][name].as_i64().ok_or_else(|| format!("missing constant {name}"))?;
        Ok(i32::try_from(value)?)
    };
    let constants = Constants {
        asia: constant("asia_id")?,
        date_start: constant("date_start_days")?,
        date_end: constant("date_end_days")?,
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
    let base = Base { catalog: Catalog::compress(&pool, &source, &schema())? };
    let compress_ms = compress_started.elapsed().as_secs_f64() * 1000.0;
    let declared_bytes = source.declared_bytes();
    let sf = source.manifest()["sf"].clone();
    let nation_rows = base.rows("nation.n_nationkey");
    let region_rows = base.rows("region.r_regionkey");
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

    let (orders, customers, suppliers, nations, regions) =
        (base.domain("orderkey"), base.domain("custkey"), base.domain("suppkey"), base.domain("nationkey"), base.domain("regionkey"));
    let (order_bits, supplier_bits, nation_bits) = (orders.bits(), suppliers.bits(), nations.bits());
    if order_bits + supplier_bits + nation_bits > 64 {
        return Err("packed answer key exceeds 64 bits for these dictionary sizes".into());
    }
    let specs = [
        IndexSpec {
            name: "nation_by_region",
            key: "nation.n_regionkey",
            domain: regions.len(),
            payload: vec![("nation.n_nationkey", false, nations.bounds())],
        },
        IndexSpec {
            name: "customer_by_nation",
            key: "customer.c_nationkey",
            domain: nations.len(),
            payload: vec![("customer.c_custkey", false, customers.bounds())],
        },
        IndexSpec {
            name: "orders_by_customer",
            key: "orders.o_custkey",
            domain: customers.len(),
            payload: vec![("orders.o_orderkey", false, orders.bounds()), ("orders.o_orderdate", true, base.bounds("orders.o_orderdate"))],
        },
        IndexSpec {
            name: "lineitem_by_order",
            key: "lineitem.l_orderkey",
            domain: orders.len(),
            payload: vec![("lineitem.l_suppkey", false, suppliers.bounds())],
        },
        IndexSpec {
            name: "supplier_by_supplier",
            key: "supplier.s_suppkey",
            domain: suppliers.len(),
            payload: vec![("supplier.s_nationkey", false, nations.bounds())],
        },
    ];
    let counts: Vec<Counts> = specs
        .iter()
        .map(|spec| {
            Ok(Counts {
                rows: Arc::new(Buffer::from_host(pool.clone(), &[base.rows(spec.key) as u64])?),
                offsets: Arc::new(Buffer::from_host(pool.clone(), &[(spec.domain + 1) as u64])?),
            })
        })
        .collect::<Result<_>>()?;

    let module = unsafe { reference::load(pool.context())? };
    let thrust = rccl_thrust::cuda::Program::load(pool.context())?;
    let cub = rccl_cub::cuda::Program::load(pool.context())?;
    let stream = pool.stream();
    let exec: Exec<'_> = thrust.par_in(stream, pool.resource())?;
    let launch = cub.on(stream)?;
    // Setup: RCCL's reusable radix controls, one per index.
    let mut sorts = specs
        .iter()
        .map(|spec| rccl_cub::device_radix_sort_pairs_buffer_workspace::<u32, u32>(&launch, base.rows(spec.key)))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let answer_bits = order_bits + supplier_bits + nation_bits;
    let segments_launch = module.prepare_segments(LaunchConfig1D::new(1, 256, 0))?;
    let expand_launch = module.prepare_expand(LaunchConfig1D::new(EXPAND_GRID, 256, EXPAND_SHARED_BYTES))?;
    let timing = Some(cuda_core::sys::CUevent_flags_enum_CU_EVENT_DEFAULT);
    let segment_capacity = (region_rows * nation_rows).max(1);

    let mut workspace = Workspace::new(&pool, &launch, segment_capacity, capacity)?;
    let mut retries = 0;
    let mut samples: Vec<Sample> = Vec::new();
    let mut checked = false;
    while samples.len() < repeat + 1 {
        let w = &mut workspace;
        stream.synchronize()?;
        let wall = Instant::now();
        let start = stream.record_event(timing)?;
        let mut built = Vec::new();
        let mut index_events = Vec::new();
        for ((spec, count), sort) in specs.iter().zip(&counts).zip(&mut sorts) {
            built.push(build_index(&module, &exec, &launch, sort, &pool, &base, spec, count)?);
            index_events.push(stream.record_event(timing)?);
        }
        let view = |index: usize, column: usize| unsafe {
            if column == 0 { built[index].offsets.view() } else { built[index].payload[column - 1].view() }
        };
        let ix = Indexes {
            region_id: base.view("region.r_regionkey"),
            region_name: base.view("region.r_name"),
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
            workspace = Workspace::new(&pool, &launch, segment_capacity, capacity)?;
            continue;
        }
        let rows = emitted as usize;
        let mut keys = temporary::<u64>(stream, rows)?;
        let mut alternate = temporary::<u64>(stream, rows)?;
        if rows > 0 {
            unsafe {
                cuda_core::memory::memcpy_dtod_async(keys.cu_deviceptr(), w.answers.cu_deviceptr(), rows * 8, stream.cu_stream())?;
            }
        }
        let mut unique = temporary::<u64>(stream, rows)?;
        let mut selected = temporary::<u64>(stream, 1)?;
        {
            let mut buffers = DoubleBuffer::new(&mut keys, &mut alternate).ok_or("radix double buffer extents differ")?;
            rccl_cub::device_radix_sort_keys_buffer(&launch, &mut buffers, 0..answer_bits, &mut w.sort)?;
            unique_copy_by(&exec, buffers.current().view(), unique.view_mut(), &mut selected, |a: u64, b: u64| a == b)?;
        }
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
                        orders.value((key >> order_shift) as u32),
                        suppliers.value(((key >> nation_bits) & supplier_mask) as u32),
                        nations.value((key & nation_mask) as u32),
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
        release(stream, alternate)?;
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
        .catalog
        .tables()
        .flat_map(|(table, stored)| {
            stored.columns.iter().map(move |c| {
                json!({"name": format!("{table}.{}", c.name), "rows": c.column.capacity(), "encoded_bytes": c.column.bytes()})
            })
        })
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
            "compressed_base_bytes": base.catalog.bytes(),
            "index_bytes": samples[0].index_bytes,
            "dictionary_ids": {
                "orderkey": orders.len(), "custkey": customers.len(), "suppkey": suppliers.len(),
                "nationkey": nations.len(), "regionkey": regions.len(),
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
