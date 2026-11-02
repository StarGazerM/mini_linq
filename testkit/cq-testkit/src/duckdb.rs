//! Optional independent in-memory SQL oracle over non-null integer tables.
//!
//! Input order determines quoted table names `mini-linq/raw/0`, etc. Raw
//! duplicates are retained. SQL controls output projection, ordering and
//! DISTINCT; `evaluate_fuzz` additionally verifies the printer's set output.
//! Opaque Rust operations and iterables have no SQL oracle and are rejected.

use crate::{Case, Result, fuzz};
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// A persistent prebuilt DuckDB CLI session; no DuckDB library is linked.
/// Set `DUCKDB_CLI` to select the executable, otherwise `duckdb` is found on PATH.
pub struct Connection {
    process: RefCell<CliProcess>,
    directory: tempfile::TempDir,
}

struct CliProcess {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Connection {
    pub fn open_in_memory() -> std::io::Result<Self> {
        Self::open(Path::new(":memory:"))
    }

    pub fn open(path: &Path) -> std::io::Result<Self> {
        let directory = tempfile::tempdir()?;
        let errors = std::fs::File::create(directory.path().join("stderr"))?;
        let binary = std::env::var_os("DUCKDB_CLI").unwrap_or_else(|| "duckdb".into());
        let mut child = Command::new(&binary)
            .args(["-batch", "-bail", "-init", "/dev/null"])
            .arg(path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(errors)
            .spawn()
            .map_err(|error| std::io::Error::other(format!(
                "cannot start DuckDB CLI {binary:?}: {error}; install DuckDB or set DUCKDB_CLI to its executable"
            )))?;
        let input = child.stdin.take().expect("piped stdin");
        let output = BufReader::new(child.stdout.take().expect("piped stdout"));
        Ok(Self {
            process: RefCell::new(CliProcess {
                child,
                input,
                output,
            }),
            directory,
        })
    }

    fn run(&self, sql: &str) -> std::io::Result<std::path::PathBuf> {
        let path = self.directory.path().join("result.csv");
        let mut process = self.process.borrow_mut();
        // All result bytes go to a file, so large results cannot deadlock pipes.
        // -bail exits on SQL errors before the completion marker can be emitted.
        let command = format!(
            ".headers off\n.mode csv\n.nullvalue \\\\N\n.output '{}'\n{}\n;\n.output stdout\n.print __CQ_DUCKDB_COMPLETE__\n",
            path.display(),
            sql
        );
        let sent = process
            .input
            .write_all(command.as_bytes())
            .and_then(|()| process.input.flush());
        let mut line = String::new();
        if sent.is_ok() {
            loop {
                line.clear();
                if process.output.read_line(&mut line)? == 0 {
                    break;
                }
                if line.trim_end() == "__CQ_DUCKDB_COMPLETE__" {
                    return Ok(path);
                }
            }
        }
        let errors = std::fs::read_to_string(self.directory.path().join("stderr"))?;
        Err(std::io::Error::other(format!(
            "DuckDB CLI failed: {errors}{}",
            sent.err().map(|e| e.to_string()).unwrap_or_default()
        )))
    }

    pub fn execute_batch(&self, sql: &str) -> std::io::Result<()> {
        self.run(sql)?;
        Ok(())
    }

    /// Exact textual values in projection order, including quoted CSV strings.
    pub fn query(&self, sql: &str) -> std::io::Result<Vec<Vec<String>>> {
        let path = self.run(sql)?;
        csv::ReaderBuilder::new()
            .has_headers(false)
            .from_path(path)
            .map_err(std::io::Error::other)?
            .records()
            .map(|record| {
                Ok(record
                    .map_err(std::io::Error::other)?
                    .iter()
                    .map(str::to_owned)
                    .collect())
            })
            .collect()
    }

    pub fn scalar(&self, sql: &str) -> std::io::Result<String> {
        let mut rows = self.query(sql)?;
        if rows.len() != 1 || rows[0].len() != 1 {
            return Err(std::io::Error::other(
                "DuckDB scalar query did not return exactly one value",
            ));
        }
        Ok(rows.remove(0).remove(0))
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let process = self.process.get_mut();
        let _ = process.input.write_all(b".quit\n");
        let _ = process.child.wait();
    }
}

/// A raw positional integer input, preserving duplicate tuples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationData {
    /// Used for diagnostics; SQL addresses tables by schema ordinal.
    pub name: String,
    pub arity: usize,
    pub rows: Vec<Vec<i32>>,
}

/// Evaluate independently authored SQL on a corpus case in declaration order.
pub fn evaluate_case(case: &Case, sql: &str) -> Result<Vec<Vec<i32>>> {
    let program = case.parse_cq()?;
    let relations = program
        .inputs()
        .map(|input| {
            let name = cq_syntax::symbol_name(&input.name);
            let (_, rows) = case
                .inputs
                .iter()
                .find(|(table, _)| *table == name)
                .ok_or_else(|| format!("{}: missing table {name}", case.name))?;
            Ok(RelationData {
                name,
                arity: input.columns.len(),
                rows: rows.iter().map(|row| row.to_vec()).collect(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    evaluate_sql(&relations, sql)
}

/// Evaluate the SQL-modeled fragment of a total abstract fuzz query.
pub fn evaluate_fuzz(case: &fuzz::Case) -> Result<Vec<Vec<i32>>> {
    if case.requires_rust_oracle() {
        return Err(
            "opaque Rust operations and generators require the compiled written-order reference"
                .into(),
        );
    }
    if case.query.schema.len() != case.data.len() {
        return Err("fuzz schema/data table counts differ".into());
    }
    let relations = case
        .query
        .schema
        .iter()
        .zip(&case.data)
        .map(|(relation, rows)| RelationData {
            name: relation.name.clone(),
            arity: relation.arity,
            rows: rows.clone(),
        })
        .collect::<Vec<_>>();
    let mut rows = evaluate_sql(&relations, &case.sql())?;
    rows.sort();
    if rows.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err("independent SQL printer failed to enforce DISTINCT".into());
    }
    Ok(rows)
}

/// Load raw inputs through CSV and return the SQL's ordered integer rows.
///
/// SQL must use the ordinal table names returned by `raw_table_name`. All
/// output columns must be non-null and convertible to `i32`.
pub fn evaluate_sql(relations: &[RelationData], sql: &str) -> Result<Vec<Vec<i32>>> {
    let connection = Connection::open_in_memory()?;
    for (input_index, relation) in relations.iter().enumerate() {
        if relation.arity == 0 || relation.rows.iter().any(|row| row.len() != relation.arity) {
            return Err(format!("{}: raw input has invalid tuple width", relation.name).into());
        }
        let raw_name = raw_table_name(input_index);
        let columns = (0..relation.arity)
            .map(|column| format!("c{column} INTEGER NOT NULL"))
            .collect::<Vec<_>>()
            .join(", ");
        connection.execute_batch(&format!(
            "CREATE TABLE {} ({columns});",
            quote_identifier(&raw_name),
        ))?;
        let path = connection
            .directory
            .path()
            .join(format!("input-{input_index}.csv"));
        {
            let mut writer = csv::WriterBuilder::new()
                .has_headers(false)
                .from_path(&path)?;
            for row in &relation.rows {
                writer.write_record(row.iter().map(i32::to_string))?;
            }
            writer.flush()?;
        }
        if !relation.rows.is_empty() {
            connection.execute_batch(&format!(
                "COPY {} FROM '{}' (FORMAT CSV, HEADER false);",
                quote_identifier(&raw_name),
                path.to_string_lossy().replace('\'', "''"),
            ))?;
        }
    }

    connection
        .query(sql)?
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|value| value.parse::<i32>().map_err(Into::into))
                .collect()
        })
        .collect()
}

/// Name of the raw table corresponding to a schema ordinal.
pub fn raw_table_name(input_index: usize) -> String {
    format!("mini-linq/raw/{input_index}")
}

/// Quote an SQL identifier, including embedded double quotes.
pub fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
