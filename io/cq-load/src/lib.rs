//! Caller-side, typed relation loading, independent of the query compiler.
//!
//! A generated [`Relation`] supplies column names and a tuple decoder from the
//! authored relation declaration. [`Loader`] canonicalizes complete rows or
//! groups them by a caller-selected leading key prefix; [`Csv`] and, with the `parquet` feature,
//! `Parquet` expose bounded batches. The default batch size is 2048 rows. CSV
//! records are parsed once and released after each decoded batch. Parquet reads
//! Arrow record batches and decodes primitive columns in bulk. The underlying
//! Parquet reader additionally owns compressed page/row-group buffers; the row
//! batch limit does not bound an individual field or compressed page's bytes.
//!
//! [`relation_loaders!`] accepts one or more IO relation schemas with ordered
//! column names and Rust types, independently of any query program:
//!
//! ```
//! cq_load::relation_loaders! {
//!     relation Reading(id: i64, label: String);
//! }
//! assert_eq!(
//!     <__MiniLinqLoadReading<i64, String> as cq_load::Relation>::COLUMNS,
//!     &["id", "label"],
//! );
//! ```
//!
//! Full CQ programs and compiler annotations are not accepted. Use
//! [`columnar_relation_loaders!`] with an explicit `@ bytes` or `@ plain` family
//! on every field to generate scan-table loaders.
//!
//! [`CsvOptions`] selects a delimiter, header handling, and batch size. Headers
//! match declaration names and may reorder columns; missing, duplicate, and
//! unknown names are errors. Headerless files use declaration order. Set
//! `trailing_delimiter` for dbgen `.tbl` records ending in an extra separator.
//! Quoting, embedded separators, escaped quotes, CRLF and multiline fields use
//! standard CSV rules. Strings are preserved exactly; numbers are not trimmed.
//! Empty strings are values, not nulls. Parquet nulls are rejected because a
//! nonoptional relation field cannot represent them. Parquet columns match
//! declaration names and may project a wider source heading. Missing or
//! duplicate names are errors. Errors include the path, physical CSV record-start
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
//! declarations using it may decode batches, but cannot use collected relation
//! loaders, hash/ordered grouping keys or query plans requiring total ordering. No
//! implicit wrapper changes the query language's comparison semantics.
//!
//! [`ZoneRows`] stores immutable decoded tuples with per-column block bounds.
//! A tuple [`ZoneSchema`] selects [`MinMax`] for exact supported types and
//! [`NoBounds`] for other fields. [`column()`] creates declaration-order markers
//! for cq-storage's positional reads and selections. Summaries always come
//! from decoded rows; block skipping saves in-memory row tests, not Parquet I/O.
//!
//! [`Loader::intern`] and, with `parquet`, `Loader::parquet_interned` prepare
//! sorted per-column [`Dictionaries`] and return rows with [`StringCode`] fields.
//! Opt in with [`interned_relation_loaders!`], which accepts the same IO
//! declarations as the plain [`relation_loaders!`] macro. It selects
//! source conversion through [`InternedField`]: codes borrow input strings into
//! column-local state; ordinary [`Load`] types remain unchanged. Each source cell
//! is hashed once, and only distinct dictionary entries allocate owned strings.
//! Finalization sorts distinct entries and remaps integer IDs in the loaded rows.
//! No source text is hashed or searched again during that remapping.
//! Collected CSV/Parquet/interned relations establish full-row sets. Parquet
//! loaders may skip sorting/deduplication with a matching trusted preparation
//! certificate (`set_manifest`); other loads canonicalize once. Grouped loaders
//! deduplicate each key/payload pair. Decoder batches remain raw streams.
//!
//! Codes preserve exact lexicographic string order within one column only.
//! Column names must be unique across the retained dictionaries; joining codes
//! from different columns does not compare their text. Translate query literals
//! once with [`Dictionaries::literal`] (`StringCode::MAX` means absent), and
//! prepare predicate relations at load time with [`Dictionaries::select`],
//! which evaluates a predicate once per distinct value. Never insert literals
//! into a loaded dictionary. Use [`Dictionaries::publish`] only for final owned
//! answers or [`Dictionaries::read`] for an explicit opaque Rust read.
//! [`StringDictionary`] also remains available for standalone `u32` encoding.
//!
//! [`verify_prefix_order`] checks loaded rows once and returns [`PrefixOrdered`],
//! an immutable, proof-carrying wrapper with the original logical row layout.
//! Equal prefixes and duplicate rows remain legal. Its retained comparison and
//! rows can be consumed by [`cq_storage::dedup::ordered`] or borrowed by
//! [`cq_storage::dedup::ordered_ref`] to sort and unique within prefix runs;
//! no unchecked sortedness or uniqueness declaration is accepted.
//!
//! [`stats_source::visit`] borrows a dynamic schema and streams the same CSV and
//! Arrow batches into statistics collection without retaining source rows.
//! Its literal-tab mode matches prepared integer inputs: no quoting, blank lines
//! skipped, CRLF accepted, and exact field counts. Scalar decoding remains the
//! batch consumer's responsibility.

mod columnar;
mod csv_source;
mod intern;
mod loader;
mod scalar;
pub mod set_manifest;
pub mod stats;
pub mod stats_manifest;
pub mod stats_source;
pub mod zone;
mod zone_docs;

#[cfg(feature = "parquet")]
mod parquet_source;

pub use columnar::{ColumnarField, ColumnarLoader, ColumnarRelation};
pub use cq_io_macros::{
    columnar_relation_loaders, interned_relation_loaders, relation_loaders,
};
pub use cq_storage::dedup::{PrefixOrderError, PrefixOrdered, verify_prefix_order};
pub use csv_source::{Csv, CsvOptions, read_csv};
pub use intern::{
    Dictionaries, InternedField, InternedRelation, StringCode, StringDictionary, StringInterner,
    dictionary_where,
};
pub use loader::{Group, Loader, canonicalize};
pub use scalar::{Date, Decimal};
pub use zone::{ColumnAt, MinMax, NoBounds, ZoneField, ZoneMetadata, ZoneRows, ZoneSchema, column};

#[cfg(feature = "parquet")]
pub use parquet_source::{ColumnError, Parquet, ParquetOptions, read_parquet, read_parquet_with_manifest};

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
    /// Source domain on which successful decoding is injective. Custom/lossy
    /// implementations must retain `None`; textual fallback is not certified.
    const KEY_ENCODING: Option<set_manifest::KeyEncoding> = None;
    // DECODE ?: Load is a generic source-to-logical conversion contract with no consumer boundary evidence.
    fn load(field: &str) -> std::result::Result<Self, String>;

    #[cfg(feature = "parquet")]
    // DECODE ?: default Arrow loading delegates logical reconstruction to a caller-defined Load implementation.
    fn load_arrow(column: &dyn arrow_array::Array) -> std::result::Result<Vec<Self>, ColumnError> {
        parquet_source::parse_arrow(column, Self::load)
    }

    /// Stream Arrow values into a column builder. Built-in scalar types avoid a
    /// temporary value vector; custom decoders retain their `load_arrow` behavior.
    #[cfg(feature = "parquet")]
    fn load_arrow_into(
        column: &dyn arrow_array::Array,
        mut emit: impl FnMut(Self),
    ) -> std::result::Result<(), ColumnError> {
        let values = Self::load_arrow(column)?;
        if values.len() != column.len() {
            return Err(ColumnError {
                row: 0,
                value: "<batch>".into(),
                message: format!(
                    "Load::load_arrow returned {} values for {} source rows",
                    values.len(),
                    column.len()
                ),
            });
        }
        for value in values {
            emit(value);
        }
        Ok(())
    }
}

/// Schema and typed batch decoder emitted from an authored relation declaration.
pub trait Relation {
    type Row;
    const COLUMNS: &'static [&'static str];
    /// Equality-preserving fields retained by this row decoder. Handwritten
    /// decoders default to no certificate support.
    const KEY_COLUMNS: &'static [(&'static str, Option<set_manifest::KeyEncoding>)] = &[];
    // DECODE ?: Relation requires owned logical rows but does not record whether callers publish or continue querying.
    fn decode(batch: &Batch<'_>) -> Result<Vec<Self::Row>>;
}

enum BatchData<'a> {
    Csv(&'a [csv::StringRecord]),
    #[cfg(feature = "parquet")]
    Parquet(&'a arrow_array::RecordBatch, u64),
}

/// Borrowed source batch, valid only while a decoder or visitor runs.
pub struct Batch<'a> {
    file: &'a Path,
    columns: &'a [&'a str],
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

    // DESIGN contract8-columnar-loader
    // Formal rule LD-ScanBlocks: source columns feed bounded tails; payload and
    // optional summary are emitted together, without a tuple-vector host.
    /// Append source values directly into their bounded scan block builder.
    pub fn append_column<T, F>(
        &self,
        index: usize,
        builder: &mut cq_storage::scan::ScanColumnBuilder<T, F>,
    ) -> Result<()>
    where
        T: Load + cq_storage::scan::OrdinalDecode,
        F: cq_storage::scan::Family,
    {
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
            BatchData::Csv(records) => {
                for record in records {
                    let value = &record[source];
                    let value = T::load(value).map_err(|message| {
                        Error::new(
                            self.file,
                            record.position().map_or(1, csv::Position::line),
                            column,
                            value,
                            message,
                        )
                    })?;
                    builder.push(value);
                }
                Ok(())
            }
            #[cfg(feature = "parquet")]
            BatchData::Parquet(batch, first_row) => {
                T::load_arrow_into(batch.column(source).as_ref(), |value| builder.push(value))
                    .map_err(|error| {
                        Error::new(
                            self.file,
                            first_row + error.row as u64,
                            column,
                            error.value,
                            error.message,
                        )
                    })
            }
        }
    }

    fn visit_strings(&self, index: usize, mut visit: impl FnMut(&str)) -> Result<()> {
        let Some((&_column, &source)) = self.columns.get(index).zip(self.mapping.get(index)) else {
            return Err(Error::new(
                self.file,
                0,
                "<schema>",
                index.to_string(),
                "column index outside schema",
            ));
        };
        match self.data {
            BatchData::Csv(records) => {
                for record in records {
                    visit(&record[source]);
                }
                Ok(())
            }
            #[cfg(feature = "parquet")]
            BatchData::Parquet(batch, first_row) => parquet_source::parse_arrow_into(
                batch.column(source).as_ref(),
                |value| {
                    visit(value);
                    Ok(())
                },
                |_| {},
            )
            .map_err(|error| {
                Error::new(
                    self.file,
                    first_row + error.row as u64,
                    _column,
                    error.value,
                    error.message,
                )
            }),
        }
    }

    /// Visit borrowed source text while its batch is alive. The closure returns
    /// only an encoded value; no string allocation is needed for string arrays.
    fn strings<T>(&self, index: usize, mut encode: impl FnMut(&str) -> T) -> Result<Vec<T>> {
        let Some((&_column, &source)) = self.columns.get(index).zip(self.mapping.get(index)) else {
            return Err(Error::new(
                self.file,
                0,
                "<schema>",
                index.to_string(),
                "column index outside schema",
            ));
        };
        match self.data {
            BatchData::Csv(records) => Ok(records
                .iter()
                .map(|record| encode(&record[source]))
                .collect()),
            #[cfg(feature = "parquet")]
            BatchData::Parquet(batch, first_row) => {
                parquet_source::string_values(batch.column(source).as_ref(), |value| {
                    Ok(encode(value))
                })
                .map_err(|error| {
                    Error::new(
                        self.file,
                        first_row + error.row as u64,
                        _column,
                        error.value,
                        error.message,
                    )
                })
            }
        }
    }

    /// Decode a declaration column, reporting the original field on failure.
    // DECODE ?: Batch::column materializes logical values for arbitrary Relation decoders; consumer facts are absent.
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
                    // DECODE ?: CSV field parsing invokes arbitrary T::load without a recorded downstream read boundary.
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
                // DECODE ?: Arrow column reconstruction invokes arbitrary T::load_arrow before its consumer is known.
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
