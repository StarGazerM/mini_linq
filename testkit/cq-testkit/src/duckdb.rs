//! Optional independent in-memory SQL oracle over non-null integer tables.
//!
//! Input order determines quoted table names `mini-linq/raw/0`, etc. Raw
//! duplicates are retained. SQL controls output projection, ordering and
//! DISTINCT; `evaluate_fuzz` additionally verifies the printer's set output.
//! Partial Rust expressions have no SQL oracle and are explicitly rejected.

use crate::{Case, Result, fuzz};
use ::duckdb::{Connection, appender_params_from_iter};

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

/// Evaluate a total abstract fuzz query using its independent SQL printer.
pub fn evaluate_fuzz(case: &fuzz::Case) -> Result<Vec<Vec<i32>>> {
    if case.has_partial_terms() {
        return Err(
            "partial terms and generators require the compiled written-order reference".into(),
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

/// Load raw inputs with appenders and return the SQL's ordered integer rows.
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
        {
            let mut appender = connection.appender(&raw_name)?;
            for row in &relation.rows {
                appender.append_row(appender_params_from_iter(row.iter().copied()))?;
            }
            appender.flush()?;
        }
    }

    let mut statement = connection.prepare(sql)?;
    let mut query = statement.query([])?;
    let arity = query
        .as_ref()
        .expect("DuckDB query rows retain their prepared statement")
        .column_count();
    let mut result = Vec::new();
    while let Some(row) = query.next()? {
        result.push(
            (0..arity)
                .map(|column| row.get::<_, i32>(column))
                .collect::<duckdb::Result<Vec<_>>>()?,
        );
    }
    Ok(result)
}

/// Name of the raw table corresponding to a schema ordinal.
pub fn raw_table_name(input_index: usize) -> String {
    format!("mini-linq/raw/{input_index}")
}

/// Quote an SQL identifier, including embedded double quotes.
pub fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
