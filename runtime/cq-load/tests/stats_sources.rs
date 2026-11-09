use cq_load::stats_source::{SourceFormat, SourceOptions, visit};
use cq_load::{Batch, CsvOptions, Relation, Result};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture(PathBuf);
impl Fixture {
    fn path(extension: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self(std::env::temp_dir().join(format!(
            "cq-stats-source-{}-{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            extension,
        )))
    }

    fn text(text: &str) -> Self {
        let file = Self::path("txt");
        std::fs::write(&file.0, text).unwrap();
        file
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct Pair;
impl Relation for Pair {
    type Row = (i64, String);
    const COLUMNS: &'static [&'static str] = &["id", "text"];

    fn decode(batch: &Batch<'_>) -> Result<Vec<Self::Row>> {
        Ok(batch.column::<i64>(0)?.into_iter().zip(batch.column::<String>(1)?).collect())
    }
}

fn pairs(file: &Fixture, options: &SourceOptions) -> Result<Vec<(i64, String)>> {
    let mut rows = Vec::new();
    visit(&file.0, Pair::COLUMNS, options, |batch| {
        rows.extend(Pair::decode(batch)?);
        Ok(())
    })?;
    Ok(rows)
}

#[test]
fn csv_matches_existing_parser_with_reordering_quotes_and_multiline_fields() {
    for (text, headers, trailing_delimiter) in [
        ("1|\"a|b\"\r\n-2|\"line\nnext\"\r\n3|\"a\"\"b\"\n4|\n", false, false),
        ("text|id\r\n\"a|b\"|1\r\n\"line\nnext\"|-2\r\n\"a\"\"b\"|3\n|4\n", true, false),
        ("text|id|\r\n\"a|b\"|1|\r\n\"line\nnext\"|-2|\r\n\"a\"\"b\"|3|\n|4|\n", true, true),
    ] {
        let file = Fixture::text(text);
        for batch_size in [1, 2, 19] {
            let options = SourceOptions {
                delimiter: b'|', headers, trailing_delimiter, batch_size,
                ..SourceOptions::default()
            };
            let oracle: Vec<_> = cq_load::Csv::<Pair>::open(&file.0, CsvOptions {
                delimiter: options.delimiter,
                headers,
                trailing_delimiter,
                batch_size,
            }).unwrap().collect::<Result<Vec<_>>>().unwrap().into_iter().flatten().collect();
            assert_eq!(pairs(&file, &options).unwrap(), oracle);
            assert_eq!(oracle, vec![
                (1, "a|b".into()), (-2, "line\nnext".into()),
                (3, "a\"b".into()), (4, "".into()),
            ]);
        }
    }
}

#[test]
fn csv_preserves_existing_error_locations_and_width_checks() {
    for (text, headers, trailing_delimiter) in [
        ("1,ok\n2\n", false, false),
        ("1,ok\n2,bad,extra\n", false, false),
        ("text,id\n\"line\nnext\",not-an-int\n", true, false),
        ("text,id,\nhello,1,not-empty\n", true, true),
        ("id,id\n", true, false),
        ("id,unknown\n", true, false),
        ("id\n", true, false),
        ("", true, false),
    ] {
        let file = Fixture::text(text);
        let options = SourceOptions {
            headers, trailing_delimiter, batch_size: 1,
            ..SourceOptions::default()
        };
        let oracle = cq_load::read_csv::<Pair>(&file.0, CsvOptions {
            delimiter: options.delimiter, headers, trailing_delimiter, batch_size: 1,
        }).unwrap_err();
        assert_eq!(pairs(&file, &options).unwrap_err(), oracle);
    }
}

#[test]
fn dynamic_schema_exceeds_tuple_arity_without_static_names() {
    let names = (0..17).map(|i| format!("column_{i}")).collect::<Vec<_>>();
    let columns = names.iter().map(String::as_str).collect::<Vec<_>>();
    let header = names.iter().rev().map(String::as_str).collect::<Vec<_>>().join(",");
    let row = (0..17).rev().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let file = Fixture::text(&format!("{header}\n{row}\n{row}\n"));
    let mut visited = 0;
    visit(&file.0, &columns, &SourceOptions {
        headers: true, batch_size: 1, ..SourceOptions::default()
    }, |batch| {
        assert_eq!(batch.len(), 1);
        for index in 0..columns.len() {
            assert_eq!(batch.column::<i64>(index)?, vec![index as i64]);
        }
        visited += batch.len();
        Ok(())
    }).unwrap();
    assert_eq!(visited, 2);
}

#[test]
fn batches_visit_source_rows_once_including_duplicates() {
    let file = Fixture::text("4\n1\n1\n-3\n8\n");
    let mut lengths = Vec::new();
    let mut values = Vec::new();
    visit(&file.0, &["id"], &SourceOptions {
        batch_size: 2, ..SourceOptions::default()
    }, |batch| {
        lengths.push(batch.len());
        values.extend(batch.column::<i64>(0)?);
        Ok(())
    }).unwrap();
    assert_eq!(lengths, [2, 2, 1]);
    assert_eq!(values, [4, 1, 1, -3, 8]);
}

#[test]
fn callback_and_source_errors_stop_traversal() {
    let file = Fixture::text("bad\n2,extra\n3\n");
    let mut calls = 0;
    let error = visit(&file.0, &["id"], &SourceOptions {
        batch_size: 1, ..SourceOptions::default()
    }, |batch| {
        calls += 1;
        batch.column::<i64>(0)?;
        Ok(())
    }).unwrap_err();
    assert_eq!(calls, 1);
    assert_eq!(error.line, 1);
    assert_eq!(error.value, "bad");

    let file = Fixture::text("1\n2,extra\n3\n");
    let mut calls = 0;
    let error = visit(&file.0, &["id"], &SourceOptions {
        batch_size: 1, ..SourceOptions::default()
    }, |_| { calls += 1; Ok(()) }).unwrap_err();
    assert_eq!(calls, 1);
    assert_eq!(error.line, 2);
    assert_eq!(error.column, "<extra>");
}

fn literal_options() -> SourceOptions {
    SourceOptions { format: SourceFormat::LiteralTabs, delimiter: b'\t', ..SourceOptions::default() }
}

#[test]
fn literal_tabs_match_prepared_signed_rows_and_skip_only_empty_lines() {
    let file = Fixture::text("\n\r\n-7\t+11\r\n\n-7\t+11\n0\t-2147483648");
    let mut rows = Vec::new();
    visit(&file.0, &["left", "right"], &SourceOptions {
        batch_size: 1, ..literal_options()
    }, |batch| {
        rows.extend(batch.column::<i32>(0)?.into_iter().zip(batch.column::<i32>(1)?));
        Ok(())
    }).unwrap();
    assert_eq!(rows, [(-7, 11), (-7, 11), (0, i32::MIN)]);

    let file = Fixture::text("\"a\"\t\"b\"\r\nleft\rright\tx\n\t\n");
    let mut rows = Vec::new();
    visit(&file.0, &["left", "right"], &literal_options(), |batch| {
        rows.extend(batch.column::<String>(0)?.into_iter().zip(batch.column::<String>(1)?));
        Ok(())
    }).unwrap();
    assert_eq!(rows, [("\"a\"".into(), "\"b\"".into()), ("left\rright".into(), "x".into()), ("".into(), "".into())]);
}

#[test]
fn literal_tabs_reject_extra_missing_and_quoted_integer_fields() {
    for (text, line, column, value) in [
        ("\n\r\n1\t2\t\n", 3, "<extra>", ""),
        ("\n1\n", 2, "right", "<missing>"),
        ("\n\"1\"\t2\n", 2, "left", "\"1\""),
        (" \t2\n", 1, "left", " "),
        ("1\t2\r", 1, "right", "2\r"),
    ] {
        let file = Fixture::text(text);
        let error = visit(&file.0, &["left", "right"], &literal_options(), |batch| {
            batch.column::<i32>(0)?;
            batch.column::<i32>(1)?;
            Ok(())
        }).unwrap_err();
        assert_eq!(error.file, file.0);
        assert_eq!((error.line, error.column.as_str(), error.value.as_str()), (line, column, value));
    }
    let file = Fixture::text("\"a\tb\"\tx\n");
    assert!(visit(&file.0, &["left", "right"], &literal_options(), |_| Ok(())).is_err());
}

#[test]
fn empty_input_still_validates_dynamic_schema_and_options() {
    let file = Fixture::text("");
    for columns in [&[][..], &["id", "id"][..], &[""][..]] {
        assert!(visit(&file.0, columns, &SourceOptions::default(), |_| panic!("empty input")).is_err());
    }
    for options in [
        SourceOptions { batch_size: 0, ..SourceOptions::default() },
        SourceOptions { delimiter: b'\n', ..SourceOptions::default() },
        SourceOptions { delimiter: b'\r', ..SourceOptions::default() },
        SourceOptions { delimiter: b'"', ..SourceOptions::default() },
        SourceOptions { delimiter: 0, ..SourceOptions::default() },
        SourceOptions { delimiter: 255, ..SourceOptions::default() },
        SourceOptions { delimiter: b',', ..literal_options() },
        SourceOptions { headers: true, ..literal_options() },
        SourceOptions { trailing_delimiter: true, ..literal_options() },
        SourceOptions { format: SourceFormat::Parquet, headers: true, ..SourceOptions::default() },
        SourceOptions { format: SourceFormat::Parquet, delimiter: b'\t', ..SourceOptions::default() },
        SourceOptions { format: SourceFormat::Parquet, trailing_delimiter: true, ..SourceOptions::default() },
    ] {
        let error = visit(&file.0, &["id"], &options, |_| panic!("empty input")).unwrap_err();
        assert_eq!(error.column, "<options>");
    }
    for options in [SourceOptions::default(), literal_options()] {
        visit(&file.0, &["id"], &options, |_| panic!("empty input")).unwrap();
    }
    assert!(serde_json::from_str::<SourceOptions>(r#"{"format":"csv","unknown":1}"#).is_err());
    assert!(serde_json::from_str::<SourceOptions>(r#"{"format":"unknown"}"#).is_err());
    let options = literal_options();
    assert_eq!(serde_json::from_str::<SourceOptions>(&serde_json::to_string(&options).unwrap()).unwrap(), options);
}

#[cfg(not(feature = "parquet"))]
#[test]
fn parquet_without_feature_is_explicitly_unsupported() {
    let file = Fixture::text("");
    let error = visit(&file.0, &["id"], &SourceOptions {
        format: SourceFormat::Parquet, ..SourceOptions::default()
    }, |_| panic!("unsupported source")).unwrap_err();
    assert!(error.message.contains("require the parquet feature"));
}

#[cfg(feature = "parquet")]
mod parquet_tests {
    use super::*;
    use arrow_array::{ArrayRef, Int64Array, RecordBatch, StringArray};
    use arrow_schema::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;
    use std::sync::Arc;

    fn parquet(ids: Vec<Option<i64>>) -> Fixture {
        let file = Fixture::path("parquet");
        let schema = Arc::new(Schema::new(vec![
            Field::new("text", DataType::Utf8, false),
            Field::new("id", DataType::Int64, true),
        ]));
        let arrays: Vec<ArrayRef> = vec![
            Arc::new(StringArray::from(vec!["value"; ids.len()])),
            Arc::new(Int64Array::from(ids)),
        ];
        let batch = RecordBatch::try_new(schema.clone(), arrays).unwrap();
        let mut writer = ArrowWriter::try_new(std::fs::File::create(&file.0).unwrap(), schema, None).unwrap();
        for offset in (0..batch.num_rows()).step_by(3) {
            writer.write(&batch.slice(offset, 3.min(batch.num_rows() - offset))).unwrap();
            writer.flush().unwrap();
        }
        writer.close().unwrap();
        file
    }

    fn options(batch_size: usize) -> SourceOptions {
        SourceOptions { format: SourceFormat::Parquet, batch_size, ..SourceOptions::default() }
    }

    #[test]
    fn parquet_visits_each_row_once_with_name_mapping_and_partial_batches() {
        let values = vec![Some(-7), Some(2), Some(2), Some(4), Some(0), Some(9), Some(10)];
        let file = parquet(values);
        for batch_size in [1, 2, 9] {
            let oracle: Vec<_> = cq_load::Parquet::<Pair>::open(
                &file.0, cq_load::ParquetOptions { batch_size },
            ).unwrap().collect::<Result<Vec<_>>>().unwrap().into_iter().flatten().collect();
            assert_eq!(pairs(&file, &options(batch_size)).unwrap(), oracle);
            assert_eq!(oracle.iter().map(|row| row.0).collect::<Vec<_>>(), [-7, 2, 2, 4, 0, 9, 10]);
            let mut visited = 0;
            visit(&file.0, Pair::COLUMNS, &options(batch_size), |batch| {
                assert!(batch.len() <= batch_size);
                visited += batch.len();
                Ok(())
            }).unwrap();
            assert_eq!(visited, 7);
        }
    }

    #[test]
    fn parquet_null_decoder_stops_with_original_row_and_column() {
        let file = parquet(vec![Some(1), None, Some(3)]);
        let mut calls = 0;
        let error = visit(&file.0, Pair::COLUMNS, &options(1), |batch| {
            calls += 1;
            Pair::decode(batch)?;
            Ok(())
        }).unwrap_err();
        assert_eq!(calls, 2);
        assert_eq!((error.line, error.column.as_str(), error.value.as_str()), (2, "id", "<NULL>"));
        assert_eq!(error, cq_load::read_parquet::<Pair>(&file.0, cq_load::ParquetOptions { batch_size: 1 }).unwrap_err());
    }

    #[test]
    fn empty_parquet_validates_schema_without_visiting_rows() {
        let file = parquet(vec![]);
        visit(&file.0, Pair::COLUMNS, &options(2), |_| panic!("empty parquet")).unwrap();
        for columns in [&["id"][..], &["id", "other"][..]] {
            assert!(visit(&file.0, columns, &options(2), |_| panic!("invalid schema")).is_err());
        }
        let invalid = Fixture::text("");
        assert!(visit(&invalid.0, Pair::COLUMNS, &options(2), |_| panic!("invalid parquet")).is_err());
    }
}
