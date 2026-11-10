use crate::{
    cache as stored,
    manifest::{Manifest, Result},
};
use cq_gpu_runtime::{Buffer, ColumnView, DeviceColumns, Dictionary, EncodedColumn, For, Pool};
use gpu_device_columns::{BaseLayout, Bounds, OutputCount, Table, pack_for, native::NativeView};
use cuda_core::{CudaEvent, PinnedHostBuffer};
use sha2::{Digest, Sha256};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::Instant,
};

pub fn parse_layout(name: &str) -> Result<BaseLayout> {
    match name {
        "for" => Ok(BaseLayout::For),
        "plain64" => Ok(BaseLayout::Plain64),
        "native" => Ok(BaseLayout::Native),
        "hostfor" => Ok(BaseLayout::HostFor),
        "delta" => Ok(BaseLayout::Delta),
        _ => Err("--base-codec requires for, plain64, native, hostfor or delta".into()),
    }
}

pub fn layout_name(layout: BaseLayout) -> &'static str {
    match layout {
        BaseLayout::For => "for",
        BaseLayout::Plain64 => "plain64",
        BaseLayout::Native => "native",
        BaseLayout::HostFor => "hostfor",
        BaseLayout::Delta => "delta",
    }
}

static BASE_LAYOUT: OnceLock<BaseLayout> = OnceLock::new();

/// Select the base-column layout once per process, before any load.
pub fn set_base_layout(layout: BaseLayout) -> Result<()> {
    BASE_LAYOUT
        .set(layout)
        .map_err(|_| "base codec is already selected".into())
}

pub fn base_layout() -> BaseLayout {
    *BASE_LAYOUT.get_or_init(|| BaseLayout::For)
}

static BASE_CACHE: OnceLock<PathBuf> = OnceLock::new();

/// Persist `hostfor` manifest-loader columns under `directory` and load
/// valid files from it instead of the manifest's raw columns.
pub fn set_base_cache(directory: PathBuf) -> Result<()> {
    BASE_CACHE
        .set(directory)
        .map_err(|_| "base cache is already selected".into())
}

/// Reject option combinations before any query runs.
pub fn check_options() -> Result<()> {
    if BASE_CACHE.get().is_some() && base_layout() != BaseLayout::HostFor {
        return Err("--base-cache requires --base-codec hostfor".into());
    }
    Ok(())
}

/// Untimed load accounting: native file reads, device set verification and
/// upload/encoding. Host packing is exclusive to the HostFor measurement mode.
#[derive(Default)]
struct LoadReport {
    columns: usize,
    unprojected: usize,
    cache_hits: usize,
    raw_reads: usize,
    source_bytes: usize,
    resident_bytes: usize,
    peak_device_bytes: usize,
    peak_reserved_device_bytes: usize,
    device_bytes_after_load: usize,
    reserved_device_bytes_after_load: usize,
    read_ms: f64,
    verify_ms: f64,
    pack_ms: f64,
    upload_encode_ms: f64,
    cache_write_ms: f64,
    catalog_ms: f64,
}

thread_local! {
    static REPORT: RefCell<LoadReport> = RefCell::default();
}

fn report(update: impl FnOnce(&mut LoadReport)) {
    REPORT.with(|report| update(&mut report.borrow_mut()));
}

fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

/// Take this thread's accumulated base-column load report.
pub fn take_load_report() -> Value {
    let report = REPORT.with(|report| std::mem::take(&mut *report.borrow_mut()));
    json!({
        "codec": layout_name(base_layout()),
        "cache": BASE_CACHE.get(),
        "columns": report.columns,
        "unprojected": report.unprojected,
        "cache_hits": report.cache_hits,
        "raw_reads": report.raw_reads,
        "source_bytes": report.source_bytes,
        "resident_bytes": report.resident_bytes,
        "peak_device_bytes": report.peak_device_bytes,
        "peak_reserved_device_bytes": report.peak_reserved_device_bytes,
        "device_bytes_after_load": report.device_bytes_after_load,
        "reserved_device_bytes_after_load": report.reserved_device_bytes_after_load,
        "read_ms": report.read_ms,
        "verify_ms": report.verify_ms,
        "pack_ms": report.pack_ms,
        "upload_encode_ms": report.upload_encode_ms,
        "catalog_ms": report.catalog_ms,
        "cache_write_ms": report.cache_write_ms,
        "scope": "base columns built for one query, outside query timings; columns/resident_bytes = encoded source and selected columns constructed; unprojected = full source columns (cacheable); source_bytes = native stored bytes read (catalog: i64 input bytes); read_ms = file I/O into reusable pinned staging, or HostFor/cache reads; verify_ms = exact device projected-set deduplication over encoded columns including synchronization; upload_encode_ms = staging/H2D/device bounds and encoding, excluding overlapped file I/O (includes optional key SHA); pack_ms = HostFor host packing only; peak_device_bytes/peak_reserved_device_bytes = pool live/reserved high-water marks observed during loading, excluding external CUB control buffers and CUDA context; catalog_ms = whole tpch_domains Catalog build",
    })
}

/// Generated `tpch_domains` loads: read the complete manifest source.
pub fn load_source(root: &Path) -> Result<gpu_compress::Source> {
    let started = Instant::now();
    let source = gpu_compress::Source::load_manifest(root)?;
    report(|report| report.read_ms += ms(started));
    Ok(source)
}

/// Generated `tpch_domains` loads: build the catalog in the selected layout.
pub fn compress_catalog(
    pool: &Arc<Pool>,
    source: &gpu_compress::Source,
    schema: &gpu_compress_host::Schema,
) -> Result<gpu_compress::Catalog> {
    let started = Instant::now();
    let catalog = gpu_compress::Catalog::compress_with(pool, source, schema, base_layout())?;
    pool.stream().synchronize()?;
    let elapsed = ms(started);
    report(|report| {
        for (_, table) in catalog.tables() {
            report.columns += table.columns.len();
            report.source_bytes += table.columns.len() * table.rows * 8;
        }
        report.resident_bytes += catalog.bytes();
        report.catalog_ms += elapsed;
    });
    Ok(catalog)
}

/// One manifest-driven loader for every query. Only referenced columns are read;
/// aliases of the same complete projection share immutable resident tables.
pub struct Loader<'a> {
    manifest: &'a Manifest,
    pool: Arc<Pool>,
    tables: BTreeMap<String, CachedTable>,
    staging: RefCell<NativeStaging>,
}

#[derive(Default)]
struct CachedTable {
    columns: BTreeMap<String, Arc<EncodedColumn>>,
    hashes: BTreeMap<String, String>,
    projections: Vec<CachedProjection>,
}

struct CachedProjection {
    names: Vec<String>,
    storage: Table,
}

/// Reusable, double-buffered native-width file staging. The allocation and
/// outstanding DMA sources live together until the loader is dropped.
struct NativeStaging {
    pool: Arc<Pool>,
    buffers: Vec<PinnedHostBuffer<u8>>,
    ready: Vec<CudaEvent>,
}

struct NativeColumn {
    data: Buffer<u8>,
    element: u32,
    hash: Option<String>,
    read_ms: f64,
    upload_ms: f64,
}

impl NativeStaging {
    fn new(pool: Arc<Pool>) -> Self {
        Self { pool, buffers: Vec::new(), ready: Vec::new() }
    }

    fn read(&mut self, path: &Path, bytes: usize, element: u32, hash: bool) -> Result<NativeColumn> {
        const CHUNK: usize = 8 * 1024 * 1024;
        let started = Instant::now();
        // A previous file read or CUDA submission may have returned an error
        // with a copy still pending. Reusing either first chunk must be safe.
        self.pool.stream().synchronize()?;
        while self.buffers.len() < 2 {
            let buffer = PinnedHostBuffer::zeroed(self.pool.context(), CHUNK)?;
            let ready = self.pool.context().new_event(None)?;
            self.buffers.push(buffer);
            self.ready.push(ready);
        }
        let data = unsafe { Buffer::uninitialized(self.pool.clone(), bytes)? };
        let io_started = Instant::now();
        let mut file = fs::File::open(path)?;
        if file.metadata()?.len() != u64::try_from(bytes)? {
            return Err(format!("{}: byte count does not match manifest", path.display()).into());
        }
        let mut read_ms = io_started.elapsed().as_secs_f64() * 1000.0;
        let mut digest = hash.then(Sha256::new);
        let mut offset = 0;
        let mut chunk = 0;
        while offset < bytes {
            let slot = chunk % 2;
            if chunk >= 2 {
                self.ready[slot].synchronize()?;
            }
            let len = CHUNK.min(bytes - offset);
            let source = self.buffers[slot].as_mut_ptr();
            // The preceding event protects this pinned range from reuse while
            // the previous asynchronous copy is still reading it.
            let staging = unsafe { std::slice::from_raw_parts_mut(source, len) };
            let io_started = Instant::now();
            file.read_exact(staging)?;
            read_ms += io_started.elapsed().as_secs_f64() * 1000.0;
            if let Some(digest) = &mut digest {
                digest.update(&*staging);
            }
            unsafe {
                cuda_core::simt::memory::memcpy_htod_async(
                    data.cu_deviceptr() + offset as u64,
                    source,
                    len,
                    self.pool.stream().cu_stream(),
                )?;
            }
            self.ready[slot].record(self.pool.stream())?;
            offset += len;
            chunk += 1;
        }
        // Detect concurrent length changes as the old whole-file reader did.
        let io_started = Instant::now();
        if file.read(&mut [0])? != 0 {
            return Err(format!("{}: byte count does not match manifest", path.display()).into());
        }
        read_ms += io_started.elapsed().as_secs_f64() * 1000.0;
        self.pool.stream().synchronize()?;
        Ok(NativeColumn {
            data,
            element,
            hash: digest.map(|digest| format!("{:x}", digest.finalize())),
            read_ms,
            upload_ms: (started.elapsed().as_secs_f64() * 1000.0 - read_ms).max(0.0),
        })
    }
}

impl Drop for NativeStaging {
    fn drop(&mut self) {
        if let Err(error) = self.pool.stream().synchronize() {
            self.pool.context().record_err::<()>(Err(error));
            // Failed completion cannot authorize freeing a live DMA source.
            for buffer in self.buffers.drain(..) {
                std::mem::forget(buffer);
            }
        }
    }
}


/// One raw manifest column; `identity` is observed before the read when the
/// column may be persisted, so a packed file never pairs new identity with
/// old values.
struct RawColumn {
    // Host values exist only for the explicit HostFor measurement option.
    values: Option<Vec<i64>>,
    data: Option<Buffer<u8>>,
    element: u32,
    hash: Option<String>,
    identity: Option<stored::Identity>,
}

impl RawColumn {
    fn view(&self) -> NativeView {
        NativeView { data: self.data.as_ref().expect("native input").ptr(), element: self.element }
    }
}

impl<'a> Loader<'a> {
    pub fn new(manifest: &'a Manifest, pool: Arc<Pool>) -> Self {
        Self {
            manifest,
            staging: RefCell::new(NativeStaging::new(pool.clone())),
            pool,
            tables: BTreeMap::new(),
        }
    }

    fn read_raw(&self, table: &str, name: &str, check_key: bool) -> Result<RawColumn> {
        let persist = BASE_CACHE.get().is_some();
        let started = Instant::now();
        let identity = persist
            .then(|| stored::identity(self.manifest, table, name))
            .transpose()?;
        let column = if base_layout() == BaseLayout::HostFor {
            let (values, hash) = self.manifest.read_column_checked(table, name, check_key || persist)?;
            report(|report| report.read_ms += ms(started));
            RawColumn { values: Some(values), data: None, element: gpu_device_columns::ELEMENT_I64, hash, identity }
        } else {
            let descriptor = self.manifest.column(table, name)?;
            let (width, element) = match descriptor.ty.as_str() {
                "i32" => (4, gpu_device_columns::ELEMENT_I32),
                "i64" => (8, gpu_device_columns::ELEMENT_I64),
                other => return Err(format!("unsupported stored type {other}").into()),
            };
            let bytes = self.manifest.relations[table].rows.checked_mul(width).ok_or("column size overflow")?;
            let native = self.staging.borrow_mut().read(
                &self.manifest.root.join(&descriptor.file), bytes, element,
                check_key && descriptor.encoding != "dictionary",
            )?;
            report(|report| {
                report.read_ms += native.read_ms;
                report.upload_encode_ms += native.upload_ms;
            });
            RawColumn { values: None, data: Some(native.data), element: native.element, hash: native.hash, identity }
        };
        let width = if self.manifest.column(table, name)?.ty == "i32" { 4 } else { 8 };
        report(|report| {
            report.raw_reads += 1;
            report.source_bytes += self.manifest.relations[table].rows * width;
        });
        Ok(column)
    }

    #[expect(
        clippy::arc_with_non_send_sync,
        reason = "DeviceColumns/Table require Arc owners for stream-affine allocations; Rc cannot implement that API"
    )]
    pub fn bind<Row: gpu_datalog_runtime::ScalarTuple>(
        &mut self,
        table: &str,
        names: &[&str],
    ) -> Result<DeviceColumns<'static, Row, For>> {
        if names.len() != Row::COLUMNS.len() {
            return Err("typed binding width differs from projection".into());
        }
        let relation = self.manifest.relations.get(table).ok_or("missing table")?;
        if relation.rows >= u32::MAX as usize {
            return Err("relation exceeds device row-ID domain".into());
        }
        let mut resident = self.tables.remove(table).unwrap_or_default();
        let cached = resident.projections.iter().find(|projection| {
            projection
                .names
                .iter()
                .map(String::as_str)
                .eq(names.iter().copied())
        });
        let storage = match cached {
            Some(projection) => Ok(projection.storage.clone()),
            None => self.build(table, names, &mut resident).inspect(|storage| {
                resident.projections.push(CachedProjection {
                    names: names.iter().map(|name| (*name).to_owned()).collect(),
                    storage: storage.clone(),
                });
            }),
        };
        // A failed build keeps every projection cached before it.
        self.tables.insert(table.to_owned(), resident);
        let storage = storage?;
        // SAFETY: generated bindings use the manifest's scalar lane ABI and
        // column order, including transparent Decimal/Date values. All views,
        // columns and count belong to this pool; text is already integer-coded.
        Ok(unsafe {
            DeviceColumns::from_parts(storage, Arc::new(Dictionary::default()), self.pool.clone())
        })
    }

    #[expect(
        clippy::arc_with_non_send_sync,
        reason = "DeviceColumns/Table require Arc owners for stream-affine allocations; Rc cannot implement that API"
    )]
    fn build(&self, table: &str, names: &[&str], resident: &mut CachedTable) -> Result<Table> {
        let relation = &self.manifest.relations[table];
        let layout = base_layout();
        // Formal rule DR-ProjectionReuse: an actually distinct cached subset proves its supersets distinct.
        let proven_set = resident.projections.iter().any(|projection| {
            projection.storage.capacity == relation.rows
                && projection
                    .names
                    .iter()
                    .all(|name| names.contains(&name.as_str()))
        });
        let keys: Vec<_> = relation
            .verified_keys
            .iter()
            .filter(|key| {
                key.set == "verified"
                    && !key.columns.is_empty()
                    && key.rows == relation.rows
                    && key.columns.keys().all(|name| {
                        names.contains(&name.as_str())
                            && self.manifest.column(table, name).is_ok_and(|column| {
                                column.encoding == "plain"
                                    && key.types.get(name) == Some(&column.ty)
                            })
                    })
            })
            .collect();
        let key_column = |name: &str| keys.iter().any(|key| key.columns.contains_key(name));
        let distinct: Vec<&str> = names
            .iter()
            .enumerate()
            .filter(|(index, name)| !names[..*index].contains(name))
            .map(|(_, name)| *name)
            .collect();
        // Keep only one native upload alive at a time. Full encoded columns
        // are the immutable source snapshot for both verification and reuse.
        for &name in &distinct {
            if resident.columns.contains_key(name) {
                continue;
            }
            let started = Instant::now();
            let hit = if let Some(directory) = BASE_CACHE.get() {
                let file = stored::path(directory, self.manifest, table, name);
                let hit = stored::read(&file, self.manifest, table, name)?;
                if let Some(hit) = &hit
                    && key_column(name)
                    && hit.hash.as_ref() != Some(&self.manifest.stored_column_hash(table, name)?)
                {
                    return Err(format!("{}: stale for current {table}.{name} bytes", file.display()).into());
                }
                hit
            } else {
                None
            };
            report(|report| report.read_ms += ms(started));
            let raw = if hit.is_none() {
                Some(self.read_raw(table, name, key_column(name))?)
            } else {
                None
            };
            let started = Instant::now();
            let fresh = if hit.is_none() && layout == BaseLayout::HostFor {
                let source = raw.as_ref().expect("raw HostFor source");
                let values = source.values.as_ref().expect("HostFor values");
                let (headers, payload) = pack_for(values);
                Some(stored::Packed {
                    rows: relation.rows,
                    bounds: Bounds::from_values(values),
                    headers,
                    payload,
                    hash: source.hash.clone(),
                })
            } else {
                None
            };
            let pack_ms = if fresh.is_some() { ms(started) } else { 0.0 };
            let started = Instant::now();
            let packed = hit.as_ref().or(fresh.as_ref());
            let column = match packed {
                Some(packed) => EncodedColumn::from_packed_parts(
                    self.pool.clone(), packed.rows, &packed.headers, &packed.payload, packed.bounds,
                )?,
                None => unsafe {
                    EncodedColumn::from_native(
                        self.pool.clone(), raw.as_ref().expect("native source").view(),
                        relation.rows, None, layout,
                    )?
                },
            };
            self.pool.stream().synchronize()?;
            let upload_encode_ms = ms(started);
            let descriptor = self.manifest.column(table, name)?;
            if descriptor.encoding == "dictionary" {
                let count = self.manifest.dictionary(descriptor)?.len();
                if let Bounds::Inclusive { min, max } = column.bounds()
                    && (min < 0 || max as u64 >= count as u64)
                {
                    return Err(format!("{table}.{name}: out-of-range dictionary code").into());
                }
            }
            let hash = packed.and_then(|packed| packed.hash.as_ref())
                .or_else(|| raw.as_ref().and_then(|raw| raw.hash.as_ref()));
            if let Some(hash) = hash {
                resident.hashes.insert(name.to_owned(), hash.clone());
            }
            let started = Instant::now();
            if let (Some(directory), Some(packed), Some(identity)) = (
                BASE_CACHE.get(), &fresh, raw.as_ref().and_then(|raw| raw.identity.as_ref()),
            ) {
                let file = stored::path(directory, self.manifest, table, name);
                stored::write(&file, self.manifest, table, name, identity, packed)?;
            }
            let cache_write_ms = ms(started);
            report(|report| {
                report.columns += 1;
                report.unprojected += 1;
                report.cache_hits += usize::from(hit.is_some());
                report.resident_bytes += column.bytes();
                report.pack_ms += pack_ms;
                report.upload_encode_ms += upload_encode_ms;
                report.cache_write_ms += cache_write_ms;
            });
            self.report_memory();
            resident.columns.insert(name.to_owned(), Arc::new(column));
            // `raw` is dropped here, before the next file allocates a device
            // buffer; no complete native relation ever exists on the device.
        }
        // Allocate publication descriptors before sort scratch so these tiny
        // long-lived buffers cannot pin otherwise wholly unused sort chunks.
        let device_views = unsafe { Buffer::<ColumnView>::uninitialized(self.pool.clone(), names.len())? };
        let metadata = unsafe { Buffer::<OutputCount>::uninitialized(self.pool.clone(), 1)? };
        // Formal rule IO-GpuVerifiedKey: hash the decoded bytes of every included verified key column.
        // A cached column lacking a checked hash is verified on the device;
        // a fresh file hash must never certify a different cached snapshot.
        let certified = keys.iter().any(|key| {
            key.columns.iter().all(|(name, expected)| resident.hashes.get(name) == Some(expected))
        });
        let selected = if proven_set || certified {
            None
        } else {
            let started = Instant::now();
            let views: Vec<_> = distinct.iter()
                .map(|name| unsafe { resident.columns[*name].view() })
                .collect();
            let selected = unsafe {
                gpu_device_columns::unique::unique_rows(self.pool.clone(), &views, relation.rows)?
            };
            self.pool.stream().synchronize()?;
            report(|report| report.verify_ms += ms(started));
            self.report_memory();
            selected
        };
        let rows = selected.as_ref().map_or(relation.rows, Buffer::len);
        let mut columns: Vec<Arc<EncodedColumn>> = Vec::with_capacity(names.len());
        for (index, &name) in names.iter().enumerate() {
            if let Some(previous) = names[..index].iter().position(|&previous| previous == name) {
                columns.push(columns[previous].clone());
                continue;
            }
            let source = &resident.columns[name];
            if let Some(positions) = &selected {
                let started = Instant::now();
                let column = EncodedColumn::from_selected(
                    self.pool.clone(), source, positions, layout,
                )?;
                self.pool.stream().synchronize()?;
                report(|report| {
                    report.columns += 1;
                    report.resident_bytes += column.bytes();
                    report.upload_encode_ms += ms(started);
                });
                self.report_memory();
                columns.push(Arc::new(column));
            } else {
                columns.push(source.clone());
            }
        }
        let views: Vec<ColumnView> = columns
            .iter()
            .map(|column| unsafe { column.view() })
            .collect();
        unsafe {
            device_views.write_async(&views)?;
            metadata.write_async(&[OutputCount { rows: rows as u64, error: 0, padding: 0 }])?;
        }
        let bounds = columns.iter().map(|column| column.bounds()).collect();
        let storage = Table {
            columns,
            views: Arc::new(device_views),
            metadata: Arc::new(metadata),
            capacity: rows,
            bounds,
            statuses: Vec::new(),
        };
        self.report_memory();
        drop(selected);
        let started = Instant::now();
        self.pool.stream().synchronize()?;
        self.pool.release_unused()?;
        report(|report| report.upload_encode_ms += ms(started));
        self.report_memory();
        Ok(storage)
    }

    fn report_memory(&self) {
        let stats = self.pool.memory_stats();
        report(|report| {
            report.peak_device_bytes = report.peak_device_bytes.max(stats.peak_bytes);
            report.peak_reserved_device_bytes = report.peak_reserved_device_bytes.max(stats.reserved_bytes);
            report.device_bytes_after_load = stats.current_bytes;
            report.reserved_device_bytes_after_load = stats.reserved_bytes;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_projection_dedup_reuse_and_dictionary_validation() -> Result<()> {
        let root = std::env::temp_dir().join(format!("cq-native-load-{}", std::process::id()));
        fs::create_dir_all(&root)?;
        fs::write(root.join("a.bin"), [3_i32, 1, 3, 2].into_iter().flat_map(i32::to_le_bytes).collect::<Vec<_>>())?;
        fs::write(root.join("b.bin"), [7_i64, -9, 7, 8].into_iter().flat_map(i64::to_le_bytes).collect::<Vec<_>>())?;
        fs::write(root.join("id.bin"), [0_i32, 1, 2, 3].into_iter().flat_map(i32::to_le_bytes).collect::<Vec<_>>())?;
        let mut manifest: Manifest = serde_json::from_value(json!({
            "schema_version": 1, "sf": 1, "byte_order": "little",
            "relations": {"r": {"rows": 4, "columns": [
                {"name":"a","type":"i32","logical_type":"integer","encoding":"plain","file":"a.bin"},
                {"name":"b","type":"i64","logical_type":"integer","encoding":"plain","file":"b.bin"},
                {"name":"id","type":"i32","logical_type":"integer","encoding":"plain","file":"id.bin"}
            ]}}
        }))?;
        manifest.root = root.clone();
        let pool = Pool::new(0, 1 << 20, 1 << 29)?;
        let loader = Loader::new(&manifest, pool.clone());
        let mut resident = CachedTable::default();
        let projected = loader.build("r", &["a", "b", "a"], &mut resident)?;
        assert_eq!(projected.capacity, 3);
        assert_eq!(projected.columns[0].to_host()?, vec![3, 1, 2]);
        assert_eq!(projected.columns[1].to_host()?, vec![7, -9, 8]);
        assert!(Arc::ptr_eq(&projected.columns[0], &projected.columns[2]));
        assert_eq!(resident.columns.len(), 2);
        assert_eq!(resident.columns["a"].capacity(), 4);
        let key = loader.build("r", &["id"], &mut resident)?;
        resident.projections.push(CachedProjection { names: vec!["id".into()], storage: key });
        let superset = loader.build("r", &["id", "a", "b"], &mut resident)?;
        assert_eq!(superset.capacity, 4);
        assert_eq!(superset.columns[1].to_host()?, vec![3, 1, 3, 2]);
        drop(loader);
        fs::write(root.join("dictionary.txt"), "zero\none\ntwo\n")?;
        let column = &mut manifest.relations.get_mut("r").unwrap().columns[0];
        column.encoding = "dictionary".into();
        column.dictionary = Some("dictionary.txt".into());
        column.dictionary_entries = Some(3);
        let loader = Loader::new(&manifest, pool);
        assert!(loader.build("r", &["a"], &mut CachedTable::default()).err()
            .is_some_and(|error| error.to_string().contains("out-of-range dictionary code")));
        drop(loader);
        fs::remove_dir_all(root)?;
        Ok(())
    }

    #[test]
    fn pinned_staging_reuses_both_chunks_and_hashes_stored_bytes() -> Result<()> {
        let path = std::env::temp_dir().join(format!("cq-native-staging-{}", std::process::id()));
        // Three chunks force reuse of a pinned range protected by its event.
        let bytes: Vec<u8> = (0..(17 * 1024 * 1024 + 4)).map(|i| (i % 251) as u8).collect();
        fs::write(&path, &bytes)?;
        let pool = Pool::new(0, 1 << 20, 1 << 29)?;
        let mut staging = NativeStaging::new(pool);
        for _ in 0..2 {
            let column = staging.read(&path, bytes.len(), gpu_device_columns::ELEMENT_I32, true)?;
            assert_eq!(column.data.to_host()?, bytes);
            assert_eq!(column.hash, Some(format!("{:x}", Sha256::digest(&bytes))));
        }
        assert_eq!(staging.buffers.len(), 2);
        assert!(staging.read(&path, bytes.len() - 4, gpu_device_columns::ELEMENT_I32, false).is_err());
        fs::remove_file(path)?;
        Ok(())
    }
}
