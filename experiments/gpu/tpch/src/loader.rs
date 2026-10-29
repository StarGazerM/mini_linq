use crate::manifest::{Manifest, Result};
use cq_gpu_runtime::{
    Buffer, Codec, ColumnView, DeviceColumns, Dictionary, EncodedColumn, For, Pool,
};
use gpu_device_columns::{OutputCount, Table};
use std::{collections::BTreeMap, sync::Arc};

/// One manifest-driven loader for every query. Only referenced columns are read;
/// repeated aliases share the same immutable resident column allocations.
pub struct Loader<'a> {
    manifest: &'a Manifest,
    pool: Arc<Pool>,
    columns: BTreeMap<(String, String), Arc<EncodedColumn>>,
}

impl<'a> Loader<'a> {
    pub fn new(manifest: &'a Manifest, pool: Arc<Pool>) -> Self {
        Self {
            manifest,
            pool,
            columns: BTreeMap::new(),
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
        let mut columns = Vec::with_capacity(names.len());
        for &name in names {
            let key = (table.to_owned(), name.to_owned());
            if !self.columns.contains_key(&key) {
                let values = self.manifest.read_column(table, name)?;
                let column = EncodedColumn::from_host(self.pool.clone(), &values, Codec::For)?;
                self.columns.insert(key.clone(), Arc::new(column));
            }
            columns.push(self.columns[&key].clone());
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
                    rows: relation.rows as u64,
                    error: 0,
                    padding: 0,
                }],
            )?),
            capacity: relation.rows,
            bounds,
            statuses: Vec::new(),
        };
        // SAFETY: generated bindings use the manifest's scalar lane ABI and
        // column order, including transparent Decimal/Date values. All views,
        // columns and count belong to this pool; text is already integer-coded.
        Ok(unsafe {
            DeviceColumns::from_parts(storage, Arc::new(Dictionary::default()), self.pool.clone())
        })
    }
}
