//! Streaming source adapters for statistics over dynamically named columns.
//!
//! CSV and Parquet share the typed loaders' readers, mappings, batch validation,
//! and scalar decoding. Literal tabs use the prepared SRD input convention,
//! rather than CSV quoting. This module neither hashes nor deduplicates rows.

use crate::csv_source::CsvReader;
use crate::{Batch, CsvOptions, DEFAULT_BATCH_SIZE, Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    #[default]
    Csv,
    LiteralTabs,
    Parquet,
}

/// Source decoding options. Non-CSV formats reject irrelevant CSV settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SourceOptions {
    pub format: SourceFormat,
    pub delimiter: u8,
    pub headers: bool,
    pub trailing_delimiter: bool,
    pub batch_size: usize,
}

impl Default for SourceOptions {
    fn default() -> Self {
        Self {
            format: SourceFormat::Csv,
            delimiter: b',',
            headers: false,
            trailing_delimiter: false,
            batch_size: DEFAULT_BATCH_SIZE,
        }
    }
}

impl SourceOptions {
    /// Validate the requested schema and effective options without opening data.
    pub fn check(&self, columns: &[&str]) -> Result<()> {
        validate(Path::new("<stats options>"), columns, self)
    }
}

/// Visit each bounded source batch once, in file order, borrowing column names.
///
/// The callback decodes cells through [`Batch`]. Any source or callback error
/// stops the traversal immediately. Empty inputs still validate their options
/// and schema. Literal tabs require a tab delimiter, no headers, and no optional
/// trailing delimiter. Parquet requires the default CSV-only settings (comma,
/// no headers, no trailing delimiter); its column names come from Arrow metadata.
pub fn visit(
    path: &Path,
    columns: &[&str],
    options: &SourceOptions,
    mut emit: impl FnMut(&Batch<'_>) -> Result<()>,
) -> Result<()> {
    validate(path, columns, options)?;
    match options.format {
        SourceFormat::Csv | SourceFormat::LiteralTabs => {
            let mut reader = if options.format == SourceFormat::LiteralTabs {
                CsvReader::literal_tabs(path, columns, options.batch_size)?
            } else {
                CsvReader::open(
                    path,
                    CsvOptions {
                        delimiter: options.delimiter,
                        headers: options.headers,
                        trailing_delimiter: options.trailing_delimiter,
                        batch_size: options.batch_size,
                    },
                    columns,
                )?
            };
            while let Some(result) = reader.next_with(&mut emit) {
                result?;
            }
            Ok(())
        }
        SourceFormat::Parquet => {
            #[cfg(feature = "parquet")]
            {
                let mut reader = crate::parquet_source::ParquetReader::open(
                    path,
                    crate::ParquetOptions { batch_size: options.batch_size },
                    columns,
                )?;
                while let Some(result) = reader.next_with(&mut emit) {
                    result?;
                }
                Ok(())
            }
            #[cfg(not(feature = "parquet"))]
            {
                Err(Error::new(
                    path,
                    0,
                    "<options>",
                    "parquet",
                    "Parquet statistics sources require the parquet feature",
                ))
            }
        }
    }
}

fn validate(path: &Path, columns: &[&str], options: &SourceOptions) -> Result<()> {
    if columns.is_empty() {
        return Err(Error::new(path, 0, "<schema>", "", "at least one column is required"));
    }
    for (index, name) in columns.iter().enumerate() {
        if name.is_empty() {
            return Err(Error::new(path, 0, "<schema>", "", "column name must not be empty"));
        }
        if columns[..index].contains(name) {
            return Err(Error::new(path, 0, *name, *name, "duplicate declared column"));
        }
    }
    CsvOptions {
        delimiter: options.delimiter,
        headers: options.headers,
        trailing_delimiter: options.trailing_delimiter,
        batch_size: options.batch_size,
    }.check(path)?;
    let message = match options.format {
        SourceFormat::LiteralTabs
            if options.delimiter != b'\t' || options.headers || options.trailing_delimiter =>
        {
            Some("literal tabs require delimiter TAB, headers=false, and trailing_delimiter=false")
        }
        SourceFormat::Parquet
            if options.delimiter != b',' || options.headers || options.trailing_delimiter =>
        {
            Some("Parquet requires default CSV-only options: delimiter comma, headers=false, and trailing_delimiter=false")
        }
        _ => None,
    };
    if let Some(message) = message {
        return Err(Error::new(path, 0, "<options>", "", message));
    }
    Ok(())
}
