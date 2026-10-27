//! Exact integer prepared-input interface for SRDatalog's field-flow DOOP query.
use std::{fs::File, io::{BufRead, BufReader, BufWriter, Write}, path::Path};

pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

mod generated;
pub use generated::{SrdConstants, SrdFacts, SrdResults};

#[cfg(feature = "test-fixture")]
pub mod fixture;

fn load_rows<const N: usize, T>(path: &Path, make: impl Fn([i32; N]) -> T) -> Result<Vec<T>> {
    let mut rows = Vec::new();
    for (line_no, line) in BufReader::new(File::open(path)?).lines().enumerate() {
        let line = line?;
        if line.is_empty() { continue; }
        let mut row = [0; N];
        let mut columns = line.split('\t');
        for value in &mut row {
            let column = columns.next().ok_or_else(|| format!("{}:{}: expected {N} columns", path.display(), line_no + 1))?;
            *value = column.parse().map_err(|error| format!("{}:{}: invalid i32 {column:?}: {error}", path.display(), line_no + 1))?;
        }
        if columns.next().is_some() {
            return Err(format!("{}:{}: expected {N} columns", path.display(), line_no + 1).into());
        }
        rows.push(make(row));
    }
    Ok(rows)
}

fn export_rows<const N: usize>(path: &Path, rows: impl Iterator<Item = [i32; N]>) -> Result<()> {
    let mut output = BufWriter::new(File::create(path)?);
    for row in rows {
        for (i, value) in row.iter().enumerate() {
            if i != 0 { write!(output, "\t")?; }
            write!(output, "{value}")?;
        }
        writeln!(output)?;
    }
    output.flush()?;
    Ok(())
}
