use std::error::Error;

use cq_workloads::{QueryCase, dataset::Dataset};
use duckdb::{Connection, appender_params_from_iter};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub fn evaluate(case: &QueryCase, dataset: &Dataset) -> Result<Vec<Vec<i32>>> {
    let source = case.module()?;
    dataset.validate(&source)?;

    evaluate_sql(&dataset.relations, case.sql)
}

pub fn evaluate_sql(
    relations: &[cq_workloads::dataset::RelationData],
    sql: &str,
) -> Result<Vec<Vec<i32>>> {
    let connection = Connection::open_in_memory()?;
    for (input_index, relation) in relations.iter().enumerate() {
        let raw_name = cq_workloads::sql::raw_table_name(input_index);
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

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
