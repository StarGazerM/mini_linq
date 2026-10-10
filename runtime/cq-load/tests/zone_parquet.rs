#![cfg(feature = "parquet")]
use arrow_array::{ArrayRef, Date32Array, Decimal128Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use cq_load::{Batch, Date, Decimal, Loader, MinMax, ParquetOptions, Relation, ZoneRows, column};
use cq_storage::rows::Rows;
use cq_storage::{BlockRead, Point, PositionalRows};
use parquet::arrow::ArrowWriter;
use parquet::file::properties::{EnabledStatistics, WriterProperties};
use std::sync::Arc;

struct Typed;
impl Relation for Typed {
    type Row = (i64, Date, Decimal, String);
    const COLUMNS: &'static [&'static str] = &["id", "day", "price", "text"];
    fn decode(batch: &Batch<'_>) -> cq_load::Result<Vec<Self::Row>> {
        Ok(batch
            .column::<i64>(0)?
            .into_iter()
            .zip(batch.column::<Date>(1)?)
            .zip(batch.column::<Decimal>(2)?)
            .zip(batch.column::<String>(3)?)
            .map(|(((id, date), price), text)| (id, date, price, text))
            .collect())
    }
}

#[test]
fn decoded_statistics_work_without_parquet_stats_and_across_row_groups() {
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
    type Summary = (MinMax<i64>, MinMax<Date>, MinMax<Decimal>, MinMax<String>);
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
            let batches =
                Loader::<Typed>::parquet_batches(&path, ParquetOptions { batch_size }).unwrap();
            let host =
                ZoneRows::from_batches::<Summary, _, _>(batches, 4.try_into().unwrap()).unwrap();
            let rows = Loader::<Typed>::parquet(&path, ParquetOptions::default()).unwrap();
            let expected = ZoneRows::from_rows::<Summary>(rows, 4.try_into().unwrap());
            assert_eq!(
                host.rows().collect::<Vec<_>>(),
                expected.rows().collect::<Vec<_>>()
            );
            assert_eq!(host.blocks().count(), 2);
            for block in host.blocks() {
                assert_eq!(
                    host.bounds(column::<0>(), block),
                    expected.bounds(column::<0>(), block)
                );
                assert_eq!(
                    host.bounds(column::<1>(), block),
                    expected.bounds(column::<1>(), block)
                );
                assert_eq!(
                    host.bounds(column::<2>(), block),
                    expected.bounds(column::<2>(), block)
                );
                assert_eq!(
                    host.bounds(column::<3>(), block),
                    expected.bounds(column::<3>(), block)
                );
            }
            assert_eq!(
                host.read(column::<0>(), &Point(0))
                    .map(|id| id.offset())
                    .collect::<Vec<_>>(),
                [3, 4]
            );
        }
        std::fs::remove_file(path).unwrap();
    }
}
