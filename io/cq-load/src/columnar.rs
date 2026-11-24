use std::path::Path;

use cq_storage::scan::{BlockRows, Family, OrdinalDecode, ScanColumn, ScanColumnBuilder, ScanColumns, ScanTable};

use crate::{Batch, Csv, CsvOptions, Dictionaries, Load, Result};

/// Generated per-column ingestion; no tuple row collection is involved.
///
/// Families and summary policy are authored by the caller. Source batches need
/// not align with storage blocks: each builder retains only its unfinished tail.
pub trait ColumnarRelation {
    const COLUMNS: &'static [&'static str];
    type Table;
    type Builders;

    fn builders(block_rows: BlockRows, summaries: bool) -> Self::Builders;
    fn append(batch: &Batch<'_>, builders: &mut Self::Builders) -> Result<()>;
    fn finish(builders: Self::Builders, dictionaries: &mut Dictionaries) -> Self::Table;
}

/// Columnar relation-set loading, independent of compiler representation selection.
/// Complete encoded rows are deduplicated at publication, after dictionary remapping.
pub struct ColumnarLoader<S>(std::marker::PhantomData<S>);

impl<S, C> ColumnarLoader<S>
where
    S: ColumnarRelation<Table = ScanTable<C>>,
    C: ScanColumns,
{
    pub fn csv(
        path: impl AsRef<Path>,
        options: CsvOptions,
        block_rows: BlockRows,
        summaries: bool,
    ) -> Result<S::Table> {
        Self::csv_interned(
            path,
            options,
            block_rows,
            summaries,
            &mut Dictionaries::default(),
        )
    }

    pub fn csv_interned(
        path: impl AsRef<Path>,
        options: CsvOptions,
        block_rows: BlockRows,
        summaries: bool,
        dictionaries: &mut Dictionaries,
    ) -> Result<S::Table> {
        let mut source = Csv::<S>::open_columns(path, options, S::COLUMNS)?;
        let mut builders = S::builders(block_rows, summaries);
        while let Some(batch) = source.next_with(|batch| S::append(batch, &mut builders)) {
            batch?;
        }
        Self::finish(builders, dictionaries)
    }

    #[cfg(feature = "parquet")]
    pub fn parquet(
        path: impl AsRef<Path>,
        options: crate::ParquetOptions,
        block_rows: BlockRows,
        summaries: bool,
    ) -> Result<S::Table> {
        Self::parquet_interned(
            path,
            options,
            block_rows,
            summaries,
            &mut Dictionaries::default(),
        )
    }

    #[cfg(feature = "parquet")]
    pub fn parquet_interned(
        path: impl AsRef<Path>,
        options: crate::ParquetOptions,
        block_rows: BlockRows,
        summaries: bool,
        dictionaries: &mut Dictionaries,
    ) -> Result<S::Table> {
        let mut source = crate::Parquet::<S>::open_columns(path, options, S::COLUMNS)?;
        let mut builders = S::builders(block_rows, summaries);
        while let Some(batch) = source.next_with(|batch| S::append(batch, &mut builders)) {
            batch?;
        }
        Self::finish(builders, dictionaries)
    }

    /// Encode borrowed Arrow columns without a Parquet file or tuple staging.
    /// Source errors and schema/conversion errors stop batch consumption at once.
    #[cfg(feature = "parquet")]
    pub fn arrow_batches(
        batches: impl IntoIterator<Item = Result<arrow_array::RecordBatch>>,
        block_rows: BlockRows,
        summaries: bool,
    ) -> Result<S::Table> {
        Self::arrow_batches_interned(batches, block_rows, summaries, &mut Dictionaries::default())
    }

    #[cfg(feature = "parquet")]
    pub fn arrow_batches_interned(
        batches: impl IntoIterator<Item = Result<arrow_array::RecordBatch>>,
        block_rows: BlockRows,
        summaries: bool,
        dictionaries: &mut Dictionaries,
    ) -> Result<S::Table> {
        let file = Path::new("<arrow>");
        let mut builders = S::builders(block_rows, summaries);
        let mut schema = None;
        let mut mapping = Vec::new();
        let mut first_row = 1;
        for batch in batches {
            let batch = batch?;
            let batch_schema = batch.schema();
            if schema.as_ref() != Some(&batch_schema) {
                let names = batch_schema
                    .fields()
                    .iter()
                    .map(|field| field.name().as_str())
                    .collect::<Vec<_>>();
                mapping = crate::column_mapping(file, &names, S::COLUMNS)?;
                schema = Some(batch_schema);
            }
            let source = Batch {
                file,
                columns: S::COLUMNS,
                mapping: &mapping,
                data: crate::BatchData::Parquet(&batch, first_row),
            };
            S::append(&source, &mut builders)?;
            first_row += batch.num_rows() as u64;
        }
        Self::finish(builders, dictionaries)
    }

    fn finish(builders: S::Builders, dictionaries: &mut Dictionaries) -> Result<S::Table> {
        let mut pending = Dictionaries::default();
        let table = S::finish(builders, &mut pending).into_set();
        dictionaries.commit(pending)?;
        Ok(table)
    }
}

/// Load-time field state for an explicitly selected storage family.
/// Ordinary fields stream into bounded block builders. Interned fields retain
/// provisional integer codes only, then remap directly into their final blocks.
pub trait ColumnarField<F: Family>: crate::InternedField + OrdinalDecode {
    type Builder;

    fn builder(block_rows: BlockRows, summaries: bool) -> Self::Builder;
    fn append(batch: &Batch<'_>, index: usize, builder: &mut Self::Builder) -> Result<()>;
    fn finish(
        column: &str,
        dictionaries: &mut Dictionaries,
        builder: Self::Builder,
    ) -> ScanColumn<Self, F>;
}

impl<T: Load + OrdinalDecode, F: Family> ColumnarField<F> for T {
    type Builder = ScanColumnBuilder<T, F>;

    fn builder(block_rows: BlockRows, summaries: bool) -> Self::Builder {
        ScanColumnBuilder::new(block_rows, summaries)
    }

    fn append(batch: &Batch<'_>, index: usize, builder: &mut Self::Builder) -> Result<()> {
        batch.append_column(index, builder)
    }

    fn finish(_: &str, _: &mut Dictionaries, builder: Self::Builder) -> ScanColumn<Self, F> {
        builder.finish()
    }
}
