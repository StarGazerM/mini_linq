//! Caller-side, typed relation loading, independent of the query compiler.
//!
//! A generated [`Relation`] supplies column names and a tuple decoder from the
//! authored relation declaration. [`Loader`] collects tuples or groups them by a
//! caller-selected leading key prefix; [`Csv`] and, with the `parquet` feature,
//! `Parquet` expose bounded batches. The default batch size is 2048 rows. CSV
//! records are parsed once and released after each decoded batch. Parquet reads
//! Arrow record batches and decodes primitive columns in bulk. The underlying
//! Parquet reader additionally owns compressed page/row-group buffers; the row
//! batch limit does not bound an individual field or compressed page's bytes.
//!
//! [`CsvOptions`] selects a delimiter, header handling, and batch size. Headers
//! match declaration names and may reorder columns; missing, duplicate, and
//! unknown names are errors. Headerless files use declaration order. Set
//! `trailing_delimiter` for dbgen `.tbl` records ending in an extra separator.
//! Quoting, embedded separators, escaped quotes, CRLF and multiline fields use
//! standard CSV rules. Strings are preserved exactly; numbers are not trimmed.
//! Empty strings are values, not nulls. Parquet nulls are rejected because a
//! nonoptional relation field cannot represent them. Parquet columns match names
//! just as CSV headers do. Errors include the path, physical CSV record-start
//! line (logical one-based row for Parquet), declaration column name and value.
//!
//! Implement [`Load::load`] to support an owned user type. Its default Parquet
//! implementation parses a textual rendering of a primitive Arrow value; types
//! may override `Load::load_arrow` to decode a whole typed Arrow column. The
//! Arrow API and dependencies exist only with the `parquet` feature. That
//! feature enables the Arrow adapter and Snappy (DuckDB's default compression),
//! not the other compression codecs or network/async APIs.
//!
//! [`Decimal<SCALE>`] stores an exact signed `i64` scaled by `10^SCALE`; `SCALE`
//! defaults to 2 and must be at most 18. Extra trailing fractional zeroes are
//! accepted, while precision loss and overflow are errors. Display always writes
//! exactly `SCALE` fractional digits. [`Date`] stores Gregorian days since
//! 1970-01-01 and parses exactly `YYYY-MM-DD` (including year 0000).
//!
//! `f64` loading preserves Rust's IEEE-754 values, including NaN and infinities.
//! It is not totally ordered and does not implement `Eq`, `Ord`, or `Hash`:
//! declarations using it may load tuples, but cannot use that field directly as
//! a hash/ordered grouping key or in query plans requiring total ordering. No
//! implicit wrapper changes the query language's comparison semantics.
//!
//! [`ZoneRows`] stores immutable decoded tuples with per-column block bounds.
//! A tuple [`ZoneSchema`] selects [`MinMax`] for exact supported types and
//! [`NoBounds`] for other fields. [`column()`] creates declaration-order markers
//! for cq-storage's positional reads and selections. Summaries always come
//! from decoded rows; block skipping saves in-memory row tests, not Parquet I/O.
//!
//! [`StringDictionary`] lets caller-side preparation replace exact string values
//! with deterministic, sorted-distinct `u32` codes. Dictionaries are per chosen
//! column, not compiler rewrites or implicit changes to source loading.

mod csv_source;
mod intern;
mod loader;
mod scalar;
pub mod zone;
mod zone_docs;

#[cfg(feature = "parquet")]
mod parquet_source;

pub use csv_source::{Csv, CsvOptions, read_csv};
pub use intern::StringDictionary;
pub use loader::{Group, Loader};
pub use scalar::{Date, Decimal};
pub use zone::{ColumnAt, MinMax, NoBounds, ZoneField, ZoneMetadata, ZoneRows, ZoneSchema, column};

#[cfg(feature = "parquet")]
pub use parquet_source::{ColumnError, Parquet, ParquetOptions, read_parquet};

use std::fmt;
use std::path::{Path, PathBuf};

/// The default maximum decoded rows in a batch.
pub const DEFAULT_BATCH_SIZE: usize = 2048;

/// A contextual source error. A zero line denotes a file-open failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub file: PathBuf,
    pub line: u64,
    pub column: String,
    pub value: String,
    pub message: String,
}

impl Error {
    fn new(
        file: &Path,
        line: u64,
        column: impl Into<String>,
        value: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            file: file.to_path_buf(),
            line,
            column: column.into(),
            value: value.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}: column {:?}, value {:?}: {}",
            self.file.display(),
            self.line,
            self.column,
            self.value,
            self.message
        )
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Parse one owned column value; optional Arrow decoding works column at a time.
pub trait Load: Sized {
    fn load(field: &str) -> std::result::Result<Self, String>;

    #[cfg(feature = "parquet")]
    fn load_arrow(column: &dyn arrow_array::Array) -> std::result::Result<Vec<Self>, ColumnError> {
        parquet_source::parse_arrow(column, Self::load)
    }
}

/// Schema and typed batch decoder emitted from an authored relation declaration.
pub trait Relation {
    type Row;
    const COLUMNS: &'static [&'static str];
    fn decode(batch: &Batch<'_>) -> Result<Vec<Self::Row>>;
}

enum BatchData<'a> {
    Csv(&'a [csv::StringRecord]),
    #[cfg(feature = "parquet")]
    Parquet(&'a arrow_array::RecordBatch, u64),
}

/// Borrowed source batch, valid only while a generated decoder runs.
pub struct Batch<'a> {
    file: &'a Path,
    columns: &'static [&'static str],
    mapping: &'a [usize],
    data: BatchData<'a>,
}

impl Batch<'_> {
    pub fn len(&self) -> usize {
        match self.data {
            BatchData::Csv(records) => records.len(),
            #[cfg(feature = "parquet")]
            BatchData::Parquet(batch, _) => batch.num_rows(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Decode a declaration column, reporting the original field on failure.
    pub fn column<T: Load>(&self, index: usize) -> Result<Vec<T>> {
        let Some((&column, &source)) = self.columns.get(index).zip(self.mapping.get(index)) else {
            return Err(Error::new(
                self.file,
                0,
                "<schema>",
                index.to_string(),
                "column index outside schema",
            ));
        };
        match self.data {
            BatchData::Csv(records) => records
                .iter()
                .map(|record| {
                    let value = &record[source];
                    T::load(value).map_err(|message| {
                        Error::new(
                            self.file,
                            record.position().map_or(1, csv::Position::line),
                            column,
                            value,
                            message,
                        )
                    })
                })
                .collect(),
            #[cfg(feature = "parquet")]
            BatchData::Parquet(batch, first_row) => {
                let values = T::load_arrow(batch.column(source).as_ref()).map_err(|error| {
                    Error::new(
                        self.file,
                        first_row + error.row as u64,
                        column,
                        error.value,
                        error.message,
                    )
                })?;
                if values.len() != batch.num_rows() {
                    return Err(Error::new(
                        self.file,
                        first_row,
                        column,
                        "<batch>",
                        format!(
                            "Load::load_arrow returned {} values for {} source rows",
                            values.len(),
                            batch.num_rows()
                        ),
                    ));
                }
                Ok(values)
            }
        }
    }
}

fn column_mapping(file: &Path, names: &[&str], declared: &[&str]) -> Result<Vec<usize>> {
    for (index, name) in names.iter().enumerate() {
        if names[..index].contains(name) {
            return Err(Error::new(file, 1, *name, *name, "duplicate source column"));
        }
        if !declared.contains(name) {
            return Err(Error::new(file, 1, *name, *name, "unknown source column"));
        }
    }
    declared
        .iter()
        .map(|name| {
            names
                .iter()
                .position(|source| source == name)
                .ok_or_else(|| Error::new(file, 1, *name, "<missing>", "missing source column"))
        })
        .collect()
}
