use crate::manifest::{Manifest, Result};
use cq_gpu_runtime::{
    Buffer, Codec, ColumnView, DeviceColumns, Dictionary, EncodedColumn, For, Pool,
};
use gpu_device_columns::{
    scan_table::{Domain, DeviceScanTable, FamilyId, OrdinalRow, ScanSlot},
    OutputCount, Table,
};
use std::{collections::BTreeMap, sync::Arc};

/// Recorded facts of one authored blocked column (P6 C): its Structures
/// family, domain, geometry and summary policy, and whether a recorded tile
/// scan demands its upload. Generated bindings print these from the records.
#[derive(Clone, Copy, Debug)]
pub struct ScanSpec {
    pub family: FamilyId,
    pub domain: Domain,
    pub block_rows: usize,
    pub summaries: bool,
    pub upload: bool,
}

/// Typed rows whose columns load into retained S1 scan slots. The manifest
/// stores each value's Scalar word; the slot encodes its declared ordinal.
pub trait ScanRow: OrdinalRow {
    fn slots(
        manifest: &Manifest,
        pool: &Arc<Pool>,
        table: &str,
        names: &[&str],
        specs: &[ScanSpec],
    ) -> Result<Vec<ScanSlot>>;
}

fn scan_slot<T>(
    manifest: &Manifest,
    pool: &Arc<Pool>,
    table: &str,
    name: &str,
    spec: &ScanSpec,
) -> Result<ScanSlot>
where
    T: gpu_datalog_runtime::Scalar<Logical = T> + cq_storage::scan::OrdinalDecode + 'static,
{
    fn encode<T, F>(words: &[i64], spec: &ScanSpec) -> Result<cq_storage::scan::ScanColumn<T, F>>
    where
        T: gpu_datalog_runtime::Scalar<Logical = T> + cq_storage::scan::OrdinalDecode,
        F: cq_storage::scan::Family,
    {
        let rows = u32::try_from(spec.block_rows)
            .ok()
            .and_then(cq_storage::scan::BlockRows::new)
            .ok_or("invalid block geometry")?;
        let mut builder = cq_storage::scan::ScanColumnBuilder::<T, F>::with_domain(
            rows,
            spec.summaries,
            false,
            spec.domain,
        )
        .map_err(|error| format!("{error:?}"))?;
        for &word in words {
            builder.push(T::decode(word as i128));
        }
        Ok(builder.finish())
    }
    let words = manifest.read_column(table, name)?;
    match spec.family {
        FamilyId::Plain => ScanSlot::new(
            pool,
            Arc::new(encode::<T, cq_storage::scan::Plain>(&words, spec)?),
            spec.family,
            spec.domain,
            spec.upload,
        ),
        FamilyId::Bytes => ScanSlot::new(
            pool,
            Arc::new(encode::<T, cq_storage::scan::Bytes>(&words, spec)?),
            spec.family,
            spec.domain,
            spec.upload,
        ),
    }
}

macro_rules! scan_rows {
    ($($name:ident $index:tt),+) => {
        impl<$($name),+> ScanRow for ($($name,)+)
        where
            $($name: gpu_datalog_runtime::Scalar<Logical = $name> + cq_storage::scan::OrdinalDecode + 'static),+
        {
            fn slots(
                manifest: &Manifest,
                pool: &Arc<Pool>,
                table: &str,
                names: &[&str],
                specs: &[ScanSpec],
            ) -> Result<Vec<ScanSlot>> {
                if names.len() != Self::WIDTH || specs.len() != Self::WIDTH {
                    return Err("scan binding width differs from its row type".into());
                }
                Ok(vec![$(scan_slot::<$name>(manifest, pool, table, names[$index], &specs[$index])?),+])
            }
        }
    };
}
scan_rows!(A 0);
scan_rows!(A 0, B 1);
scan_rows!(A 0, B 1, C 2);
scan_rows!(A 0, B 1, C 2, D 3);
scan_rows!(A 0, B 1, C 2, D 3, E 4);
scan_rows!(A 0, B 1, C 2, D 3, E 4, G 5);
scan_rows!(A 0, B 1, C 2, D 3, E 4, G 5, H 6);
scan_rows!(A 0, B 1, C 2, D 3, E 4, G 5, H 6, I 7);
scan_rows!(A 0, B 1, C 2, D 3, E 4, G 5, H 6, I 7, J 8);
scan_rows!(A 0, B 1, C 2, D 3, E 4, G 5, H 6, I 7, J 8, K 9);
scan_rows!(A 0, B 1, C 2, D 3, E 4, G 5, H 6, I 7, J 8, K 9, L 10);
scan_rows!(A 0, B 1, C 2, D 3, E 4, G 5, H 6, I 7, J 8, K 9, L 10, M 11);

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

    /// Load an authored blocked relation (P6 C): every column is host-encoded
    /// once into its declared S1 family and retained; only recorded demanded
    /// columns cross to the device, through pinned staging, without a raw
    /// upload or device encoding pass.
    pub fn bind_scan<Row: ScanRow>(
        &mut self,
        table: &str,
        names: &[&str],
        specs: &[ScanSpec],
    ) -> Result<DeviceScanTable<Row>> {
        let relation = self.manifest.relations.get(table).ok_or("missing table")?;
        let slots = Row::slots(self.manifest, &self.pool, table, names, specs)?;
        DeviceScanTable::new(self.pool.clone(), relation.rows, slots)
    }
}
