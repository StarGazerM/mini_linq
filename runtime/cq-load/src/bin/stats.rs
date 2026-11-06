//! Explicit offline statistics collection; no workload or planner execution.
use cq_load::stats_manifest::DatasetRequest;
use cq_syntax::evidence;
use std::{collections::BTreeMap, io::Write, path::{Path, PathBuf}};

type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;
const USAGE: &str = "cq-stats stats --request schema.json --dataset DATASET_DIR --out evidence.json [--expect-dataset-hash sha256:HEX]\ncq-stats check --evidence evidence.json\n\nOne decoded pass per relation, using cq-load columnar sources. Data identity is\ntyped-row-stream-v1, including source duplicates/order before set deduplication.\nNo sampling, workload execution, timing or plan selection.";

fn publish(path: &Path, bytes: &[u8]) -> Result<()> {
    let filename = path.file_name().ok_or("output must name a file")?.to_str().ok_or("output path must be UTF-8")?;
    let temporary = path.with_file_name(format!(".{filename}.cq-stats-{}.tmp", std::process::id()));
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let result = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() { let _ = std::fs::remove_file(&temporary); }
    result?;
    Ok(())
}
fn output_identity(path: &Path) -> Result<PathBuf> {
    if path.exists() { return Ok(path.canonicalize()?); }
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    Ok(parent.canonicalize()?.join(path.file_name().ok_or("output must name a file")?))
}
fn run() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let command = args.next().ok_or(USAGE)?;
    if command == "--help" || command == "-h" { println!("{USAGE}"); return Ok(()); }
    if command != "stats" && command != "check" { return Err(USAGE.into()); }
    let mut flags = BTreeMap::new();
    while let Some(flag) = args.next() {
        let allowed = if command == "check" { flag == "--evidence" }
            else { matches!(flag.as_str(), "--request" | "--dataset" | "--out" | "--expect-dataset-hash") };
        if !allowed { return Err(format!("unknown {command} option {flag}").into()); }
        let value = args.next().ok_or_else(|| format!("missing value for {flag}"))?;
        if flags.insert(flag.clone(), value).is_some() { return Err(format!("duplicate {flag}").into()); }
    }
    let required = |flag: &str| -> Result<&str> {
        flags.get(flag).map(String::as_str).ok_or_else(|| format!("missing {flag}\n{USAGE}").into())
    };
    if command == "check" {
        let document = evidence::parse_bytes(&std::fs::read(required("--evidence")?)?)?;
        println!("{} relations={}", document.dataset.hash, document.relations.len());
        return Ok(());
    }
    let request_path = Path::new(required("--request")?);
    let request = DatasetRequest::parse_json(&std::fs::read_to_string(request_path)?)?;
    let root = Path::new(required("--dataset")?);
    let output = Path::new(required("--out")?);
    let identity = output_identity(output)?;
    if identity == request_path.canonicalize()? { return Err("output would overwrite the statistics request".into()); }
    for path in request.relations.iter().map(|r| &r.path).chain(request.metadata.iter().map(|m| &m.path)) {
        let source = root.join(path);
        if source.exists() && source.canonicalize()? == identity { return Err("output would overwrite dataset input".into()); }
    }
    let document = request.collect(root)?;
    if let Some(expected) = flags.get("--expect-dataset-hash") {
        if expected != &document.dataset.hash { return Err("collected dataset hash differs from --expect-dataset-hash".into()); }
    }
    // All source traversal and validation completes before publication. A failed
    // input never overwrites a prior complete evidence file.
    publish(output, document.canonical_json()?.as_bytes())?;
    println!("{} relations={}", document.dataset.hash, document.relations.len());
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("cq-stats: {error}");
        std::process::exit(1);
    }
}
