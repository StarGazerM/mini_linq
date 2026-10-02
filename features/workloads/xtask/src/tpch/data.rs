use crate::Result;
use duckdb::Connection;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub(super) fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(super) struct Database {
    pub connection: Connection,
    pub directory: PathBuf,
    pub version: String,
    pub default_threads: usize,
}

pub(super) fn dbgen(root: &Path, sf: &str) -> Result<Database> {
    let directory = root.join("target/bench/data").join(format!("tpch-{sf}"));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("dbgen.duckdb");
    let probe = Connection::open_in_memory()?;
    let version: String = probe.query_row("SELECT version()", [], |row| row.get(0))?;
    let threads: usize =
        probe.query_row("SELECT current_setting('threads')", [], |row| row.get(0))?;
    drop(probe);
    let expected = format!("TPC-H-derived\nDuckDB {version}\nCALL dbgen(sf={sf})\n");
    let marker = directory.join("dbgen.manifest");
    if !path.exists() || std::fs::read_to_string(&marker).ok().as_deref() != Some(&expected) {
        eprintln!(
            "generating TPC-H-derived SF {sf} using DuckDB {version} (excluded from query timing)..."
        );
        let temporary = directory.join("dbgen-building.duckdb");
        for file in [temporary.clone(), temporary.with_extension("duckdb.wal")] {
            if file.exists() {
                std::fs::remove_file(file)?;
            }
        }
        let connection = Connection::open(&temporary)?;
        let extensions = root.join("target/bench/extensions");
        std::fs::create_dir_all(&extensions)?;
        connection.execute_batch(&format!(
            "SET extension_directory={};",
            literal(&extensions.to_string_lossy())
        ))?;
        if connection.execute_batch("LOAD tpch").is_err() {
            install_tpch(&connection, &extensions, &version)?;
        }
        let started = Instant::now();
        connection.execute_batch(&format!("CALL dbgen(sf={sf}); CHECKPOINT;"))?;
        drop(connection);
        std::fs::rename(temporary, &path)?;
        std::fs::write(&marker, expected)?;
        eprintln!("dbgen SF {sf}: {:.3} s", started.elapsed().as_secs_f64());
    }
    let connection = Connection::open(&path)?;
    Ok(Database {
        connection,
        directory,
        version,
        default_threads: threads,
    })
}

fn install_tpch(connection: &Connection, extensions: &Path, version: &str) -> Result<()> {
    // DuckDB's HTTPS repository route first needs httpfs. Download the official
    // signed archive directly, then let DuckDB install and verify it locally.
    let platform: String = connection.query_row("PRAGMA platform", [], |row| row.get(0))?;
    let download = extensions.join("downloads").join(version).join(&platform);
    std::fs::create_dir_all(&download)?;
    let archive = download.join("tpch.duckdb_extension.gz");
    if !archive.exists() {
        let url =
            format!("https://extensions.duckdb.org/{version}/{platform}/tpch.duckdb_extension.gz");
        let partial = archive.with_extension("gz.partial");
        eprintln!("downloading signed DuckDB tpch extension from {url}");
        let status = std::process::Command::new("curl")
            .args([
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--retry",
                "2",
                "--output",
            ])
            .arg(&partial)
            .arg(&url)
            .status()?;
        if !status.success() {
            return Err(format!("DuckDB tpch download failed ({status}); network access to extensions.duckdb.org is required on first use; cache {}", archive.display()).into());
        }
        std::fs::rename(partial, &archive)?;
    }
    connection.execute_batch(&format!(
        "INSTALL {}; LOAD tpch;",
        literal(&archive.to_string_lossy())
    ))?;
    Ok(())
}

pub(super) fn export(
    database: &Database,
    name: &str,
    exports: &[(String, String)],
) -> Result<PathBuf> {
    let directory = database.directory.join(name);
    std::fs::create_dir_all(&directory)?;
    for (table, columns) in exports {
        // Only actual dbgen columns may be exported; predicates and expression
        // evaluation belong in the authored queries, inside query timing.
        if !identifier(table) || !columns.split(',').all(|c| identifier(c.trim())) {
            return Err(
                format!("invalid dbgen table/column projection: {table} ({columns})").into(),
            );
        }
        let parquet = directory.join(format!("{table}.parquet"));
        let marker = directory.join(format!("{table}.manifest"));
        let declaration = format!(
            "TPC-H-derived\nDuckDB {}\nSELECT {columns} FROM {table}\n",
            database.version
        );
        if !parquet.exists()
            || std::fs::read_to_string(&marker).ok().as_deref() != Some(&declaration)
        {
            let temp = parquet.with_extension("parquet.partial");
            if temp.exists() {
                std::fs::remove_file(&temp)?;
            }
            database.connection.execute_batch(&format!(
                "COPY (SELECT {columns} FROM {table}) TO {} (FORMAT PARQUET, COMPRESSION SNAPPY, ROW_GROUP_SIZE 2048)",
                literal(&temp.to_string_lossy())
            ))?;
            std::fs::rename(temp, &parquet)?;
            std::fs::write(marker, declaration)?;
        }
    }
    Ok(directory)
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

pub(super) fn preload(directory: &Path, exports: &[(String, String)]) -> Result<(Connection, f64)> {
    let connection = Connection::open_in_memory()?;
    let started = Instant::now();
    for (table, _) in exports {
        connection.execute_batch(&format!(
            "CREATE TABLE {table} AS SELECT * FROM read_parquet({})",
            literal(&directory.join(format!("{table}.parquet")).to_string_lossy())
        ))?;
    }
    Ok((connection, started.elapsed().as_secs_f64() * 1000.0))
}
