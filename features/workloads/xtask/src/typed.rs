//! Typed golden fixtures use declarations for loading and independent SQL for answers.

use std::path::PathBuf;

use cq_workloads::typed::Case;
use duckdb::Connection;

use crate::{Result, Scale, ScenarioSelection, Selection};

pub(super) fn golden(
    selection: Selection,
    scale: Scale,
    scenarios: ScenarioSelection,
    write: bool,
) -> Result<Vec<PathBuf>> {
    if !matches!(
        scenarios,
        ScenarioSelection::All | ScenarioSelection::One(crate::Scenario::Coverage)
    ) {
        return Ok(Vec::new());
    }
    let cases = selection.typed().collect::<Vec<_>>();
    if cases.is_empty() {
        return Ok(Vec::new());
    }
    if scale != Scale::Tiny {
        return Err("the typed corpus supports tiny fixtures only".into());
    }
    let mut checked = Vec::new();
    for case in cases {
        let connection = load(case, write)?;
        let sql = format!(
            "SELECT CAST(COLUMNS(*) AS VARCHAR) FROM ({}) AS answer",
            case.sql.trim().trim_end_matches(';'),
        );
        let mut statement = connection.prepare(&sql)?;
        let mut query = statement.query([])?;
        let arity = query.as_ref().expect("live prepared query").column_count();
        let mut rows = Vec::new();
        while let Some(row) = query.next()? {
            rows.push(
                (0..arity)
                    .map(|i| row.get::<_, String>(i))
                    .collect::<duckdb::Result<Vec<_>>>()?,
            );
        }
        let bytes = cq_workloads::typed::canonical_bytes(rows);
        let path = case.path("golden").with_extension("csv");
        if write {
            std::fs::create_dir_all(path.parent().expect("golden parent"))?;
            std::fs::write(&path, &bytes)?;
        } else {
            crate::golden::compare_bytes(&path, &bytes).map_err(crate::testkit_error)?;
        }
        println!("golden {:<27} typed CSV + Parquet", case.name);
        checked.push(path);
    }
    // The gate includes execution of the same authored CQ over generated
    // loaders for both source formats, in addition to oracle recomputation.
    let status = crate::cargo::command()
        .current_dir(crate::repository_root())
        .args([
            "test",
            "--locked",
            "-p",
            "cq-workload-e2e",
            "--test",
            "typed",
            "--features",
            "parquet",
        ])
        .status()?;
    if !status.success() {
        return Err(format!("typed CSV/Parquet execution gate failed: {status}").into());
    }
    Ok(checked)
}

fn load(case: &Case, write: bool) -> Result<Connection> {
    let connection = Connection::open_in_memory()?;
    for relation in case.schema {
        let name = quote_identifier(relation.name);
        let columns = relation
            .columns
            .iter()
            .map(|(name, ty)| {
                format!("{} {} NOT NULL", quote_identifier(name), sql_type(*ty))
            })
            .collect::<Vec<_>>()
            .join(", ");
        let csv = case.path("data").join(format!("{}.csv", relation.name));
        let parquet = csv.with_extension("parquet");
        connection.execute_batch(&format!(
            "CREATE TABLE {name} ({columns}); COPY {name} FROM {} (HEADER true, NULLSTR '\\N');",
            quote_string(&csv.to_string_lossy()),
        ))?;
        if write {
            connection.execute_batch(&format!(
                "COPY {name} TO {} (FORMAT PARQUET, COMPRESSION SNAPPY, ROW_GROUP_SIZE 2048);",
                quote_string(&parquet.to_string_lossy()),
            ))?;
        }
        let mismatch: bool = connection.query_row(&format!(
            "SELECT EXISTS ((SELECT * FROM {name} EXCEPT ALL SELECT * FROM read_parquet({path})) UNION ALL (SELECT * FROM read_parquet({path}) EXCEPT ALL SELECT * FROM {name}))",
            path = quote_string(&parquet.to_string_lossy()),
        ), [], |row| row.get(0))?;
        if mismatch {
            return Err(format!(
                "{} has different rows from {}",
                parquet.display(),
                csv.display()
            )
            .into());
        }
    }
    Ok(connection)
}

fn sql_type(ty: cq_workloads::schema::ColumnType) -> &'static str {
    use cq_workloads::schema::ColumnType;
    match ty {
        ColumnType::I32 => "INTEGER",
        ColumnType::I64 => "BIGINT",
        ColumnType::Text => "VARCHAR",
        ColumnType::Date => "DATE",
        // A scaled i64 can use nineteen decimal digits, including i64::MIN.
        ColumnType::Decimal => "DECIMAL(19,2)",
    }
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn quote_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
