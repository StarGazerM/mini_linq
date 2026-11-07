use crate::{
    cache as stored,
    manifest::{Manifest, Result},
};
use cq_gpu_runtime::{
    Buffer, ColumnView, DeviceColumns, Dictionary, EncodedColumn, For, Pool,
};
use gpu_device_columns::{BaseLayout, Bounds, OutputCount, Table, pack_for};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::BTreeMap,
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
        _ => Err("--base-codec requires for, plain64, native or hostfor".into()),
    }
}

pub fn layout_name(layout: BaseLayout) -> &'static str {
    match layout {
        BaseLayout::For => "for",
        BaseLayout::Plain64 => "plain64",
        BaseLayout::Native => "native",
        BaseLayout::HostFor => "hostfor",
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

/// Untimed load accounting for one query's base columns: host reads, set
/// verification, host packing, and synchronous upload (plus device encode).
#[derive(Default)]
struct LoadReport {
    columns: usize,
    unprojected: usize,
    cache_hits: usize,
    raw_reads: usize,
    source_bytes: usize,
    resident_bytes: usize,
    read_ms: f64,
    verify_ms: f64,
    pack_ms: f64,
    upload_encode_ms: f64,
    cache_write_ms: f64,
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
        "read_ms": report.read_ms,
        "verify_ms": report.verify_ms,
        "pack_ms": report.pack_ms,
        "upload_encode_ms": report.upload_encode_ms,
        "cache_write_ms": report.cache_write_ms,
        "scope": "base columns built for one query, outside query timings; columns = resident columns built (unprojected: full source columns, the only cacheable ones); source_bytes = rows x 8 (raw i64 upload size); read_ms = raw manifest columns or packed files; verify_ms = projected-set deduplication; Catalog (tpch_domains) loads report read_ms and upload_encode_ms only",
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
        report.upload_encode_ms += elapsed;
    });
    Ok(catalog)
}

/// One manifest-driven loader for every query. Only referenced columns are read;
/// aliases of the same complete projection share immutable resident tables.
pub struct Loader<'a> {
    manifest: &'a Manifest,
    pool: Arc<Pool>,
    tables: BTreeMap<String, CachedTable>,
}

#[derive(Default)]
struct CachedTable {
    columns: BTreeMap<String, Arc<EncodedColumn>>,
    projections: Vec<CachedProjection>,
}

struct CachedProjection {
    names: Vec<String>,
    storage: Table,
}

/// One raw manifest column; `identity` is observed before the read when the
/// column may be persisted, so a packed file never pairs new identity with
/// old values.
struct RawColumn {
    values: Vec<i64>,
    hash: Option<String>,
    identity: Option<stored::Identity>,
}

impl<'a> Loader<'a> {
    pub fn new(manifest: &'a Manifest, pool: Arc<Pool>) -> Self {
        Self {
            manifest,
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
        let (values, hash) = self
            .manifest
            .read_column_checked(table, name, check_key || persist)?;
        report(|report| {
            report.read_ms += ms(started);
            report.raw_reads += 1;
        });
        Ok(RawColumn {
            values,
            hash,
            identity,
        })
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
            projection.names.iter().map(String::as_str).eq(names.iter().copied())
        });
        let storage = if let Some(projection) = cached {
            projection.storage.clone()
        } else {
            let storage = self.build(table, names, &mut resident)?;
            resident.projections.push(CachedProjection {
                names: names.iter().map(|name| (*name).to_owned()).collect(),
                storage: storage.clone(),
            });
            storage
        };
        self.tables.insert(table.to_owned(), resident);
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
                && projection.names.iter().all(|name| names.contains(&name.as_str()))
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
                                column.encoding == "plain" && key.types.get(name) == Some(&column.ty)
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
        // Packed files hold complete source columns and their stored-byte hash.
        let mut packed: BTreeMap<&str, stored::Packed> = BTreeMap::new();
        if let Some(directory) = BASE_CACHE.get() {
            for &name in &distinct {
                if resident.columns.contains_key(name) {
                    continue;
                }
                let started = Instant::now();
                let file = stored::path(directory, self.manifest, table, name);
                if let Some(hit) = stored::read(&file, self.manifest, table, name)? {
                    packed.insert(name, hit);
                }
                report(|report| report.read_ms += ms(started));
            }
        }
        let mut raw: BTreeMap<&str, RawColumn> = BTreeMap::new();
        if !proven_set {
            for &name in &distinct {
                if !packed.contains_key(name) {
                    raw.insert(name, self.read_raw(table, name, key_column(name))?);
                }
            }
        }
        // Formal rule IO-GpuVerifiedKey: hash the decoded bytes of every included verified key column.
        let hash = |name: &str| {
            raw.get(name)
                .and_then(|column| column.hash.as_ref())
                .or_else(|| packed.get(name).and_then(|column| column.hash.as_ref()))
        };
        let certified = keys.iter().any(|key| {
            key.columns
                .iter()
                .all(|(name, expected)| hash(name) == Some(expected))
        });
        let selected = if proven_set || certified {
            None
        } else {
            // Deduplication needs every projected value, including packed hits.
            for &name in &distinct {
                if !raw.contains_key(name) {
                    raw.insert(name, self.read_raw(table, name, key_column(name))?);
                }
            }
            let started = Instant::now();
            let columns: Vec<&[i64]> = distinct
                .iter()
                .map(|name| raw[name].values.as_slice())
                .collect();
            let selected = gpu_compress_host::unique_rows(&columns, relation.rows)?;
            report(|report| report.verify_ms += ms(started));
            selected
        };
        let rows = selected.as_ref().map_or(relation.rows, Vec::len);
        let mut scratch = Vec::new();
        let mut columns: Vec<Arc<EncodedColumn>> = Vec::with_capacity(names.len());
        for (index, &name) in names.iter().enumerate() {
            if let Some(previous) = names[..index].iter().position(|&previous| previous == name) {
                columns.push(columns[previous].clone());
                continue;
            }
            if selected.is_none()
                && let Some(column) = resident.columns.get(name)
            {
                columns.push(column.clone());
                continue;
            }
            let hit = selected.is_none().then(|| packed.remove(name)).flatten();
            let source = match (&hit, raw.remove(name)) {
                (Some(_), _) => None,
                (None, Some(column)) => Some(column),
                (None, None) => Some(self.read_raw(table, name, false)?),
            };
            let projected = match (&source, &selected) {
                (None, _) => &[][..],
                (Some(column), Some(selected)) => {
                    scratch.clear();
                    scratch.extend(selected.iter().map(|&row| column.values[row as usize]));
                    scratch.as_slice()
                }
                (Some(column), None) => column.values.as_slice(),
            };
            let started = Instant::now();
            let fresh = match (hit.is_none(), layout) {
                (true, BaseLayout::HostFor) => {
                    let (headers, payload) = pack_for(projected);
                    Some(stored::Packed {
                        rows,
                        bounds: Bounds::from_values(projected),
                        headers,
                        payload,
                        hash: source.as_ref().and_then(|column| column.hash.clone()),
                    })
                }
                _ => None,
            };
            let pack_ms = if fresh.is_some() { ms(started) } else { 0.0 };
            let started = Instant::now();
            let column = match hit.as_ref().or(fresh.as_ref()) {
                Some(packed) => EncodedColumn::from_packed_parts(
                    self.pool.clone(),
                    packed.rows,
                    &packed.headers,
                    &packed.payload,
                    packed.bounds,
                )?,
                None => EncodedColumn::from_host_layout(self.pool.clone(), projected, layout)?,
            };
            self.pool.stream().synchronize()?;
            let upload_encode_ms = ms(started);
            let started = Instant::now();
            if let (Some(directory), Some(packed), None, Some(identity)) = (
                BASE_CACHE.get(),
                &fresh,
                &selected,
                source.as_ref().and_then(|column| column.identity.as_ref()),
            ) {
                let file = stored::path(directory, self.manifest, table, name);
                stored::write(&file, self.manifest, table, name, identity, packed)?;
            }
            let cache_write_ms = ms(started);
            report(|report| {
                report.columns += 1;
                report.unprojected += usize::from(selected.is_none());
                report.cache_hits += usize::from(hit.is_some());
                report.source_bytes += column.capacity() * 8;
                report.resident_bytes += column.bytes();
                report.pack_ms += pack_ms;
                report.upload_encode_ms += upload_encode_ms;
                report.cache_write_ms += cache_write_ms;
            });
            let column = Arc::new(column);
            if selected.is_none() {
                resident.columns.insert(name.to_owned(), column.clone());
            }
            columns.push(column);
        }
        let views: Vec<ColumnView> = columns
            .iter()
            .map(|column| unsafe { column.view() })
            .collect();
        let bounds = columns.iter().map(|column| column.bounds()).collect();
        Ok(Table {
            columns,
            views: Arc::new(Buffer::from_host(self.pool.clone(), &views)?),
            metadata: Arc::new(Buffer::from_host(
                self.pool.clone(),
                &[OutputCount {
                    rows: rows as u64,
                    error: 0,
                    padding: 0,
                }],
            )?),
            capacity: rows,
            bounds,
            statuses: Vec::new(),
        })
    }
}
