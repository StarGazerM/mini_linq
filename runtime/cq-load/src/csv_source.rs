use crate::{Batch, BatchData, DEFAULT_BATCH_SIZE, Error, Relation, Result, column_mapping};
use std::fs::File;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug)]
pub struct CsvOptions {
    pub delimiter: u8,
    pub headers: bool,
    pub batch_size: usize,
    /// Accept exactly one extra, empty field at the end of every record/header.
    pub trailing_delimiter: bool,
}

impl Default for CsvOptions {
    fn default() -> Self {
        Self {
            delimiter: b',',
            headers: false,
            batch_size: DEFAULT_BATCH_SIZE,
            trailing_delimiter: false,
        }
    }
}

/// Single-pass CSV source yielding at most `batch_size` owned tuples per batch.
pub struct Csv<S> {
    reader: csv::Reader<File>,
    file: PathBuf,
    mapping: Vec<usize>,
    source_columns: Vec<&'static str>,
    options: CsvOptions,
    finished: bool,
    schema: PhantomData<S>,
}

impl<S: Relation> Csv<S> {
    pub fn open(path: impl AsRef<Path>, options: CsvOptions) -> Result<Self> {
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
        if !options.delimiter.is_ascii() || matches!(options.delimiter, 0 | b'\r' | b'\n' | b'"') {
            return Err(Error::new(
                file,
                0,
                "<options>",
                options.delimiter.to_string(),
                "delimiter must be an ASCII byte other than NUL, a quote or a newline",
            ));
        }
        let input = File::open(file)
            .map_err(|error| Error::new(file, 0, "<file>", "", error.to_string()))?;
        let mut reader = csv::ReaderBuilder::new()
            .delimiter(options.delimiter)
            .has_headers(options.headers)
            .flexible(true)
            .from_reader(input);
        let mapping = if options.headers {
            let header = reader.headers().map_err(|error| csv_error(file, error))?;
            let mut names = header.iter().collect::<Vec<_>>();
            if options.trailing_delimiter
                && names.len() == S::COLUMNS.len() + 1
                && names.last() == Some(&"")
            {
                names.pop();
            }
            column_mapping(file, &names, S::COLUMNS)?
        } else {
            (0..S::COLUMNS.len()).collect()
        };
        let mut source_columns = S::COLUMNS.to_vec();
        for (declared, &source) in mapping.iter().enumerate() {
            source_columns[source] = S::COLUMNS[declared];
        }
        Ok(Self {
            reader,
            file: file.to_path_buf(),
            mapping,
            source_columns,
            options,
            finished: false,
            schema: PhantomData,
        })
    }
}

impl<S: Relation> Iterator for Csv<S> {
    type Item = Result<Vec<S::Row>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }
        let mut records = Vec::with_capacity(self.options.batch_size);
        for _ in 0..self.options.batch_size {
            let mut record = csv::StringRecord::new();
            match self.reader.read_record(&mut record) {
                Ok(false) => {
                    self.finished = true;
                    break;
                }
                Err(error) => {
                    self.finished = true;
                    return Some(Err(csv_error(&self.file, error)));
                }
                Ok(true) => {}
            }
            if let Err(error) = validate_width(
                &self.file,
                &self.source_columns,
                &record,
                self.options.trailing_delimiter,
            ) {
                self.finished = true;
                return Some(Err(error));
            }
            records.push(record);
        }
        if records.is_empty() {
            return None;
        }
        let batch = Batch {
            file: &self.file,
            columns: S::COLUMNS,
            mapping: &self.mapping,
            data: BatchData::Csv(&records),
        };
        // DECODE V: Loader::csv_hash/csv_btree group these decoded rows; P4 keeps index keys encoded and P5 builds scan columns directly.
        let decoded = S::decode(&batch);
        if decoded.is_err() {
            self.finished = true;
        }
        Some(decoded)
    }
}

impl<S: Relation> std::iter::FusedIterator for Csv<S> {}

// DECODE ?: read_csv collects the logical batch decoder for arbitrary callers without a recorded final-use boundary.
pub fn read_csv<S: Relation>(path: impl AsRef<Path>, options: CsvOptions) -> Result<Vec<S::Row>> {
    // DECODE ?: try_fold drives Csv::next reconstruction; caller-specific relational and diagnostic uses are classified at their sites.
    Csv::<S>::open(path, options)?.try_fold(Vec::new(), |mut rows, batch| {
        rows.extend(batch?);
        Ok(rows)
    })
}

fn validate_width(
    file: &Path,
    columns: &[&str],
    record: &csv::StringRecord,
    trailing: bool,
) -> Result<()> {
    let expected = columns.len();
    let extra_empty = trailing && record.len() == expected + 1 && record.get(expected) == Some("");
    if record.len() == expected || extra_empty {
        return Ok(());
    }
    let line = record.position().map_or(1, csv::Position::line);
    let (column, value) = if record.len() < expected {
        (columns[record.len()], "<missing>")
    } else {
        ("<extra>", record.get(expected).unwrap_or(""))
    };
    Err(Error::new(
        file,
        line,
        column,
        value,
        format!("expected {expected} columns, found {}", record.len()),
    ))
}

fn csv_error(file: &Path, error: csv::Error) -> Error {
    Error::new(
        file,
        error.position().map_or(0, csv::Position::line),
        "<record>",
        "<unreadable>",
        error.to_string(),
    )
}
