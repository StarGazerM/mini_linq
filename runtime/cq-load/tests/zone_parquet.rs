#![cfg(feature = "parquet")]
use arrow_array::{ArrayRef, Date32Array, Decimal128Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use cq_load::{Batch, ColumnarField, ColumnarLoader, ColumnarRelation, Date, Decimal, Dictionaries, ParquetOptions, StringCode};
use cq_storage::OrderKey;
use cq_storage::scan::{BlockRows, Bytes, Family, OrdinalDecode, Predicate, ScanColumn, ScanColumnBuilder, ScanTable, VECTOR_ROWS};
use parquet::arrow::ArrowWriter;
use parquet::file::properties::{EnabledStatistics, WriterProperties};
use std::sync::Arc;

struct Typed;
impl ColumnarRelation for Typed {
    const COLUMNS: &'static [&'static str] = &["id", "day", "price", "text"];
    type Table = ScanTable<(ScanColumn<i64, Bytes>, ScanColumn<Date, Bytes>, ScanColumn<Decimal, Bytes>, ScanColumn<StringCode, Bytes>)>;
    type Builders = (ScanColumnBuilder<i64, Bytes>, ScanColumnBuilder<Date, Bytes>, ScanColumnBuilder<Decimal, Bytes>, <StringCode as ColumnarField<Bytes>>::Builder);
    fn builders(rows: BlockRows, summaries: bool) -> Self::Builders {
        (ScanColumnBuilder::new(rows, summaries), ScanColumnBuilder::new(rows, summaries),
            ScanColumnBuilder::new(rows, summaries), <StringCode as ColumnarField<Bytes>>::builder(rows, summaries))
    }
    fn append(batch: &Batch<'_>, builders: &mut Self::Builders) -> cq_load::Result<()> {
        batch.append_column(0, &mut builders.0)?;
        batch.append_column(1, &mut builders.1)?;
        batch.append_column(2, &mut builders.2)?;
        <StringCode as ColumnarField<Bytes>>::append(batch, 3, &mut builders.3)
    }
    fn finish(builders: Self::Builders, dictionaries: &mut Dictionaries) -> Self::Table {
        ScanTable::new((builders.0.finish(), builders.1.finish(), builders.2.finish(),
            <StringCode as ColumnarField<Bytes>>::finish("text", dictionaries, builders.3)))
    }
}

fn check_column<T: OrdinalDecode + std::fmt::Debug, F: Family>(
    column: &ScanColumn<T, F>, expected: &[T],
) {
    let mut actual = Vec::new();
    let mut buffer = [expected[0]; VECTOR_ROWS];
    for (block, header) in column.headers().iter().enumerate() {
        for vector in 0..(header.rows as usize).div_ceil(VECTOR_ROWS) {
            let count = column.decode_vector(block, vector, &mut buffer);
            actual.extend_from_slice(&buffer[..count]);
        }
        let summary = column.summaries().unwrap()[block];
        let start = block * column.block_rows().get();
        for value in &expected[start..start + header.rows as usize] {
            assert!(summary.lo <= value.ordinal() && value.ordinal() <= summary.hi);
        }
    }
    assert_eq!(actual, expected);
}

#[test]
fn encoded_statistics_work_without_parquet_stats_and_across_row_groups() {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, true),
        Field::new("day", DataType::Date32, false),
        Field::new("price", DataType::Decimal128(18, 3), false),
        Field::new("text", DataType::Utf8, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![9, -1, 7, 0, 0, 42, 5])) as ArrayRef,
            Arc::new(Date32Array::from(vec![0, -1, 11016, 0, 0, 99, 98])),
            Arc::new(
                Decimal128Array::from(vec![12300, -10, 1000, 0, 0, 20000, 99990])
                    .with_precision_and_scale(18, 3)
                    .unwrap(),
            ),
            Arc::new(StringArray::from(vec![
                "é",
                "a",
                "e\u{301}",
                "",
                "",
                "a long text",
                "中",
            ])),
        ],
    )
    .unwrap();
    for (index, stats) in [EnabledStatistics::None, EnabledStatistics::Page]
        .into_iter()
        .enumerate()
    {
        let path = std::env::temp_dir().join(format!(
            "cq-zone-parquet-{}-{index}.parquet",
            std::process::id()
        ));
        let props = WriterProperties::builder()
            .set_statistics_enabled(stats)
            .set_max_row_group_row_count(Some(3))
            .build();
        let mut writer = ArrowWriter::try_new(
            std::fs::File::create(&path).unwrap(),
            schema.clone(),
            Some(props),
        )
        .unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        // Nullable Arrow schema with no actual nulls is accepted. Price has a
        // different physical scale: bounds must use the decoded exact values.
        for batch_size in [1, 2, 5, 100] {
            let mut dictionaries = Dictionaries::default();
            let host = ColumnarLoader::<Typed>::parquet_interned(
                &path, ParquetOptions { batch_size }, BlockRows::new(1024).unwrap(),
                true, &mut dictionaries,
            ).unwrap();
            assert_eq!(host.len(), 7);
            let columns = host.columns();
            check_column(&columns.0, &[9, -1, 7, 0, 0, 42, 5]);
            let dates: Vec<Date> = ["1970-01-01", "1969-12-31", "2000-02-29", "1970-01-01", "1970-01-01", "1970-04-10", "1970-04-09"]
                .into_iter().map(|s| s.parse().unwrap()).collect();
            check_column(&columns.1, &dates);
            let prices: Vec<Decimal> = ["12.30", "-0.01", "1.00", "0", "0", "20.00", "99.99"]
                .into_iter().map(|s| s.parse().unwrap()).collect();
            check_column(&columns.2, &prices);
            let text: Vec<StringCode> = ["é", "a", "e\u{301}", "", "", "a long text", "中"]
                .into_iter().map(|s| StringCode(dictionaries.get("text").unwrap().lookup(s).unwrap())).collect();
            check_column(&columns.3, &text);
            let mut selected = Vec::new();
            columns.0.filter(&Predicate::Point(0_i64.ordinal()), |row| selected.push(row));
            assert_eq!(selected, [3, 4]);
        }
        std::fs::remove_file(path).unwrap();
    }
}
