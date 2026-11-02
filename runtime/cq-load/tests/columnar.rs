use cq_load::{
    Batch, ColumnarField, ColumnarLoader, ColumnarRelation, CsvOptions, Date, Decimal,
    Dictionaries, Relation, Result, StringCode,
};
use cq_storage::scan::{
    BlockRows, Bytes, Family, OrdinalDecode, Plain, ScanColumn, ScanColumnBuilder, ScanTable,
    VECTOR_ROWS,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture(PathBuf);
impl Fixture {
    fn path(extension: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self(std::env::temp_dir().join(format!(
            "cq-columnar-{}-{}.{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed), extension,
        )))
    }
    fn csv(text: &str) -> Self {
        let file = Self::path("csv");
        std::fs::write(&file.0, text).unwrap();
        file
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct Numeric<F>(std::marker::PhantomData<F>);
impl<F: Family> ColumnarRelation for Numeric<F> {
    const COLUMNS: &'static [&'static str] = &["id", "day", "price"];
    type Table = ScanTable<(ScanColumn<i64, F>, ScanColumn<Date, F>, ScanColumn<Decimal, F>)>;
    type Builders = (ScanColumnBuilder<i64, F>, ScanColumnBuilder<Date, F>, ScanColumnBuilder<Decimal, F>);
    fn builders(rows: BlockRows, summaries: bool) -> Self::Builders {
        (ScanColumnBuilder::new(rows, summaries), ScanColumnBuilder::new(rows, summaries), ScanColumnBuilder::new(rows, summaries))
    }
    fn append(batch: &Batch<'_>, builders: &mut Self::Builders) -> Result<()> {
        batch.append_column(0, &mut builders.0)?;
        batch.append_column(1, &mut builders.1)?;
        batch.append_column(2, &mut builders.2)
    }
    fn finish(builders: Self::Builders, _: &mut Dictionaries) -> Self::Table {
        ScanTable::new((builders.0.finish(), builders.1.finish(), builders.2.finish()))
    }
}

struct NumericRows;
impl Relation for NumericRows {
    type Row = (i64, Date, Decimal);
    const COLUMNS: &'static [&'static str] = Numeric::<Bytes>::COLUMNS;
    fn decode(batch: &Batch<'_>) -> Result<Vec<Self::Row>> {
        Ok(batch.column::<i64>(0)?.into_iter()
            .zip(batch.column::<Date>(1)?)
            .zip(batch.column::<Decimal>(2)?)
            .map(|((id, day), price)| (id, day, price)).collect())
    }
}

fn decoded<T: OrdinalDecode + std::fmt::Debug + PartialEq, F: Family>(column: &ScanColumn<T, F>) -> Vec<T> {
    let mut result = Vec::with_capacity(column.len());
    let mut values = [T::from_ordinal(0); VECTOR_ROWS];
    for (block, header) in column.headers().iter().enumerate() {
        for vector in 0..(header.rows as usize).div_ceil(VECTOR_ROWS) {
            let len = column.decode_vector(block, vector, &mut values);
            result.extend_from_slice(&values[..len]);
        }
    }
    assert_eq!(result.len(), column.len());
    if let Some(summaries) = column.summaries() {
        assert_eq!(summaries.len(), column.headers().len());
        for (summary, values) in summaries.iter().zip(result.chunks(column.block_rows().get())) {
            for value in values {
                assert!(summary.lo <= value.ordinal() && value.ordinal() <= summary.hi);
            }
            assert_eq!(summary.lo, values.iter().map(|value| value.ordinal()).min().unwrap());
            assert_eq!(summary.hi, values.iter().map(|value| value.ordinal()).max().unwrap());
        }
    }
    result
}

fn assert_numeric<F: Family>(table: &<Numeric<F> as ColumnarRelation>::Table, expected: &[(i64, Date, Decimal)], summaries: bool) {
    assert_eq!(table.len(), expected.len());
    assert_eq!(table.is_empty(), expected.is_empty());
    let columns = table.columns();
    assert_eq!(columns.0.summaries().is_some(), summaries);
    assert_eq!(columns.1.summaries().is_some(), summaries);
    assert_eq!(columns.2.summaries().is_some(), summaries);
    assert_eq!(decoded(&columns.0), expected.iter().map(|row| row.0).collect::<Vec<_>>());
    assert_eq!(decoded(&columns.1), expected.iter().map(|row| row.1).collect::<Vec<_>>());
    assert_eq!(decoded(&columns.2), expected.iter().map(|row| row.2).collect::<Vec<_>>());
}

fn numeric_rows(count: usize) -> Vec<(i64, Date, Decimal)> {
    (0..count).map(|i| {
        let id = match i % 1031 { 0 => i64::MIN, 1 => i64::MAX, _ => i as i64 - 1500 };
        (id, Date::from_days(i as i32 - 1500), Decimal::from_scaled(i as i64 * 13 - 9000).unwrap())
    }).collect()
}

#[test]
fn csv_roundtrip_matches_row_loader_across_partial_blocks_and_batches() {
    let rows = numeric_rows(3 * VECTOR_ROWS + 17);
    let mut text = String::from("price,id,day\n");
    for (id, day, price) in &rows {
        text.push_str(&format!("{price},{id},{day}\n"));
    }
    let file = Fixture::csv(&text);
    for batch_size in [1, 257, 1024, 2053, 8192] {
        let options = CsvOptions { headers: true, batch_size, ..CsvOptions::default() };
        let oracle = cq_load::read_csv::<NumericRows>(&file.0, options).unwrap();
        assert_eq!(oracle, rows);
        for summaries in [false, true] {
            for block_rows in [1024, 2048] {
                let block_rows = BlockRows::new(block_rows).unwrap();
                let bytes = ColumnarLoader::<Numeric<Bytes>>::csv(&file.0, options, block_rows, summaries).unwrap();
                let plain = ColumnarLoader::<Numeric<Plain>>::csv(&file.0, options, block_rows, summaries).unwrap();
                assert_eq!(bytes.block_rows(), block_rows);
                assert_numeric::<Bytes>(&bytes, &oracle, summaries);
                assert_numeric::<Plain>(&plain, &oracle, summaries);
            }
        }
    }
}

#[test]
fn empty_input_preserves_layout_and_summary_policy() {
    let file = Fixture::csv("id,day,price\n");
    let options = CsvOptions { headers: true, ..CsvOptions::default() };
    for summaries in [false, true] {
        let table = ColumnarLoader::<Numeric<Bytes>>::csv(&file.0, options, BlockRows::new(2048).unwrap(), summaries).unwrap();
        assert_numeric::<Bytes>(&table, &[], summaries);
        assert!(table.columns().0.headers().is_empty());
    }
}

type Codes<F> = <StringCode as ColumnarField<F>>::Builder;
struct Interned;
impl ColumnarRelation for Interned {
    const COLUMNS: &'static [&'static str] = &["left", "right", "id"];
    type Table = ScanTable<(ScanColumn<StringCode, Bytes>, ScanColumn<StringCode, Plain>, ScanColumn<i64, Bytes>)>;
    type Builders = (Codes<Bytes>, Codes<Plain>, ScanColumnBuilder<i64, Bytes>);
    fn builders(rows: BlockRows, summaries: bool) -> Self::Builders {
        (<StringCode as ColumnarField<Bytes>>::builder(rows, summaries),
         <StringCode as ColumnarField<Plain>>::builder(rows, summaries),
         ScanColumnBuilder::new(rows, summaries))
    }
    fn append(batch: &Batch<'_>, builders: &mut Self::Builders) -> Result<()> {
        <StringCode as ColumnarField<Bytes>>::append(batch, 0, &mut builders.0)?;
        <StringCode as ColumnarField<Plain>>::append(batch, 1, &mut builders.1)?;
        batch.append_column(2, &mut builders.2)
    }
    fn finish(builders: Self::Builders, dictionaries: &mut Dictionaries) -> Self::Table {
        ScanTable::new((
            <StringCode as ColumnarField<Bytes>>::finish("left", dictionaries, builders.0),
            <StringCode as ColumnarField<Plain>>::finish("right", dictionaries, builders.1),
            builders.2.finish(),
        ))
    }
}

fn assert_domains(table: &<Interned as ColumnarRelation>::Table, dictionaries: &Dictionaries, count: usize) {
    let left = decoded(&table.columns().0);
    let right = decoded(&table.columns().1);
    let ids = decoded(&table.columns().2);
    assert_eq!(ids, (0..count as i64).collect::<Vec<_>>());
    for i in 0..count {
        let text = ["z", "", "a", "z"][i % 4];
        assert_eq!(dictionaries.read("left", left[i]), text);
        assert_eq!(dictionaries.read("right", right[i]), if i % 2 == 0 { "a" } else { "z" });
    }
    assert_eq!(dictionaries.domain("left").lookup("a"), Some(1));
    assert_eq!(dictionaries.domain("right").lookup("a"), Some(0));
    assert_eq!(dictionaries.domain("left").lookup("absent"), None);
}

#[test]
fn interned_remap_encodes_final_domains_and_summaries() {
    let count = 2 * VECTOR_ROWS + 19;
    let mut text = String::new();
    for i in 0..count {
        text.push_str(&format!("{},{},{i}\n", ["z", "", "a", "z"][i % 4], if i % 2 == 0 { "a" } else { "z" }));
    }
    let file = Fixture::csv(&text);
    for batch_size in [1, 333, 4096] {
        let mut dictionaries = Dictionaries::default();
        let table = ColumnarLoader::<Interned>::csv_interned(
            &file.0, CsvOptions { batch_size, ..CsvOptions::default() },
            BlockRows::new(1024).unwrap(), true, &mut dictionaries,
        ).unwrap();
        assert_domains(&table, &dictionaries, count);
        // Reusing the same finalized domains must not change prior code meaning.
        let again = ColumnarLoader::<Interned>::csv_interned(
            &file.0, CsvOptions { batch_size: 107, ..CsvOptions::default() },
            BlockRows::new(2048).unwrap(), true, &mut dictionaries,
        ).unwrap();
        assert_domains(&again, &dictionaries, count);
    }
}

#[test]
fn csv_errors_and_dictionary_conflicts_are_transactional() {
    let file = Fixture::csv("z,a,1\na,z,2\nnew,new,broken\n");
    let mut dictionaries = Dictionaries::default();
    dictionaries.insert("retained", ["unchanged"]);
    let error = ColumnarLoader::<Interned>::csv_interned(
        &file.0, CsvOptions { batch_size: 1, ..CsvOptions::default() },
        BlockRows::new(1024).unwrap(), true, &mut dictionaries,
    ).err().unwrap();
    assert_eq!((error.line, error.column.as_str(), error.value.as_str()), (3, "id", "broken"));
    assert!(dictionaries.get("left").is_none());
    assert!(dictionaries.get("right").is_none());
    assert_eq!(dictionaries.read("retained", StringCode(0)), "unchanged");

    dictionaries.insert("right", ["existing"]);
    let file = Fixture::csv("new,conflicting,1\n");
    let error = ColumnarLoader::<Interned>::csv_interned(
        &file.0, CsvOptions::default(), BlockRows::new(1024).unwrap(), true, &mut dictionaries,
    ).err().unwrap();
    assert_eq!(error.column, "right");
    assert!(dictionaries.get("left").is_none());
    assert_eq!(dictionaries.read("right", StringCode(0)), "existing");
}

#[cfg(feature = "parquet")]
mod parquet_tests {
    use super::*;
    use arrow_array::{ArrayRef, Date32Array, Decimal128Array, Int64Array, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use cq_load::ParquetOptions;
    use parquet::arrow::ArrowWriter;
    use std::sync::Arc;

    fn parquet(fields: Vec<Field>, arrays: Vec<ArrayRef>) -> Fixture {
        let file = Fixture::path("parquet");
        let schema = Arc::new(Schema::new(fields));
        let batch = RecordBatch::try_new(schema.clone(), arrays).unwrap();
        let mut writer = ArrowWriter::try_new(std::fs::File::create(&file.0).unwrap(), schema, None).unwrap();
        for offset in (0..batch.num_rows()).step_by(733) {
            writer.write(&batch.slice(offset, 733.min(batch.num_rows() - offset))).unwrap();
            writer.flush().unwrap();
        }
        writer.close().unwrap();
        file
    }

    #[test]
    fn native_arrow_columns_match_row_loader_and_enclose_partial_blocks() {
        let rows = numeric_rows(3 * VECTOR_ROWS + 37);
        let file = parquet(
            vec![Field::new("price", DataType::Decimal128(18, 3), false), Field::new("day", DataType::Date32, false), Field::new("id", DataType::Int64, false)],
            vec![
                Arc::new(Decimal128Array::from(rows.iter().map(|row| i128::from(row.2.scaled()) * 10).collect::<Vec<_>>()).with_precision_and_scale(18, 3).unwrap()),
                Arc::new(Date32Array::from(rows.iter().map(|row| row.1.days_since_epoch()).collect::<Vec<_>>())),
                Arc::new(Int64Array::from(rows.iter().map(|row| row.0).collect::<Vec<_>>())),
            ],
        );
        for batch_size in [1, 257, 1024, 2053, 8192] {
            let options = ParquetOptions { batch_size };
            let oracle = cq_load::read_parquet::<NumericRows>(&file.0, options).unwrap();
            assert_eq!(oracle, rows);
            for summaries in [false, true] {
                let blocks = BlockRows::new(2048).unwrap();
                let bytes = ColumnarLoader::<Numeric<Bytes>>::parquet(&file.0, options, blocks, summaries).unwrap();
                let plain = ColumnarLoader::<Numeric<Plain>>::parquet(&file.0, options, blocks, summaries).unwrap();
                assert_numeric::<Bytes>(&bytes, &oracle, summaries);
                assert_numeric::<Plain>(&plain, &oracle, summaries);
            }
        }
    }

    #[test]
    fn arrow_interning_uses_final_codes_and_rolls_back_late_nulls() {
        let count = 2 * VECTOR_ROWS + 31;
        for null in [false, true] {
            let file = parquet(
                vec![Field::new("left", DataType::Utf8, true), Field::new("right", DataType::Utf8, false), Field::new("id", DataType::Int64, false)],
                vec![
                    Arc::new(StringArray::from((0..count).map(|i| if null && i == count - 1 { None } else { Some(["z", "", "a", "z"][i % 4]) }).collect::<Vec<_>>())),
                    Arc::new(StringArray::from((0..count).map(|i| if i % 2 == 0 { "a" } else { "z" }).collect::<Vec<_>>())),
                    Arc::new(Int64Array::from((0..count as i64).collect::<Vec<_>>())),
                ],
            );
            for batch_size in [333, 4096] {
                let mut dictionaries = Dictionaries::default();
                dictionaries.insert("retained", ["value"]);
                let result = ColumnarLoader::<Interned>::parquet_interned(
                    &file.0, ParquetOptions { batch_size }, BlockRows::new(1024).unwrap(), true, &mut dictionaries,
                );
                if null {
                    let error = result.err().unwrap();
                    assert_eq!(error.line, count as u64);
                    assert_eq!(error.column, "left");
                    assert_eq!(error.value, "<NULL>");
                    assert!(dictionaries.get("left").is_none());
                    assert!(dictionaries.get("right").is_none());
                } else {
                    assert_domains(&result.unwrap(), &dictionaries, count);
                }
                assert_eq!(dictionaries.read("retained", StringCode(0)), "value");
            }
        }
    }

    fn numeric_batch(rows: &[(i64, Date, Decimal)], reorder: bool) -> RecordBatch {
        let mut fields = vec![
            Field::new("id", DataType::Int64, false),
            Field::new("day", DataType::Date32, false),
            Field::new("price", DataType::Decimal128(18, 2), false),
        ];
        let mut arrays: Vec<ArrayRef> = vec![
            Arc::new(Int64Array::from(rows.iter().map(|row| row.0).collect::<Vec<_>>())),
            Arc::new(Date32Array::from(rows.iter().map(|row| row.1.days_since_epoch()).collect::<Vec<_>>())),
            Arc::new(Decimal128Array::from(rows.iter().map(|row| i128::from(row.2.scaled())).collect::<Vec<_>>()).with_precision_and_scale(18, 2).unwrap()),
        ];
        if reorder {
            fields.swap(0, 2);
            arrays.swap(0, 2);
        }
        RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays).unwrap()
    }

    #[test]
    fn arrow_batches_accept_empty_batches_and_column_reordering() {
        let rows = numeric_rows(2 * VECTOR_ROWS + 11);
        let batches = [
            Ok(numeric_batch(&[], false)),
            Ok(numeric_batch(&rows[..333], false)),
            Ok(numeric_batch(&[], true)),
            Ok(numeric_batch(&rows[333..], true)),
            Ok(numeric_batch(&[], false)),
        ];
        let table = ColumnarLoader::<Numeric<Bytes>>::arrow_batches(
            batches, BlockRows::new(1024).unwrap(), true,
        ).unwrap();
        assert_numeric::<Bytes>(&table, &rows, true);
        let empty = ColumnarLoader::<Numeric<Plain>>::arrow_batches(
            std::iter::empty(), BlockRows::new(2048).unwrap(), false,
        ).unwrap();
        assert_numeric::<Plain>(&empty, &[], false);
    }

    #[test]
    fn arrow_batches_stop_at_first_error_without_publishing_domains() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("left", DataType::Utf8, false),
            Field::new("right", DataType::Utf8, false),
            Field::new("id", DataType::Int64, true),
        ]));
        let batch = |id| RecordBatch::try_new(schema.clone(), vec![
            Arc::new(StringArray::from(vec!["new"])),
            Arc::new(StringArray::from(vec!["other"])),
            Arc::new(Int64Array::from(vec![id])),
        ]).unwrap();
        let calls = std::cell::Cell::new(0);
        let mut dictionaries = Dictionaries::default();
        dictionaries.insert("retained", ["unchanged"]);
        let batches = [Ok(batch(Some(1))), Ok(batch(None))].into_iter().chain(
            std::iter::once_with(|| panic!("read a batch after the first conversion error"))
        ).inspect(|_| calls.set(calls.get() + 1));
        let error = ColumnarLoader::<Interned>::arrow_batches_interned(
            batches, BlockRows::new(1024).unwrap(), true, &mut dictionaries,
        ).err().unwrap();
        assert_eq!(calls.get(), 2);
        assert_eq!(error.line, 2);
        assert_eq!(error.column, "id");
        assert_eq!(error.value, "<NULL>");
        assert!(dictionaries.get("left").is_none());
        assert!(dictionaries.get("right").is_none());
        assert_eq!(dictionaries.read("retained", StringCode(0)), "unchanged");

        let source_error = cq_load::Error {
            file: PathBuf::from("external.arrow"), line: 17,
            column: "<record>".into(), value: "<unreadable>".into(), message: "source failed".into(),
        };
        let batches = [Ok(batch(Some(1))), Err(source_error.clone())].into_iter().chain(
            std::iter::once_with(|| panic!("read a batch after the first source error"))
        );
        let error = ColumnarLoader::<Interned>::arrow_batches_interned(
            batches, BlockRows::new(1024).unwrap(), true, &mut dictionaries,
        ).err().unwrap();
        assert_eq!(error, source_error);
        assert!(dictionaries.get("left").is_none());
        assert!(dictionaries.get("right").is_none());
    }

    #[test]
    fn arrow_batches_report_missing_columns_before_encoding() {
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)])),
            vec![Arc::new(Int64Array::from(vec![1]))],
        ).unwrap();
        let error = ColumnarLoader::<Numeric<Bytes>>::arrow_batches(
            [Ok(batch)], BlockRows::new(1024).unwrap(), true,
        ).err().unwrap();
        assert_eq!(error.column, "day");
        assert_eq!(error.value, "<missing>");
    }
}
