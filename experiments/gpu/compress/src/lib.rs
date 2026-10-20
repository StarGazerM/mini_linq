//! Pre-query compression: the stored representation that queries bind to.
//!
//! The compiler makes no compression decision. This crate reads source columns
//! at their declared types, interns every join domain into one order-preserving
//! dictionary of dense u32 IDs, FOR bit-packs every column per 1024-row vector
//! and keeps the inverse maps for decoding answers. Queries declare relations
//! over the stored types: domain ID newtypes (`domain_id!`) and `i32`.
//!
//! Contract consumed by query code, per column: logical kind (domain ID or
//! i32), value bounds, row count and the resident `EncodedColumn` whose device
//! row access (`decode_u32` / `decode`) the generated kernels call. Per domain:
//! its size, ID width and inverse map.

use gpu_device_columns::{
    Bounds, Buffer, Codec, ColumnView, DeviceColumns, Dictionary, EncodedColumn, OutputCount, Pool, Table,
};
use serde_json::Value;
use std::{collections::BTreeMap, error::Error, fs, path::Path, sync::Arc};

pub type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

/// Declare a domain ID newtype: a dense u32 dictionary ID of one join domain.
#[macro_export]
macro_rules! domain_id {
    ($(#[$meta:meta])* $vis:vis $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(transparent)]
        $vis struct $name(pub u32);
    };
}

/// Source columns read at their declared types (`i64` or `i32`), host-only.
pub struct Source {
    columns: BTreeMap<String, Vec<i64>>,
    declared_bytes: usize,
    manifest: Value,
}

impl Source {
    /// Read `manifest.json` and its little-endian column files.
    pub fn load_manifest(dir: &Path) -> Result<Self> {
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
        Ok(Self { columns, declared_bytes, manifest })
    }
    pub fn col(&self, name: &str) -> Result<&[i64]> {
        self.columns.get(name).map(Vec::as_slice).ok_or_else(|| format!("missing column {name}").into())
    }
    pub fn declared_bytes(&self) -> usize {
        self.declared_bytes
    }
    pub fn manifest(&self) -> &Value {
        &self.manifest
    }
}

/// Order-preserving dictionary of one join domain: dense IDs in [0, len).
/// len < u32::MAX keeps u32::MAX free as a sentinel.
pub struct Domain {
    name: String,
    values: Vec<i64>,
}

impl Domain {
    fn new(name: &str, columns: &[&[i64]]) -> Result<Self> {
        let mut values: Vec<i64> = columns.iter().flat_map(|c| c.iter().copied()).collect();
        values.sort_unstable();
        values.dedup();
        if values.len() >= u32::MAX as usize {
            return Err(format!("domain {name} exceeds the u32 ID domain").into());
        }
        Ok(Self { name: name.to_string(), values })
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
    /// Bits of an ID: ceil(log2(len)).
    pub fn bits(&self) -> u32 {
        if self.len() <= 1 { 0 } else { 64 - ((self.len() - 1) as u64).leading_zeros() }
    }
    pub fn bounds(&self) -> Bounds {
        if self.is_empty() { Bounds::Empty } else { Bounds::Inclusive { min: 0, max: self.len() as i64 - 1 } }
    }
    /// Source value of an ID (answer decoding).
    pub fn value(&self, id: u32) -> i64 {
        self.values[id as usize]
    }
    /// ID of a source value, e.g. a query constant; `None` if absent.
    pub fn id(&self, value: i64) -> Option<u32> {
        self.values.binary_search(&value).ok().map(|id| id as u32)
    }
    fn ids(&self, column: &[i64]) -> Vec<u32> {
        column.iter().map(|v| self.values.binary_search(v).expect("value in its domain") as u32).collect()
    }
}

/// Stored kind of one column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Dense u32 ID of the named domain.
    Id(String),
    /// Declared 32-bit integer, stored at that type.
    I32,
}

/// Which source columns share a domain, and every stored table's columns.
pub struct Schema {
    /// Domain name -> member source columns (`table.column`).
    pub domains: Vec<(String, Vec<String>)>,
    /// Table -> (column, kind), in stored column order.
    pub tables: Vec<(String, Vec<(String, Kind)>)>,
}

pub struct CompressedColumn {
    pub name: String,
    pub kind: Kind,
    pub column: Arc<EncodedColumn>,
}

pub struct CompressedTable {
    pub rows: usize,
    pub columns: Vec<CompressedColumn>,
}

/// Resident compressed tables plus their domains.
pub struct Catalog {
    pool: Arc<Pool>,
    domains: BTreeMap<String, Domain>,
    tables: BTreeMap<String, CompressedTable>,
}

impl Catalog {
    /// Intern every domain, then FOR-pack and upload every table column.
    pub fn compress(pool: &Arc<Pool>, source: &Source, schema: &Schema) -> Result<Self> {
        let mut domains = BTreeMap::new();
        for (name, members) in &schema.domains {
            let columns = members.iter().map(|m| source.col(m)).collect::<Result<Vec<_>>>()?;
            domains.insert(name.clone(), Domain::new(name, &columns)?);
        }
        let mut tables = BTreeMap::new();
        for (table, columns) in &schema.tables {
            let mut rows = None;
            let mut stored = Vec::new();
            for (column, kind) in columns {
                let qualified = format!("{table}.{column}");
                let values = source.col(&qualified)?;
                if *rows.get_or_insert(values.len()) != values.len() {
                    return Err(format!("{qualified}: row count differs within {table}").into());
                }
                let encoded = match kind {
                    Kind::Id(domain) => {
                        let domain = domains.get(domain).ok_or_else(|| format!("{qualified}: unknown domain {domain}"))?;
                        if !schema.domains.iter().any(|(d, members)| d == domain.name() && members.contains(&qualified)) {
                            return Err(format!("{qualified} is not a member of domain {}", domain.name()).into());
                        }
                        EncodedColumn::from_host(pool.clone(), &domain.ids(values), Codec::For)?
                    }
                    Kind::I32 => {
                        let values = values.iter().map(|&v| Ok(i32::try_from(v)?)).collect::<Result<Vec<i32>>>()?;
                        EncodedColumn::from_host(pool.clone(), &values, Codec::For)?
                    }
                };
                stored.push(CompressedColumn { name: column.clone(), kind: kind.clone(), column: Arc::new(encoded) });
            }
            tables.insert(table.clone(), CompressedTable { rows: rows.unwrap_or(0), columns: stored });
        }
        Ok(Self { pool: pool.clone(), domains, tables })
    }
    pub fn domain(&self, name: &str) -> Result<&Domain> {
        self.domains.get(name).ok_or_else(|| format!("unknown domain {name}").into())
    }
    pub fn table(&self, name: &str) -> Result<&CompressedTable> {
        self.tables.get(name).ok_or_else(|| format!("unknown table {name}").into())
    }
    pub fn column(&self, table: &str, column: &str) -> Result<&CompressedColumn> {
        self.table(table)?
            .columns
            .iter()
            .find(|c| c.name == column)
            .ok_or_else(|| format!("unknown column {table}.{column}").into())
    }
    /// Device view of a column for hand-written kernels.
    pub fn view(&self, table: &str, column: &str) -> Result<ColumnView> {
        Ok(unsafe { self.column(table, column)?.column.view() })
    }
    pub fn rows(&self, table: &str) -> Result<usize> {
        Ok(self.table(table)?.rows)
    }
    /// Encoded bytes of every resident column.
    pub fn bytes(&self) -> usize {
        self.tables.values().flat_map(|t| &t.columns).map(|c| c.column.bytes()).sum()
    }
    pub fn tables(&self) -> impl Iterator<Item = (&String, &CompressedTable)> {
        self.tables.iter()
    }
    /// Bind a stored table as a resident relation for compiled queries. `Row`
    /// is the tuple of stored types (domain ID newtypes and `i32`). Setup only:
    /// the column descriptors and row count are uploaded synchronously.
    pub fn device_columns<Row>(&self, table: &str) -> Result<DeviceColumns<'static, Row, gpu_device_columns::For>> {
        let stored = self.table(table)?;
        let views: Vec<ColumnView> = stored.columns.iter().map(|c| unsafe { c.column.view() }).collect();
        let metadata = Buffer::from_host(self.pool.clone(), &[OutputCount { rows: stored.rows as u64, error: 0, padding: 0 }])?;
        let table = Table {
            columns: stored.columns.iter().map(|c| c.column.clone()).collect(),
            views: Arc::new(Buffer::from_host(self.pool.clone(), &views)?),
            metadata: Arc::new(metadata),
            capacity: stored.rows,
            bounds: stored.columns.iter().map(|c| c.column.bounds()).collect(),
            statuses: Vec::new(),
        };
        // SAFETY: every column belongs to this pool and was initialized by
        // `compress`; the views and count are owned by `table`; bounds are the
        // columns' own. The string dictionary is unused by integer domains.
        Ok(unsafe { DeviceColumns::from_parts(table, Arc::new(Dictionary::default()), self.pool.clone()) })
    }
}
