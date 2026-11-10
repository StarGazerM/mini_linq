//! Export raw dbgen projections and certify keys against the resulting Parquet.
//! Adjacent `.parquet.set.json` files bind verified keys to whole-file SHA-256.
//! Existing exports are upgraded on preparation without rewriting their rows.
//! CPU loaders discover these certificates automatically; `CQ_LOAD_MANIFEST`
//! overrides the manifest path (one JSON manifest may contain multiple files).
//! A missing/stale certificate or a projection omitting every key still dedups.
use crate::Result;
use cq_testkit::duckdb::Connection;
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
    let version = probe.scalar("SELECT version()")?;
    let threads = probe.scalar("SELECT current_setting('threads')")?.parse()?;
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
        install_tpch(&connection, &extensions, &version)?;
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
    let platform = connection.scalar("PRAGMA platform")?;
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
        certify(&database.connection, table, columns, &parquet)?;
    }
    Ok(directory)
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn primary_key(table: &str) -> &'static [&'static str] {
    match table {
        "lineitem" => &["l_orderkey", "l_linenumber"],
        "orders" => &["o_orderkey"],
        "partsupp" => &["ps_partkey", "ps_suppkey"],
        "part" => &["p_partkey"],
        "supplier" => &["s_suppkey"],
        "customer" => &["c_custkey"],
        "nation" => &["n_nationkey"],
        "region" => &["r_regionkey"],
        _ => &[],
    }
}

fn certify(connection: &Connection, table: &str, columns: &str, parquet: &Path) -> Result<()> {
    use cq_load::set_manifest::{self, Manifest};
    let path = set_manifest::adjacent_path(parquet);
    let sha256 = set_manifest::file_sha256(parquet)?;
    let file = parquet.file_name().and_then(|name| name.to_str())
        .ok_or("non-UTF8 Parquet filename")?;
    if std::fs::read(&path).ok()
        .and_then(|bytes| serde_json::from_slice::<Manifest>(&bytes).ok())
        .is_some_and(|manifest| manifest.schema == set_manifest::SCHEMA
            && manifest.files.len() == 1
            && manifest.files[0].file == file
            && manifest.files[0].sha256 == sha256
            && ((manifest.files[0].set == "verified" && !manifest.files[0].verified_keys.is_empty())
                || (manifest.files[0].set == "uncertified" && manifest.files[0].verified_keys.is_empty())))
    {
        return Ok(());
    }
    let record = certify_file(connection, table, columns, parquet, sha256)?;
    let manifest = Manifest { schema: set_manifest::SCHEMA.into(), files: vec![record] };
    write_manifest(&path, &manifest)
}

fn write_manifest(path: &Path, manifest: &cq_load::set_manifest::Manifest) -> Result<()> {
    let temporary = path.with_extension("json.partial");
    std::fs::write(&temporary, serde_json::to_vec_pretty(manifest)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

/// Formal rule IO-CpuPrepareKey: verify actual exported key values, then bind
/// the successful certificate to the immutable file's SHA-256, not dbgen lore.
fn certify_file(
    connection: &Connection,
    table: &str,
    columns: &str,
    parquet: &Path,
    sha256: String,
) -> Result<cq_load::set_manifest::CertifiedFile> {
    let heading = columns.split(',').map(str::trim).collect::<Vec<_>>();
    let primary = primary_key(table);
    let key = if !primary.is_empty() && primary.iter().all(|column| heading.contains(column)) {
        primary
    } else {
        heading.as_slice()
    };
    let verified_keys = if verify_key(connection, parquet, key)? {
        vec![key.iter().map(|column| (*column).to_owned()).collect()]
    } else {
        Vec::new()
    };
    if cq_load::set_manifest::file_sha256(parquet)? != sha256 {
        return Err(format!("Parquet changed during key verification: {}", parquet.display()).into());
    }
    Ok(cq_load::set_manifest::CertifiedFile {
        file: parquet.file_name().and_then(|name| name.to_str())
            .ok_or("non-UTF8 Parquet filename")?.into(),
        sha256,
        set: if verified_keys.is_empty() { "uncertified" } else { "verified" }.into(),
        verified_keys,
    })
}

fn verify_key(connection: &Connection, parquet: &Path, columns: &[&str]) -> Result<bool> {
    if columns.is_empty() || !columns.iter().all(|column| identifier(column)) {
        return Ok(false);
    }
    let source = format!("read_parquet({})", literal(&parquet.to_string_lossy()));
    let nulls = columns.iter().map(|column| format!("{column} IS NULL")).collect::<Vec<_>>().join(" OR ");
    let columns = columns.join(",");
    // GROUP BY catches repeated keys even when payloads differ; no declared
    // primary-key constraint, sample, or full-row count substitutes for this.
    Ok(connection.scalar(&format!(
        "SELECT CASE WHEN NOT EXISTS (SELECT 1 FROM {source} GROUP BY {columns} \
         HAVING count(*) > 1 OR {nulls}) THEN 1 ELSE 0 END"
    ))? == "1")
}

/// Certify exactly the supplied files, writing only the requested JSON output.
/// No dbgen, export, adjacent manifest, or dataset file is modified.
pub(crate) fn certify_existing(arguments: Vec<String>) -> Result<()> {
    use cq_load::set_manifest::{self, Manifest};
    let mut arguments = arguments.into_iter();
    if arguments.next().as_deref() != Some("--output") {
        return Err("tpch-manifest requires --output FILE.json INPUT.parquet...".into());
    }
    let output = PathBuf::from(arguments.next().ok_or("--output requires a JSON path")?);
    if output.extension().and_then(|extension| extension.to_str()) != Some("json") {
        return Err("tpch-manifest output must have a .json extension".into());
    }
    let inputs = arguments.map(PathBuf::from).collect::<Vec<_>>();
    if inputs.is_empty() {
        return Err("tpch-manifest requires at least one explicit Parquet input".into());
    }
    let mut names = std::collections::BTreeSet::new();
    for input in &inputs {
        if input.extension().and_then(|extension| extension.to_str()) != Some("parquet")
            || !names.insert(input.file_name().ok_or("input requires a filename")?)
        {
            return Err("inputs must be .parquet files with distinct filenames".into());
        }
    }
    let connection = Connection::open_in_memory()?;
    let mut files = Vec::with_capacity(inputs.len());
    for input in &inputs {
        let sha256 = set_manifest::file_sha256(input)?;
        let table = input.file_stem().and_then(|name| name.to_str()).ok_or("non-UTF8 input filename")?;
        let schema = connection.query(&format!(
            "DESCRIBE SELECT * FROM read_parquet({})", literal(&input.to_string_lossy())
        ))?;
        let columns = schema.iter().map(|row| row[0].as_str()).collect::<Vec<_>>().join(",");
        files.push(certify_file(&connection, table, &columns, input, sha256)?);
    }
    write_manifest(&output, &Manifest { schema: set_manifest::SCHEMA.into(), files })
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

#[cfg(test)]
mod tests {
    use super::*;
    use cq_load::set_manifest::{self, Manifest};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Directory(PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "cpu-tpch-certificates-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    fn manifest(path: &Path) -> Manifest {
        serde_json::from_slice(&std::fs::read(set_manifest::adjacent_path(path)).unwrap()).unwrap()
    }

    #[test]
    fn preparation_verifies_keys_and_preserves_duplicate_projections() {
        let directory = Directory::new();
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE orders(o_orderkey BIGINT, payload BIGINT); \
             INSERT INTO orders VALUES (3, 7), (1, 7), (2, 8);"
        ).unwrap();
        let database = Database {
            connection, directory: directory.0.clone(), version: "test".into(), default_threads: 1,
        };
        let full = export(&database, "full", &[("orders".into(), "o_orderkey,payload".into())]).unwrap();
        let parquet = full.join("orders.parquet");
        let certified = manifest(&parquet);
        assert_eq!(certified.files[0].verified_keys, vec![vec!["o_orderkey".to_owned()]]);
        assert_eq!(certified.files[0].set, "verified");
        assert_eq!(certified.files[0].sha256, set_manifest::file_sha256(&parquet).unwrap());
        let projected = export(&database, "projected", &[("orders".into(), "payload".into())]).unwrap();
        let projected = projected.join("orders.parquet");
        assert!(manifest(&projected).files[0].verified_keys.is_empty());
        assert_eq!(manifest(&projected).files[0].set, "uncertified");
        assert_eq!(database.connection.scalar(&format!(
            "SELECT count(*) FROM read_parquet({})", literal(&projected.to_string_lossy())
        )).unwrap(), "3");

        // Old prepared data without certificates is upgraded without rewriting.
        let original = std::fs::read(&parquet).unwrap();
        std::fs::remove_file(set_manifest::adjacent_path(&parquet)).unwrap();
        export(&database, "full", &[("orders".into(), "o_orderkey,payload".into())]).unwrap();
        assert_eq!(std::fs::read(&parquet).unwrap(), original);
        assert_eq!(manifest(&parquet).files[0].verified_keys, certified.files[0].verified_keys);

        // Manifest-only adoption writes outside the input directory and leaves
        // both the Parquet bytes and adjacent certificate untouched.
        let adjacent = std::fs::read(set_manifest::adjacent_path(&parquet)).unwrap();
        let output = directory.0.join("override.json");
        certify_existing(vec![
            "--output".into(), output.to_string_lossy().into_owned(),
            parquet.to_string_lossy().into_owned(),
        ]).unwrap();
        let override_manifest: Manifest = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(override_manifest.files[0].set, "verified");
        assert_eq!(override_manifest.files[0].verified_keys, certified.files[0].verified_keys);
        assert_eq!(std::fs::read(&parquet).unwrap(), original);
        assert_eq!(std::fs::read(set_manifest::adjacent_path(&parquet)).unwrap(), adjacent);
    }

    #[test]
    fn preparation_does_not_trust_declared_primary_keys_or_nulls() {
        let directory = Directory::new();
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(
            "CREATE TABLE orders(o_orderkey BIGINT, payload BIGINT); \
             INSERT INTO orders VALUES (1, 7), (1, 8);"
        ).unwrap();
        let database = Database {
            connection, directory: directory.0.clone(), version: "test".into(), default_threads: 1,
        };
        let full = export(&database, "full", &[("orders".into(), "o_orderkey,payload".into())]).unwrap();
        let parquet = full.join("orders.parquet");
        assert!(manifest(&parquet).files[0].verified_keys.is_empty());
        assert_eq!(manifest(&parquet).files[0].set, "uncertified");
        // Replacing bytes invalidates the old content-bound record.
        std::fs::remove_file(&parquet).unwrap();
        database.connection.execute_batch(&format!(
            "COPY (SELECT * FROM (VALUES (1,7), (1,7)) AS t(o_orderkey,payload)) \
             TO {} (FORMAT PARQUET)", literal(&parquet.to_string_lossy())
        )).unwrap();
        certify(&database.connection, "orders", "o_orderkey,payload", &parquet).unwrap();
        assert!(manifest(&parquet).files[0].verified_keys.is_empty());
        std::fs::remove_file(&parquet).unwrap();
        database.connection.execute_batch(&format!(
            "COPY (SELECT NULL::BIGINT AS o_orderkey, 7 AS payload) \
             TO {} (FORMAT PARQUET)", literal(&parquet.to_string_lossy())
        )).unwrap();
        certify(&database.connection, "orders", "o_orderkey,payload", &parquet).unwrap();
        assert!(manifest(&parquet).files[0].verified_keys.is_empty());
    }

    #[test]
    fn heading_verification_and_manifest_only_argument_contract() {
        let directory = Directory::new();
        let connection = Connection::open_in_memory().unwrap();
        let parquet = directory.0.join("orders.parquet");
        connection.execute_batch(&format!(
            "COPY (SELECT * FROM (VALUES (2,7), (1,7)) AS t(x,y)) TO {} (FORMAT PARQUET)",
            literal(&parquet.to_string_lossy()),
        )).unwrap();
        let output = directory.0.join("override.json");
        let arguments = vec![
            "--output".into(), output.to_string_lossy().into_owned(),
            parquet.to_string_lossy().into_owned(),
        ];
        certify_existing(arguments.clone()).unwrap();
        let certificate: Manifest = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(certificate.files[0].set, "verified");
        assert_eq!(certificate.files[0].verified_keys, vec![vec!["x".to_owned(), "y".to_owned()]]);
        assert!(!set_manifest::adjacent_path(&parquet).exists());
        let mut duplicate = arguments;
        duplicate.push(parquet.to_string_lossy().into_owned());
        assert!(certify_existing(duplicate).is_err());
        assert!(certify_existing(vec![]).is_err());
        assert!(certify_existing(vec!["--output".into(), output.to_string_lossy().into_owned()]).is_err());
    }
}
