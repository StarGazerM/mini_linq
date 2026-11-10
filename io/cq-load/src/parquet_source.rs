use crate::{Batch, BatchData, DEFAULT_BATCH_SIZE, Date, Error, Relation, Result};
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

/// Arrow-backed source supporting row batches or direct column ingestion.
pub struct Parquet<S> {
    reader: ParquetReader<'static>,
    schema: PhantomData<S>,
}

pub(crate) struct ParquetReader<'a> {
    reader: ParquetRecordBatchReader,
    file: PathBuf,
    mapping: Vec<usize>,
    columns: &'a [&'a str],
    next_row: u64,
    finished: bool,
    certified_set: bool,
}

impl<S: Relation> Parquet<S> {
    pub fn open(path: impl AsRef<Path>, options: ParquetOptions) -> Result<Self> {
        Self::open_columns(path, options, S::COLUMNS)
    }
}

impl<S> Parquet<S> {
    pub(crate) fn open_columns(
        path: impl AsRef<Path>,
        options: ParquetOptions,
        columns: &'static [&'static str],
    ) -> Result<Self> {
        Ok(Self {
            reader: ParquetReader::open(path.as_ref(), options, columns)?,
            schema: PhantomData,
        })
    }
    pub(crate) fn open_certified(
        path: &Path,
        options: ParquetOptions,
        columns: &'static [&'static str],
        manifest: &Path,
        key_columns: &[(&str, Option<crate::set_manifest::KeyEncoding>)],
    ) -> Result<Self> {
        Ok(Self {
            reader: ParquetReader::open_with_manifest(path, options, columns, Some(manifest), key_columns)?,
            schema: PhantomData,
        })
    }

    pub(crate) fn certified_set(&self) -> bool {
        self.reader.certified_set
    }

    pub(crate) fn next_with<T>(
        &mut self,
        decode: impl FnOnce(&Batch<'_>) -> Result<T>,
    ) -> Option<Result<T>> {
        self.reader.next_with(decode)
    }
}

impl<'a> ParquetReader<'a> {
    pub(crate) fn open(
        file: &Path,
        options: ParquetOptions,
        columns: &'a [&'a str],
    ) -> Result<Self> {
        Self::open_with_manifest(file, options, columns, None, &[])
    }

    fn open_with_manifest(
        file: &Path,
        options: ParquetOptions,
        columns: &'a [&'a str],
        manifest: Option<&Path>,
        key_columns: &[(&str, Option<crate::set_manifest::KeyEncoding>)],
    ) -> Result<Self> {
        if options.batch_size == 0 {
            return Err(Error::new(
                file,
                0,
                "<options>",
                "0",
                "batch size must be positive",
            ));
        }
        let mut input = File::open(file)
            .map_err(|error| Error::new(file, 0, "<file>", "", error.to_string()))?;
        let reader_input = input.try_clone()
            .map_err(|error| Error::new(file, 0, "<file>", "", error.to_string()))?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(reader_input)
            .map_err(|error| Error::new(file, 0, "<schema>", "<unreadable>", error.to_string()))?;
        let names = builder
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect::<Vec<_>>();
        for (index, name) in names.iter().enumerate() {
            if names[..index].contains(name) {
                return Err(Error::new(file, 1, *name, *name, "duplicate source column"));
            }
        }
        let mapping = columns.iter().map(|column| {
            names.iter().position(|name| name == column)
                .ok_or_else(|| Error::new(file, 1, *column, "<missing>", "missing source column"))
        }).collect::<Result<Vec<_>>>()?;
        // Hash the same opened file used by the reader, never a second pathname.
        let certified_set = manifest.is_some_and(|manifest| {
            crate::set_manifest::certified(manifest, file, &mut input, builder.schema(), key_columns)
        });
        let reader = builder
            .with_batch_size(options.batch_size)
            .build()
            .map_err(|error| Error::new(file, 0, "<file>", "<unreadable>", error.to_string()))?;
        Ok(Self {
            reader,
            file: file.to_path_buf(),
            mapping,
            columns,
            next_row: 1,
            finished: false,
            certified_set,
        })
    }
}

impl ParquetReader<'_> {
    pub(crate) fn next_with<T>(
        &mut self,
        decode: impl FnOnce(&Batch<'_>) -> Result<T>,
    ) -> Option<Result<T>> {
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
            columns: self.columns,
            mapping: &self.mapping,
            data: BatchData::Parquet(&batch, self.next_row),
        };
        // The callback consumes borrowed arrays directly for columnar loading.
        let decoded = decode(&source);
        self.next_row += batch.num_rows() as u64;
        if decoded.is_err() {
            self.finished = true;
        }
        Some(decoded)
    }
}

impl<S: Relation> Iterator for Parquet<S> {
    type Item = Result<Vec<S::Row>>;

    fn next(&mut self) -> Option<Self::Item> {
        self.next_with(S::decode)
    }
}

impl<S: Relation> std::iter::FusedIterator for Parquet<S> {}

// DECODE ?: read_parquet collects logical rows for arbitrary hosts without a caller publication or opaque-read fact.
pub fn read_parquet<S: Relation>(
    path: impl AsRef<Path>,
    options: ParquetOptions,
) -> Result<Vec<S::Row>>
where
    S::Row: Ord,
{
    let path = path.as_ref();
    read_parquet_with_manifest::<S>(path, options, &crate::set_manifest::manifest_path(path))
}

/// Load with an explicit certificate path instead of adjacent/environment lookup.
/// Missing, malformed and mismatching certificates retain canonicalization.
pub fn read_parquet_with_manifest<S: Relation>(
    path: impl AsRef<Path>,
    options: ParquetOptions,
    manifest: &Path,
) -> Result<Vec<S::Row>>
where
    S::Row: Ord,
{
    let mut source = Parquet::<S>::open_certified(path.as_ref(), options, S::COLUMNS, manifest, S::KEY_COLUMNS)?;
    let certified_set = source.certified_set();
    // DECODE ?: this fold reconstructs rows for the caller's tuple-vector host.
    let mut rows = source.try_fold(Vec::new(), |mut rows, batch| {
        rows.extend(batch?);
        Ok::<_, Error>(rows)
    })?;
    crate::loader::canonicalize_unless_certified(&mut rows, certified_set);
    Ok(rows)
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
    parse: impl FnMut(&str) -> std::result::Result<T, String>,
) -> std::result::Result<Vec<T>, ColumnError> {
    let mut values = Vec::with_capacity(column.len());
    parse_arrow_into(column, parse, |value| values.push(value))?;
    Ok(values)
}

pub(crate) fn parse_arrow_into<T>(
    column: &dyn Array,
    mut parse: impl FnMut(&str) -> std::result::Result<T, String>,
    mut emit: impl FnMut(T),
) -> std::result::Result<(), ColumnError> {
    macro_rules! string_column {
        ($array:ty) => {
            if let Some(array) = column.as_any().downcast_ref::<$array>() {
                return array
                    .iter()
                    .enumerate()
                    .try_for_each(|(row, value)| {
                        let value = value.ok_or_else(|| ColumnError::null(row))?;
                        // DECODE ?: this Arrow string entry is converted by an arbitrary parser; publication or Rust-read evidence is absent.
                        parse(value).map_err(|message| ColumnError {
                            row,
                            value: value.into(),
                            message,
                        }).map(&mut emit)
                    });
            }
        };
    }
    string_column!(StringArray);
    string_column!(LargeStringArray);
    string_column!(StringViewArray);
    // DECODE ?: nonstring Arrow fallback delegates reconstruction with the same unknown consumer as string_values.
    parse_arrow_nonstring(column, parse, emit)
}

// DECODE ?: parse_arrow is a generic logical reconstruction adapter, not an established publication boundary.
pub(crate) fn parse_arrow<T>(
    column: &dyn Array,
    parse: impl FnMut(&str) -> std::result::Result<T, String>,
) -> std::result::Result<Vec<T>, ColumnError> {
    // DECODE ?: parse_arrow forwards to string_values without adding caller-use evidence.
    string_values(column, parse)
}

// DECODE ?: parse_arrow_nonstring reconstructs logical values for arbitrary Load implementations.
fn parse_arrow_nonstring<T>(
    column: &dyn Array,
    mut parse: impl FnMut(&str) -> std::result::Result<T, String>,
    mut emit: impl FnMut(T),
) -> std::result::Result<(), ColumnError> {
    macro_rules! primitive {
        ($($array:ident),* $(,)?) => {$ (
            if let Some(array) = column.as_any().downcast_ref::<arrow_array::$array>() {
                return array.iter().enumerate().try_for_each(|(row, value)| {
                    let value = value.ok_or_else(|| ColumnError::null(row))?.to_string();
                    // DECODE ?: primitive Arrow text is parsed into caller T before its runtime use is known.
                    parse(&value).map_err(|message| ColumnError { row, value, message }).map(&mut emit)
                });
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
        return array.iter().enumerate().try_for_each(|(row, value)| {
            // DECODE ?: Arrow epoch days become Date for fallback rendering, with no opaque-read or answer fact.
            let value = Date::from_days(value.ok_or_else(|| ColumnError::null(row))?).to_string();
            // DECODE ?: the rendered Date is parsed into arbitrary T whose subsequent consumer is unrecorded.
            parse(&value)
                .map_err(|message| ColumnError {
                    row,
                    value,
                    message,
                })
                .map(&mut emit)
        });
    }
    if let (Some(array), arrow_schema::DataType::Decimal128(_, scale)) = (
        column
            .as_any()
            .downcast_ref::<arrow_array::Decimal128Array>(),
        column.data_type(),
    ) {
        return array.iter().enumerate().try_for_each(|(row, value)| {
            // DECODE ?: Decimal128 coefficient and scale become logical decimal text for generic fallback loading.
            let value = decimal_text(value.ok_or_else(|| ColumnError::null(row))?, *scale);
            // DECODE ?: decimal text is reconstructed as caller T with no recorded final consumer.
            parse(&value)
                .map_err(|message| ColumnError {
                    row,
                    value,
                    message,
                })
                .map(&mut emit)
        });
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
