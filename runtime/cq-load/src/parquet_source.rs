use crate::{Batch, BatchData, DEFAULT_BATCH_SIZE, Date, Error, Relation, Result, column_mapping};
use arrow_array::{Array, LargeStringArray, StringArray, StringViewArray};
use parquet::arrow::arrow_reader::{ParquetRecordBatchReader, ParquetRecordBatchReaderBuilder};
use std::fs::File;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

/// Zero-based row offset and offending value from a column decoder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnError {
    pub row: usize,
    pub value: String,
    pub message: String,
}

impl ColumnError {
    pub fn null(row: usize) -> Self {
        Self {
            row,
            value: "<NULL>".into(),
            message: "null cannot inhabit a nonoptional field".into(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ParquetOptions {
    pub batch_size: usize,
}

impl Default for ParquetOptions {
    fn default() -> Self {
        Self {
            batch_size: DEFAULT_BATCH_SIZE,
        }
    }
}

/// Arrow-backed source yielding bounded decoded batches in file row order.
pub struct Parquet<S> {
    reader: ParquetRecordBatchReader,
    file: PathBuf,
    mapping: Vec<usize>,
    next_row: u64,
    finished: bool,
    schema: PhantomData<S>,
}

impl<S: Relation> Parquet<S> {
    pub fn open(path: impl AsRef<Path>, options: ParquetOptions) -> Result<Self> {
        let file = path.as_ref();
        if options.batch_size == 0 {
            return Err(Error::new(
                file,
                0,
                "<options>",
                "0",
                "batch size must be positive",
            ));
        }
        let input = File::open(file)
            .map_err(|error| Error::new(file, 0, "<file>", "", error.to_string()))?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(input)
            .map_err(|error| Error::new(file, 0, "<schema>", "<unreadable>", error.to_string()))?;
        let names = builder
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>();
        let mapping = column_mapping(file, &names, S::COLUMNS)?;
        let reader = builder
            .with_batch_size(options.batch_size)
            .build()
            .map_err(|error| Error::new(file, 0, "<file>", "<unreadable>", error.to_string()))?;
        Ok(Self {
            reader,
            file: file.to_path_buf(),
            mapping,
            next_row: 1,
            finished: false,
            schema: PhantomData,
        })
    }
}

impl<S: Relation> Iterator for Parquet<S> {
    type Item = Result<Vec<S::Row>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let batch = match self.reader.next() {
            None => {
                self.finished = true;
                return None;
            }
            Some(Err(error)) => {
                self.finished = true;
                return Some(Err(Error::new(
                    &self.file,
                    self.next_row,
                    "<record>",
                    "<unreadable>",
                    error.to_string(),
                )));
            }
            Some(Ok(batch)) => batch,
        };
        let source = Batch {
            file: &self.file,
            columns: S::COLUMNS,
            mapping: &self.mapping,
            data: BatchData::Parquet(&batch, self.next_row),
        };
        // DECODE V: zone_tpch feeds these owned decoded tuples to membership scans; P5 must retain columnar payloads and materialize late.
        let decoded = S::decode(&source);
        self.next_row += batch.num_rows() as u64;
        if decoded.is_err() {
            self.finished = true;
        }
        Some(decoded)
    }
}

impl<S: Relation> std::iter::FusedIterator for Parquet<S> {}

// DECODE ?: read_parquet collects logical rows for arbitrary hosts without a caller publication or opaque-read fact.
pub fn read_parquet<S: Relation>(
    path: impl AsRef<Path>,
    options: ParquetOptions,
) -> Result<Vec<S::Row>> {
    // DECODE ?: this fold drives Parquet::next reconstruction before the generic caller's downstream use is known.
    Parquet::<S>::open(path, options)?.try_fold(Vec::new(), |mut rows, batch| {
        rows.extend(batch?);
        Ok(rows)
    })
}

pub(crate) fn native_values<T>(
    values: impl Iterator<Item = Option<T>>,
) -> std::result::Result<Vec<T>, ColumnError> {
    values
        .enumerate()
        .map(|(row, value)| value.ok_or_else(|| ColumnError::null(row)))
        .collect()
}

// DECODE ?: string_values invokes caller parsing over Arrow strings without a recorded logical consumer.
pub(crate) fn string_values<T>(
    column: &dyn Array,
    parse: impl Fn(&str) -> std::result::Result<T, String>,
) -> std::result::Result<Vec<T>, ColumnError> {
    macro_rules! string_column {
        ($array:ty) => {
            if let Some(array) = column.as_any().downcast_ref::<$array>() {
                return array
                    .iter()
                    .enumerate()
                    .map(|(row, value)| {
                        let value = value.ok_or_else(|| ColumnError::null(row))?;
                        // DECODE ?: this Arrow string entry is converted by an arbitrary parser; publication or Rust-read evidence is absent.
                        parse(value).map_err(|message| ColumnError {
                            row,
                            value: value.into(),
                            message,
                        })
                    })
                    .collect();
            }
        };
    }
    string_column!(StringArray);
    string_column!(LargeStringArray);
    string_column!(StringViewArray);
    // DECODE ?: nonstring Arrow fallback delegates reconstruction with the same unknown consumer as string_values.
    parse_arrow_nonstring(column, parse)
}

// DECODE ?: parse_arrow is a generic logical reconstruction adapter, not an established publication boundary.
pub(crate) fn parse_arrow<T>(
    column: &dyn Array,
    parse: impl Fn(&str) -> std::result::Result<T, String>,
) -> std::result::Result<Vec<T>, ColumnError> {
    // DECODE ?: parse_arrow forwards to string_values without adding caller-use evidence.
    string_values(column, parse)
}

// DECODE ?: parse_arrow_nonstring reconstructs logical values for arbitrary Load implementations.
fn parse_arrow_nonstring<T>(
    column: &dyn Array,
    parse: impl Fn(&str) -> std::result::Result<T, String>,
) -> std::result::Result<Vec<T>, ColumnError> {
    macro_rules! primitive {
        ($($array:ident),* $(,)?) => {$ (
            if let Some(array) = column.as_any().downcast_ref::<arrow_array::$array>() {
                return array.iter().enumerate().map(|(row, value)| {
                    let value = value.ok_or_else(|| ColumnError::null(row))?.to_string();
                    // DECODE ?: primitive Arrow text is parsed into caller T before its runtime use is known.
                    parse(&value).map_err(|message| ColumnError { row, value, message })
                }).collect();
            }
        )* };
    }
    primitive!(
        BooleanArray,
        Int8Array,
        Int16Array,
        Int32Array,
        Int64Array,
        UInt8Array,
        UInt16Array,
        UInt32Array,
        UInt64Array,
        Float32Array,
        Float64Array
    );
    if let Some(array) = column.as_any().downcast_ref::<arrow_array::Date32Array>() {
        return array
            .iter()
            .enumerate()
            .map(|(row, value)| {
                // DECODE ?: Arrow epoch days become Date for fallback rendering, with no opaque-read or answer fact.
                let value =
                    Date::from_days(value.ok_or_else(|| ColumnError::null(row))?).to_string();
                // DECODE ?: the rendered Date is parsed into arbitrary T whose subsequent consumer is unrecorded.
                parse(&value).map_err(|message| ColumnError {
                    row,
                    value,
                    message,
                })
            })
            .collect();
    }
    if let (Some(array), arrow_schema::DataType::Decimal128(_, scale)) = (
        column
            .as_any()
            .downcast_ref::<arrow_array::Decimal128Array>(),
        column.data_type(),
    ) {
        return array
            .iter()
            .enumerate()
            .map(|(row, value)| {
                // DECODE ?: Decimal128 coefficient and scale become logical decimal text for generic fallback loading.
                let value = decimal_text(value.ok_or_else(|| ColumnError::null(row))?, *scale);
                // DECODE ?: decimal text is reconstructed as caller T with no recorded final consumer.
                parse(&value).map_err(|message| ColumnError {
                    row,
                    value,
                    message,
                })
            })
            .collect();
    }
    Err(ColumnError {
        row: 0,
        value: format!("<{:?}>", column.data_type()),
        message: "unsupported Arrow source type; implement Load::load_arrow for this user type"
            .into(),
    })
}

// DECODE ?: decimal_text reconstructs scaled decimal text for both fallback loading and error diagnostics.
pub(crate) fn decimal_text(value: i128, scale: i8) -> String {
    let sign = if value < 0 { "-" } else { "" };
    let digits = value.unsigned_abs().to_string();
    if scale <= 0 {
        return format!("{sign}{digits}{}", "0".repeat((-i16::from(scale)) as usize));
    }
    let scale = scale as usize;
    if digits.len() <= scale {
        format!("{sign}0.{}{digits}", "0".repeat(scale - digits.len()))
    } else {
        let split = digits.len() - scale;
        format!("{sign}{}.{}", &digits[..split], &digits[split..])
    }
}
