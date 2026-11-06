use crate::{Batch, BatchData, DEFAULT_BATCH_SIZE, Error, Relation, Result, column_mapping};
use std::fs::File;
use std::io::{BufRead, BufReader};
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

impl CsvOptions {
    pub(crate) fn check(&self, file: &Path) -> Result<()> {
        if self.batch_size == 0 {
            return Err(Error::new(file, 0, "<options>", "0", "batch size must be positive"));
        }
        if !self.delimiter.is_ascii() || matches!(self.delimiter, 0 | b'\r' | b'\n' | b'"') {
            return Err(Error::new(file, 0, "<options>", self.delimiter.to_string(),
                "delimiter must be an ASCII byte other than NUL, a quote or a newline"));
        }
        Ok(())
    }
}

/// Single-pass CSV source with bounded batches and columnar ingestion support.
pub struct Csv<S> {
    reader: CsvReader<'static>,
    schema: PhantomData<S>,
}

pub(crate) struct CsvReader<'a> {
    reader: RecordReader,
    file: PathBuf,
    mapping: Vec<usize>,
    columns: &'a [&'a str],
    source_columns: Vec<&'a str>,
    options: CsvOptions,
    finished: bool,
}

impl<S: Relation> Csv<S> {
    pub fn open(path: impl AsRef<Path>, options: CsvOptions) -> Result<Self> {
        Self::open_columns(path, options, S::COLUMNS)
    }
}

impl<S> Csv<S> {
    pub(crate) fn open_columns(
        path: impl AsRef<Path>,
        options: CsvOptions,
        columns: &'static [&'static str],
    ) -> Result<Self> {
        Ok(Self {
            reader: CsvReader::open(path.as_ref(), options, columns)?,
            schema: PhantomData,
        })
    }

    pub(crate) fn next_with<T>(
        &mut self,
        decode: impl FnOnce(&Batch<'_>) -> Result<T>,
    ) -> Option<Result<T>> {
        self.reader.next_with(decode)
    }
}

impl<'a> CsvReader<'a> {
    pub(crate) fn open(
        path: &Path,
        options: CsvOptions,
        columns: &'a [&'a str],
    ) -> Result<Self> {
        let file = path;
        options.check(file)?;
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
                && names.len() == columns.len() + 1
                && names.last() == Some(&"")
            {
                names.pop();
            }
            column_mapping(file, &names, columns)?
        } else {
            (0..columns.len()).collect()
        };
        let mut source_columns = columns.to_vec();
        for (declared, &source) in mapping.iter().enumerate() {
            source_columns[source] = columns[declared];
        }
        Ok(Self {
            reader: RecordReader::Csv(reader),
            file: file.to_path_buf(),
            mapping,
            source_columns,
            columns,
            options,
            finished: false,
        })
    }

    pub(crate) fn literal_tabs(
        path: &Path,
        columns: &'a [&'a str],
        batch_size: usize,
    ) -> Result<Self> {
        let input = File::open(path)
            .map_err(|error| Error::new(path, 0, "<file>", "", error.to_string()))?;
        Ok(Self {
            reader: RecordReader::LiteralTabs {
                reader: BufReader::new(input),
                line: String::new(),
                position: csv::Position::new(),
            },
            file: path.to_path_buf(),
            mapping: (0..columns.len()).collect(),
            columns,
            source_columns: columns.to_vec(),
            options: CsvOptions {
                delimiter: b'\t',
                batch_size,
                ..CsvOptions::default()
            },
            finished: false,
        })
    }
}

impl CsvReader<'_> {
    pub(crate) fn next_with<T>(
        &mut self,
        decode: impl FnOnce(&Batch<'_>) -> Result<T>,
    ) -> Option<Result<T>> {
        if self.finished {
            return None;
        }
        let mut records = Vec::with_capacity(self.options.batch_size);
        for _ in 0..self.options.batch_size {
            let mut record = csv::StringRecord::new();
            match self.reader.read_record(&self.file, &mut record) {
                Ok(false) => {
                    self.finished = true;
                    break;
                }
                Err(error) => {
                    self.finished = true;
                    return Some(Err(error));
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
            columns: self.columns,
            mapping: &self.mapping,
            data: BatchData::Csv(&records),
        };
        let decoded = decode(&batch);
        if decoded.is_err() {
            self.finished = true;
        }
        Some(decoded)
    }
}

impl<S: Relation> Iterator for Csv<S> {
    type Item = Result<Vec<S::Row>>;

    fn next(&mut self) -> Option<Self::Item> {
        // DECODE V: row-mode CSV decoding precedes hash/btree grouping; P4 must retain encoded index keys. Columnar scans bypass this decoder.
        self.next_with(S::decode)
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

enum RecordReader {
    Csv(csv::Reader<File>),
    LiteralTabs {
        reader: BufReader<File>,
        line: String,
        position: csv::Position,
    },
}

impl RecordReader {
    fn read_record(&mut self, file: &Path, record: &mut csv::StringRecord) -> Result<bool> {
        match self {
            Self::Csv(reader) => reader.read_record(record).map_err(|error| csv_error(file, error)),
            Self::LiteralTabs { reader, line, position } => loop {
                line.clear();
                let bytes = reader.read_line(line).map_err(|error| {
                    Error::new(file, position.line(), "<record>", "<unreadable>", error.to_string())
                })?;
                if bytes == 0 {
                    return Ok(false);
                }
                record.set_position(Some(position.clone()));
                position.set_byte(position.byte() + bytes as u64);
                position.set_line(position.line() + 1);
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                if line.is_empty() {
                    continue;
                }
                record.extend(line.split('\t'));
                position.set_record(position.record() + 1);
                return Ok(true);
            },
        }
    }
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
