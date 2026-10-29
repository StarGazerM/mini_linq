use cq_load::{Batch, Csv, CsvOptions, Date, Decimal, Load, Relation, Result, read_csv};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct FileFixture(PathBuf);

impl FileFixture {
    fn csv(text: &str) -> Self {
        let fixture = Self::path("csv");
        std::fs::write(&fixture.0, text).unwrap();
        fixture
    }

    fn path(extension: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        Self(std::env::temp_dir().join(format!(
            "cq-load-{}-{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            extension
        )))
    }
}

impl Drop for FileFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct Typed;
impl Relation for Typed {
    type Row = (i64, String, Date, Decimal, bool);
    const COLUMNS: &'static [&'static str] = &["id", "name", "day", "price", "active"];
    // DECODE P: Typed reconstructs rows solely for CSV/Parquet loading diagnostics and answer assertions.
    fn decode(batch: &Batch<'_>) -> Result<Vec<Self::Row>> {
        // DECODE P: the Typed diagnostic exposes decoded ids to its expected-row oracle.
        let ids = batch.column::<i64>(0)?;
        // DECODE P: the Typed diagnostic exposes decoded names to its expected-row oracle.
        let names = batch.column::<String>(1)?;
        // DECODE P: the Typed diagnostic exposes decoded dates to its expected-row oracle.
        let days = batch.column::<Date>(2)?;
        // DECODE P: the Typed diagnostic exposes decoded prices to its expected-row oracle.
        let prices = batch.column::<Decimal>(3)?;
        // DECODE P: the Typed diagnostic exposes decoded booleans to its expected-row oracle.
        let active = batch.column::<bool>(4)?;
        Ok(ids
            .into_iter()
            .zip(names)
            .zip(days)
            .zip(prices)
            .zip(active)
            .map(|((((id, name), day), price), active)| (id, name, day, price, active))
            .collect())
    }
}

struct Ids;
impl Relation for Ids {
    type Row = (i64,);
    const COLUMNS: &'static [&'static str] = &["id"];
    // DECODE P: Ids reconstructs singleton rows for batch-size and loader-error diagnostics only.
    fn decode(batch: &Batch<'_>) -> Result<Vec<Self::Row>> {
        Ok(batch
            // DECODE P: the Ids fixture exposes each decoded integer to its diagnostic row assertions.
            .column::<i64>(0)?
            .into_iter()
            .map(|value| (value,))
            .collect())
    }
}

#[test]
fn primitive_ranges_and_owned_extension() {
    macro_rules! integer_range {
        ($($ty:ty),* $(,)?) => {$ (
            // DECODE P: the integer-range diagnostic publishes the reconstructed minimum to its assertion.
            assert_eq!(<$ty>::load(&<$ty>::MIN.to_string()).unwrap(), <$ty>::MIN);
            // DECODE P: the integer-range diagnostic publishes the reconstructed maximum to its assertion.
            assert_eq!(<$ty>::load(&<$ty>::MAX.to_string()).unwrap(), <$ty>::MAX);
            // DECODE P: the integer-overflow diagnostic exposes only the failed reconstruction result.
            assert!(<$ty>::load("184467440737095516160").is_err());
            // DECODE P: the integer-whitespace diagnostic exposes only the parsing rejection.
            assert!(<$ty>::load(" 1").is_err());
        )* };
    }
    integer_range!(i8, i16, i32, i64, u8, u16, u32, u64);
    // DECODE P: the true-token diagnostic publishes the reconstructed boolean to assert!.
    assert!(bool::load("true").unwrap());
    // DECODE P: the false-token diagnostic publishes the reconstructed boolean to assert!.
    assert!(!bool::load("false").unwrap());
    // DECODE P: the uppercase-boolean diagnostic exposes the rejected reconstruction result.
    assert!(bool::load("TRUE").is_err());
    // DECODE P: the NaN diagnostic publishes the reconstructed float to its classification assertion.
    assert!(f64::load("NaN").unwrap().is_nan());
    // DECODE P: the negative-infinity diagnostic publishes the reconstructed float to assert_eq!.
    assert_eq!(f64::load("-inf").unwrap(), f64::NEG_INFINITY);
    // DECODE P: the fractional-float diagnostic publishes the reconstructed value to assert_eq!.
    assert_eq!(f64::load("1.25").unwrap(), 1.25);
    // DECODE P: the empty-string diagnostic publishes the reconstructed String to assert_eq!.
    assert_eq!(String::load("").unwrap(), "");
    // DECODE P: the boxed-string diagnostic exposes reconstructed text to assert_eq!.
    assert_eq!(&*Box::<str>::load("hello").unwrap(), "hello");
    // DECODE P: the Arc-string diagnostic exposes reconstructed text to assert_eq!.
    assert_eq!(&*std::sync::Arc::<str>::load("hello").unwrap(), "hello");
    // DECODE P: the Rc-string diagnostic exposes reconstructed text to assert_eq!.
    assert_eq!(&*std::rc::Rc::<str>::load("hello").unwrap(), "hello");
    #[derive(Debug, PartialEq)]
    struct Id(u64);
    impl Load for Id {
        // DECODE P: this test-only Id loader reconstructs values for extension diagnostics.
        fn load(field: &str) -> std::result::Result<Self, String> {
            field
                .strip_prefix("ID-")
                .ok_or_else(|| "missing ID prefix".into())
                // DECODE P: the Id diagnostic reconstructs its stripped numeric payload for the expected-value oracle.
                .and_then(u64::load)
                .map(Self)
        }
    }
    // DECODE P: the prefixed-Id diagnostic publishes the reconstructed extension value.
    assert_eq!(Id::load("ID-42").unwrap(), Id(42));
    // DECODE P: the missing-prefix diagnostic exposes the rejected Id reconstruction.
    assert!(Id::load("42").is_err());
}

#[test]
fn decimals_are_exact_and_checked_at_i64_boundaries() {
    // DECODE P: the cents diagnostic exposes the decoded decimal coefficient for an exactness assertion.
    assert_eq!(Decimal::<2>::load("12.30").unwrap().scaled(), 1230);
    assert_eq!(
        // DECODE P: the trailing-zero diagnostic exposes a reconstructed decimal through display comparison.
        Decimal::<2>::load("+12.30000").unwrap().to_string(),
        "12.30"
    );
    // DECODE P: the negative-cent diagnostic exposes the reconstructed decimal coefficient.
    assert_eq!(Decimal::<2>::load("-0.01").unwrap().scaled(), -1);
    // DECODE P: the scale-zero diagnostic exposes the reconstructed decimal's display text.
    assert_eq!(Decimal::<0>::load("12.000").unwrap().to_string(), "12");
    assert_eq!(
        // DECODE P: the scale-eighteen diagnostic exposes its reconstructed least-significant coefficient.
        Decimal::<18>::load("0.000000000000000001")
            .unwrap()
            .scaled(),
        1
    );
    for (source, expected) in [
        ("92233720368547758.07", i64::MAX),
        ("-92233720368547758.08", i64::MIN),
    ] {
        // DECODE P: decimal-boundary diagnostics reconstruct extrema only for exact coefficient and display assertions.
        let value = Decimal::<2>::load(source).unwrap();
        assert_eq!(value.scaled(), expected);
        assert_eq!(value.to_string(), source);
    }
    for source in [
        "92233720368547758.08",
        "-92233720368547758.09",
        "1.001",
        ".2",
        "--1",
        "",
        "1e2",
        "1.2.3",
        " 1",
        "é",
    ] {
        // DECODE P: malformed-decimal diagnostics expose reconstruction failures without relational processing.
        assert!(Decimal::<2>::load(source).is_err(), "{source}");
    }
    // DECODE P: the unsupported-text-scale diagnostic exposes the rejected decimal parse.
    assert!(Decimal::<19>::load("0").is_err());
    // DECODE P: the unsupported-coefficient-scale diagnostic exposes the rejected Decimal construction.
    assert!(Decimal::<19>::from_scaled(0).is_err());
}

#[test]
fn dates_validate_gregorian_leap_days_and_roundtrip_epoch() {
    // DECODE P: the epoch-date diagnostic publishes the reconstructed date's zero day count.
    assert_eq!(Date::load("1970-01-01").unwrap().days_since_epoch(), 0);
    // DECODE P: the pre-epoch diagnostic publishes the reconstructed negative day count.
    assert_eq!(Date::load("1969-12-31").unwrap().days_since_epoch(), -1);
    // DECODE P: the leap-day diagnostic publishes the reconstructed epoch offset.
    assert_eq!(Date::load("2000-02-29").unwrap().days_since_epoch(), 11016);
    for date in [
        "0000-01-01",
        "0000-02-29",
        "0001-01-01",
        "1900-03-01",
        "2000-02-29",
        "2024-12-31",
        "9999-12-31",
    ] {
        // DECODE P: Gregorian-date diagnostics expose reconstructed dates only through round-trip display assertions.
        assert_eq!(Date::load(date).unwrap().to_string(), date);
    }
    for date in [
        "1900-02-29",
        "2023-02-29",
        "2000-02-30",
        "2024-04-31",
        "2024-00-01",
        "2024-13-01",
        "2024-01-00",
        "2024-1-01",
        "2024-01-01x",
        "ééééé",
    ] {
        // DECODE P: invalid-calendar diagnostics expose rejected date reconstruction results.
        assert!(Date::load(date).is_err(), "{date}");
    }
    for day in -100000..100000 {
        assert_eq!(
            // DECODE P: the epoch sweep constructs Date from days solely for its diagnostic text round trip.
            // DECODE P: the epoch sweep parses the rendered date solely for its diagnostic day-count assertion.
            Date::load(&Date::from_days(day).to_string())
                .unwrap()
                .days_since_epoch(),
            day
        );
    }
}

#[test]
fn csv_headers_reorder_columns_quoting_and_dbgen_trailing_separator() {
    let input = FileFixture::csv(
        "name|price|active|day|id|\r\n\"a|b\"|12.30|true|2000-02-29|9223372036854775807|\r\n\"a\"\"b\"|-0.01|false|1969-12-31|-42|\r\n",
    );
    // DECODE P: the reordered-CSV diagnostic publishes reconstructed rows solely to its expected-answer assertions.
    let rows = read_csv::<Typed>(
        &input.0,
        CsvOptions {
            headers: true,
            delimiter: b'|',
            trailing_delimiter: true,
            batch_size: 1,
        },
    )
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0],
        (
            i64::MAX,
            "a|b".into(),
            // DECODE P: the CSV answer oracle reconstructs its expected date literal for comparison.
            Date::load("2000-02-29").unwrap(),
            // DECODE P: the CSV answer oracle reconstructs its expected Decimal literal for comparison.
            Decimal::load("12.30").unwrap(),
            true
        )
    );
    assert_eq!(rows[1].1, "a\"b");
    assert_eq!(rows[1].3.scaled(), -1);
}

#[test]
fn csv_batches_are_bounded_default_and_custom() {
    let input = FileFixture::csv(&(0..4099).map(|id| format!("{id}\n")).collect::<String>());
    let batches = Csv::<Ids>::open(&input.0, CsvOptions::default())
        .unwrap()
        // DECODE P: consuming the CSV iterator reconstructs rows solely for batch-boundary and sequence diagnostics.
        .collect::<Result<Vec<_>>>()
        .unwrap();
    assert_eq!(
        batches.iter().map(Vec::len).collect::<Vec<_>>(),
        [2048, 2048, 3]
    );
    assert_eq!(
        batches
            .into_iter()
            .flatten()
            .map(|row| row.0)
            .collect::<Vec<_>>(),
        (0..4099).collect::<Vec<_>>()
    );
    assert!(
        Csv::<Ids>::open(
            &input.0,
            CsvOptions {
                batch_size: 0,
                ..Default::default()
            }
        )
        .is_err()
    );
    let empty = FileFixture::csv("");
    assert!(
        // DECODE P: the empty-source diagnostic invokes reconstruction only to expose an empty answer.
        read_csv::<Ids>(&empty.0, CsvOptions::default())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn csv_errors_identify_original_value_column_and_physical_line_and_fuse() {
    let input = FileFixture::csv(
        "id,name,day,price,active\n1,\"a\nb\",2024-01-01,1.00,true\n2,x,2024-02-30,2.00,false\n",
    );
    let mut batches = Csv::<Typed>::open(
        &input.0,
        CsvOptions {
            headers: true,
            batch_size: 1,
            ..Default::default()
        },
    )
    .unwrap();
    // DECODE P: the quoted-newline diagnostic publishes the first reconstructed batch's string to assert_eq!.
    assert_eq!(batches.next().unwrap().unwrap()[0].1, "a\nb");
    // DECODE P: the invalid-date batch diagnostic exposes the second reconstruction's contextual error.
    let error = batches.next().unwrap().unwrap_err();
    assert_eq!(error.file, input.0);
    assert_eq!(error.line, 4);
    assert_eq!(error.column, "day");
    assert_eq!(error.value, "2024-02-30");
    assert!(
        error
            .to_string()
            .contains("column \"day\", value \"2024-02-30\"")
    );
    assert!(batches.next().is_none());
    assert!(batches.next().is_none());
}

#[test]
fn csv_rejects_schema_mismatch_and_extra_nonempty_field() {
    for text in ["other\n1\n", "id,id\n1,2\n", "id,other\n1,2\n"] {
        let input = FileFixture::csv(text);
        assert!(
            Csv::<Ids>::open(
                &input.0,
                CsvOptions {
                    headers: true,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    for text in ["1,2\n", "1,,\n"] {
        let input = FileFixture::csv(text);
        assert!(
            // DECODE P: the row-width diagnostic invokes CSV loading only to expose its rejection.
            read_csv::<Ids>(
                &input.0,
                CsvOptions {
                    trailing_delimiter: true,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
    let missing = FileFixture::csv("1,x,2024-01-01\n");
    // DECODE P: the missing-field diagnostic invokes CSV loading only to expose its source-column error.
    let error = read_csv::<Typed>(&missing.0, CsvOptions::default()).unwrap_err();
    assert_eq!(error.column, "price");
    assert_eq!(error.value, "<missing>");
    let header_missing = FileFixture::csv("name,day,price,active\n");
    let error = match Csv::<Typed>::open(
        &header_missing.0,
        CsvOptions {
            headers: true,
            ..Default::default()
        },
    ) {
        Err(error) => error,
        Ok(_) => panic!("missing id header was accepted"),
    };
    assert_eq!(error.column, "id");
    let reordered_missing = FileFixture::csv("name,day,price,active,id\nx,2024-01-01,1.00,true\n");
    // DECODE P: the reordered-missing-field diagnostic exposes only the failed row-loading result.
    let error = read_csv::<Typed>(
        &reordered_missing.0,
        CsvOptions {
            headers: true,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.column, "id");
    for delimiter in [b'"', b'\n', b'\r', 0, 255] {
        assert!(
            Csv::<Ids>::open(
                &missing.0,
                CsvOptions {
                    delimiter,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}

#[cfg(feature = "parquet")]
mod parquet_tests {
    use super::*;
    use arrow_array::{
        ArrayRef, BooleanArray, Date32Array, Decimal128Array, Int64Array, RecordBatch, StringArray,
    };
    use arrow_schema::{DataType, Field, Schema};
    use cq_load::{
        Dictionaries, InternedField, InternedRelation, Loader, Parquet, ParquetOptions, StringCode,
        read_parquet,
    };
    use parquet::arrow::ArrowWriter;
    use parquet::basic::Compression;
    use parquet::file::properties::WriterProperties;
    use std::sync::Arc;

    fn write(fields: Vec<Field>, arrays: Vec<ArrayRef>) -> FileFixture {
        let file = FileFixture::path("parquet");
        let schema = Arc::new(Schema::new(fields));
        let batch = RecordBatch::try_new(schema.clone(), arrays).unwrap();
        let properties = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();
        let mut writer = ArrowWriter::try_new(
            std::fs::File::create(&file.0).unwrap(),
            schema,
            Some(properties),
        )
        .unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        file
    }

    struct InternedTyped;

    impl InternedRelation for InternedTyped {
        type Row = (i64, StringCode, Date, Decimal, bool);
        type Source = Typed;

        fn intern(
            rows: Vec<<Typed as Relation>::Row>,
            dictionaries: &mut Dictionaries,
        ) -> Vec<Self::Row> {
            StringCode::prepare("name", rows.iter().map(|row| &row.1), dictionaries);
            let dictionary = dictionaries.get("name");
            rows.into_iter()
                .map(|(id, name, day, price, active)| {
                    (id, StringCode::intern(name, dictionary), day, price, active)
                })
                .collect()
        }
    }

    #[test]
    fn parquet_interning_sorts_across_batches_and_preserves_source_rows() {
        let file = write(
            vec![
                Field::new("name", DataType::Utf8, false),
                Field::new("day", DataType::Date32, false),
                Field::new("price", DataType::Decimal128(18, 2), false),
                Field::new("id", DataType::Int64, false),
                Field::new("active", DataType::Boolean, false),
            ],
            vec![
                Arc::new(StringArray::from(vec!["z", "", "a", "z"])),
                Arc::new(Date32Array::from(vec![0, -1, 1, 0])),
                Arc::new(
                    Decimal128Array::from(vec![123, -10, 99, 123])
                        .with_precision_and_scale(18, 2)
                        .unwrap(),
                ),
                Arc::new(Int64Array::from(vec![2, 1, 3, 2])),
                Arc::new(BooleanArray::from(vec![true, false, true, true])),
            ],
        );
        let mut dictionaries = Dictionaries::default();
        let rows = Loader::<InternedTyped>::parquet_interned(
            &file.0,
            ParquetOptions { batch_size: 2 },
            &mut dictionaries,
        )
        .unwrap();
        assert_eq!(
            rows.iter().map(|row| (row.0, row.1)).collect::<Vec<_>>(),
            [
                (2, StringCode(2)),
                (1, StringCode(0)),
                (3, StringCode(1)),
                (2, StringCode(2))
            ]
        );
        assert_eq!(rows[0], rows[3]);
        assert_eq!(dictionaries.literal("name", "missing"), StringCode::MAX);
        assert_eq!(
            dictionaries.select("name", |value| value.is_empty()),
            [(StringCode(0),)]
        );
        // DECODE P: the interned loader diagnostic publishes dictionary text for comparison with its source-row oracle.
        let published: Vec<_> = rows
            .into_iter()
            .map(|(id, name, day, price, active)| {
                (id, dictionaries.publish("name", name), day, price, active)
            })
            .collect();
        // DECODE P: the plain loader supplies the expected logical rows for the interning equivalence assertion.
        let expected = read_parquet::<Typed>(&file.0, ParquetOptions::default()).unwrap();
        assert_eq!(published, expected);
        let one_row_batches = Loader::<InternedTyped>::parquet_interned(
            &file.0,
            ParquetOptions { batch_size: 1 },
            &mut dictionaries,
        )
        .unwrap();
        assert_eq!(one_row_batches[0].1, StringCode(2));
        assert_eq!(one_row_batches[2].1, StringCode(1));
    }

    #[test]
    fn parquet_source_errors_leave_existing_dictionaries_unchanged() {
        let file = write(
            vec![Field::new("other", DataType::Int64, false)],
            vec![Arc::new(Int64Array::from(vec![1]))],
        );
        let mut dictionaries = Dictionaries::default();
        dictionaries.insert("retained", ["value"]);
        let error = Loader::<InternedTyped>::parquet_interned(
            &file.0,
            ParquetOptions::default(),
            &mut dictionaries,
        )
        .unwrap_err();
        assert_eq!(error.column, "other");
        assert!(dictionaries.get("name").is_none());
        assert_eq!(dictionaries.literal("retained", "value"), StringCode(0));
    }

    #[test]
    fn parquet_native_columns_match_csv_and_reorder() {
        let file = write(
            vec![
                Field::new("name", DataType::Utf8, false),
                Field::new("day", DataType::Date32, false),
                Field::new("price", DataType::Decimal128(18, 3), false),
                Field::new("id", DataType::Int64, false),
                Field::new("active", DataType::Boolean, false),
            ],
            vec![
                Arc::new(StringArray::from(vec!["quoted, text", "", "é"])),
                Arc::new(Date32Array::from(vec![0, -1, 11016])),
                Arc::new(
                    Decimal128Array::from(vec![12300, -10, 990])
                        .with_precision_and_scale(18, 3)
                        .unwrap(),
                ),
                Arc::new(Int64Array::from(vec![i64::MAX, -1, 42])),
                Arc::new(BooleanArray::from(vec![true, false, true])),
            ],
        );
        let text = FileFixture::csv(
            "9223372036854775807,\"quoted, text\",1970-01-01,12.30,true\n-1,,1969-12-31,-0.01,false\n42,é,2000-02-29,0.99,true\n",
        );
        let batches = Parquet::<Typed>::open(&file.0, ParquetOptions { batch_size: 2 })
            .unwrap()
            // DECODE P: consuming native Parquet batches exposes reconstructed rows solely to the CSV-equivalence diagnostic.
            .collect::<Result<Vec<_>>>()
            .unwrap();
        assert_eq!(batches.iter().map(Vec::len).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(
            batches.into_iter().flatten().collect::<Vec<_>>(),
            // DECODE P: the CSV side of the Parquet-equivalence diagnostic publishes expected logical rows.
            read_csv::<Typed>(&text.0, CsvOptions::default()).unwrap()
        );
        assert!(Parquet::<Typed>::open(&file.0, ParquetOptions { batch_size: 0 }).is_err());
    }

    #[test]
    fn parquet_null_errors_preserve_file_row_and_column_across_batches() {
        let file = write(
            vec![Field::new("id", DataType::Int64, true)],
            vec![Arc::new(Int64Array::from(vec![Some(1), Some(2), None]))],
        );
        let mut batches = Parquet::<Ids>::open(&file.0, ParquetOptions { batch_size: 2 }).unwrap();
        // DECODE P: the nullable-Parquet diagnostic publishes the valid first batch to its row assertion.
        assert_eq!(batches.next().unwrap().unwrap(), [(1,), (2,)]);
        // DECODE P: the nullable-Parquet diagnostic exposes the failing second batch's null error.
        let error = batches.next().unwrap().unwrap_err();
        assert_eq!(error.file, file.0);
        assert_eq!(error.line, 3);
        assert_eq!(error.column, "id");
        assert_eq!(error.value, "<NULL>");
        assert!(batches.next().is_none());
    }

    #[test]
    fn parquet_decimal_precision_and_overflow_are_contextual() {
        struct Prices;
        impl Relation for Prices {
            type Row = (Decimal,);
            const COLUMNS: &'static [&'static str] = &["price"];
            // DECODE P: Prices reconstructs diagnostic rows solely to test contextual decimal errors.
            fn decode(batch: &Batch<'_>) -> Result<Vec<Self::Row>> {
                Ok(batch
                    // DECODE P: the Prices diagnostic exposes Decimal reconstruction failures to its error oracle.
                    .column::<Decimal>(0)?
                    .into_iter()
                    .map(|value| (value,))
                    .collect())
            }
        }
        for (scaled, rendered) in [
            (1001, "1.001"),
            (92233720368547758080, "92233720368547758.080"),
        ] {
            let file = write(
                vec![Field::new("price", DataType::Decimal128(22, 3), false)],
                vec![Arc::new(
                    Decimal128Array::from(vec![scaled])
                        .with_precision_and_scale(22, 3)
                        .unwrap(),
                )],
            );
            // DECODE P: the precision/overflow diagnostic invokes Parquet reconstruction only to publish contextual errors.
            let error = read_parquet::<Prices>(&file.0, ParquetOptions::default()).unwrap_err();
            assert_eq!(error.line, 1);
            assert_eq!(error.column, "price");
            assert_eq!(error.value, rendered);
        }
    }

    #[test]
    fn parquet_user_types_default_to_primitive_text_and_can_override_batches() {
        #[derive(Debug, PartialEq)]
        struct Label(String);
        impl Load for Label {
            // DECODE P: the test-only Label loader constructs host values for extension-adapter diagnostics.
            fn load(field: &str) -> std::result::Result<Self, String> {
                Ok(Self(format!("ID-{field}")))
            }
        }
        assert_eq!(
            // DECODE P: the integer-to-Label diagnostic exposes reconstructed labels to its expected answer array.
            Label::load_arrow(&Int64Array::from(vec![1, 2])).unwrap(),
            [Label("ID-1".into()), Label("ID-2".into())]
        );
        assert_eq!(
            // DECODE P: the string-to-Label diagnostic exposes reconstructed labels to its expected answer array.
            Label::load_arrow(&StringArray::from(vec!["3"])).unwrap(),
            [Label("ID-3".into())]
        );
        // DECODE P: the narrow-integer Arrow diagnostic exposes the failed reconstruction's source value.
        let error = i8::load_arrow(&Int64Array::from(vec![128])).unwrap_err();
        assert_eq!(error.value, "128");
        let wrong = write(
            vec![Field::new("other", DataType::Int64, false)],
            vec![Arc::new(Int64Array::from(vec![1]))],
        );
        assert!(Parquet::<Ids>::open(&wrong.0, ParquetOptions::default()).is_err());
    }

    #[test]
    fn parquet_rejects_user_decoder_that_changes_column_length() {
        #[derive(Debug)]
        struct Invalid;
        impl Load for Invalid {
            // DECODE P: this deliberately invalid test loader constructs a value only for loader-contract diagnostics.
            fn load(_: &str) -> std::result::Result<Self, String> {
                Ok(Self)
            }
            // DECODE P: this deliberately empty test decoder exposes a length-contract violation, not query data.
            fn load_arrow(
                _: &dyn arrow_array::Array,
            ) -> std::result::Result<Vec<Self>, cq_load::ColumnError> {
                Ok(Vec::new())
            }
        }
        struct InvalidSchema;
        impl Relation for InvalidSchema {
            type Row = (Invalid,);
            const COLUMNS: &'static [&'static str] = &["id"];
            // DECODE P: InvalidSchema invokes reconstruction only to expose a diagnostic column-length error.
            fn decode(batch: &Batch<'_>) -> Result<Vec<Self::Row>> {
                Ok(batch
                    // DECODE P: the InvalidSchema column conversion exposes the malformed decoder result to its error oracle.
                    .column::<Invalid>(0)?
                    .into_iter()
                    .map(|value| (value,))
                    .collect())
            }
        }
        let file = write(
            vec![Field::new("id", DataType::Int64, false)],
            vec![Arc::new(Int64Array::from(vec![1, 2]))],
        );
        // DECODE P: the invalid-length diagnostic invokes Parquet reconstruction solely to expose its contract error.
        let error = read_parquet::<InvalidSchema>(&file.0, ParquetOptions::default()).unwrap_err();
        assert_eq!(error.file, file.0);
        assert_eq!(error.line, 1);
        assert_eq!(error.column, "id");
        assert!(
            error
                .message
                .contains("returned 0 values for 2 source rows")
        );
    }
}
