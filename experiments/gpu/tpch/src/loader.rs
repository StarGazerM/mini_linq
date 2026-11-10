use crate::manifest::{Manifest, Result};
use cq_gpu_runtime::{
    Buffer, Codec, ColumnView, DeviceColumns, Dictionary, EncodedColumn, For, Pool,
};
use gpu_device_columns::{OutputCount, Table};
use std::{collections::BTreeMap, sync::Arc};

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

struct LoadedColumn<'a> {
    name: &'a str,
    values: Vec<i64>,
    hash: Option<String>,
}

impl AsRef<[i64]> for LoadedColumn<'_> {
    fn as_ref(&self) -> &[i64] {
        &self.values
    }
}

impl<'a> Loader<'a> {
    pub fn new(manifest: &'a Manifest, pool: Arc<Pool>) -> Self {
        Self {
            manifest,
            pool,
            tables: BTreeMap::new(),
        }
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
        if !self.tables.contains_key(table) {
            self.tables.insert(table.to_owned(), CachedTable::default());
        }
        let cache = self.tables.get_mut(table).expect("inserted table cache");
        let cached = cache.projections.iter().find(|projection| {
            projection.names.iter().map(String::as_str).eq(names.iter().copied())
        });
        let storage = if let Some(projection) = cached {
            projection.storage.clone()
        } else {
            // Formal rule DR-ProjectionReuse: an actually distinct cached subset proves its supersets distinct.
            let proven_set = cache.projections.iter().any(|projection| {
                projection.storage.capacity == relation.rows
                    && projection.names.iter().all(|name| names.contains(&name.as_str()))
            });
            let mut keys = relation.verified_keys.iter().filter(|key| {
                key.set == "verified" && !key.columns.is_empty() && key.rows == relation.rows
                    && key.columns.keys().all(|name| {
                        names.contains(&name.as_str())
                            && self.manifest.column(table, name).is_ok_and(|column| {
                                column.encoding == "plain"
                                    && key.types.get(name) == Some(&column.ty)
                            })
                    })
            });
            let loaded = if proven_set {
                Vec::new()
            } else {
                names.iter().enumerate()
                    .filter(|(index, name)| !names[..*index].contains(name))
                    .map(|(_, name)| {
                        self.manifest.read_column_checked(
                            table, name, keys.clone().any(|key| key.columns.contains_key(*name)),
                        ).map(|(values, hash)| LoadedColumn { name, values, hash })
                    })
                    .collect::<Result<Vec<_>>>()?
            };
            // Formal rule IO-GpuVerifiedKey: hash the decoded bytes of every included verified key column.
            let certified = keys.any(|key| key.columns.iter().all(|(name, expected)| {
                loaded.iter().any(|column| column.name == name && column.hash.as_ref() == Some(expected))
            }));
            let selected = if proven_set || certified {
                None
            } else {
                gpu_compress_host::unique_rows(&loaded, relation.rows)?
            };
            let rows = selected.as_ref().map_or(relation.rows, Vec::len);
            let mut values = loaded.into_iter().map(|column| column.values);
            let mut scratch = Vec::new();
            let mut columns: Vec<Arc<EncodedColumn>> = Vec::with_capacity(names.len());
            for (index, &name) in names.iter().enumerate() {
                if let Some(previous) = names[..index].iter().position(|&previous| previous == name) {
                    columns.push(columns[previous].clone());
                    continue;
                }
                let raw = values.next();
                if selected.is_none()
                    && let Some(column) = cache.columns.get(name)
                {
                    columns.push(column.clone());
                    continue;
                }
                let raw = match raw {
                    Some(values) => values,
                    None => self.manifest.read_column(table, name)?,
                };
                let projected = if let Some(selected) = &selected {
                    scratch.clear();
                    scratch.extend(selected.iter().map(|&row| raw[row as usize]));
                    scratch.as_slice()
                } else {
                    raw.as_slice()
                };
                let column = Arc::new(EncodedColumn::from_host(
                    self.pool.clone(), projected, Codec::For,
                )?);
                if selected.is_none() {
                    cache.columns.insert(name.to_owned(), column.clone());
                }
                columns.push(column);
            }
            let views: Vec<ColumnView> = columns
                .iter()
                .map(|column| unsafe { column.view() })
                .collect();
            let bounds = columns.iter().map(|column| column.bounds()).collect();
            let storage = Table {
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
            };
            cache.projections.push(CachedProjection {
                names: names.iter().map(|name| (*name).to_owned()).collect(),
                storage: storage.clone(),
            });
            storage
        };
        // SAFETY: generated bindings use the manifest's scalar lane ABI and
        // column order, including transparent Decimal/Date values. All views,
        // columns and count belong to this pool; text is already integer-coded.
        Ok(unsafe {
            DeviceColumns::from_parts(storage, Arc::new(Dictionary::default()), self.pool.clone())
        })
    }
}
