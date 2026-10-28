//! Exact integer prepared-input interface for SRDatalog's DOOP query: the
//! field-flow formulation by default, or SRDatalog's default formulation with
//! the `default-query` feature (the program its default benchmark builds).
use std::{fs::File, io::{BufRead, BufReader, BufWriter, Write}, path::Path};

pub type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

#[cfg(not(feature = "default-query"))]
mod generated;
#[cfg(feature = "default-query")]
#[path = "generated_default.rs"]
mod generated;
pub use generated::{SrdConstants, SrdFacts, SrdResults};

// The fixture asserts the 43 field-flow outputs.
#[cfg(all(feature = "test-fixture", feature = "default-query"))]
compile_error!("test-fixture covers only the field-flow query");
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
