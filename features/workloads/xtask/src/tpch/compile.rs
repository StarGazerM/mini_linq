use crate::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(super) struct Built {
    pub binary: PathBuf,
    pub unit_ms: f64,
    pub timing_report: PathBuf,
}

pub(super) fn build(root: &Path, source: &Path, name: &str) -> Result<Built> {
    let cache = root.join("target/bench");
    let directory = cache.join("crates").join(name);
    std::fs::create_dir_all(directory.join("src"))?;
    let package = format!("tpch-derived-{name}");
    let mut manifest = format!(
        "[package]\nname = {package:?}\nversion = \"0.0.0\"\nedition = \"2024\"\nrust-version = \"1.85\"\n[workspace]\n[dependencies]\n"
    );
    for (dependency, relative, features) in [
        ("mini-linq", "facade/mini-linq", ""),
        ("cq-load", "runtime/cq-load", ", features = [\"parquet\"]"),
        (
            "cq-storage",
            "runtime/cq-storage",
            ", features = [\"counters\"]",
        ),
        ("cq-state-providers", "runtime/cq-state-providers", ""),
        ("cq-ext-aggregate", "extensions/cq-ext-aggregate", ""),
        ("cq-ext-correlated", "extensions/cq-ext-correlated", ""),
        ("cq-ext-negation", "extensions/cq-ext-negation", ""),
        ("cq-ext-union", "extensions/cq-ext-union", ""),
    ] {
        manifest.push_str(&format!(
            "{dependency} = {{ path = {:?}{features} }}\n",
            root.join(relative)
        ));
    }
    let oracle_lock = std::fs::read_to_string(root.join("xtask/Cargo.lock"))?;
    let serde_version = oracle_lock
        .split("name = \"serde_json\"\nversion = \"")
        .nth(1)
        .and_then(|v| v.split('"').next())
        .ok_or("serde_json absent from oracle lock")?;
    manifest.push_str(&format!("serde_json = \"={serde_version}\"\n"));
    std::fs::write(directory.join("Cargo.toml"), manifest)?;
    // Preserve the repository's dependency versions. Cargo may only adapt the
    // root-package entries here; the actual measured build always uses --locked.
    std::fs::copy(root.join("Cargo.lock"), directory.join("Cargo.lock"))?;
    let source_text = format!(
        "// Measurement build {:?}\n#[allow(unused_imports, unused_macros, dead_code)] #[path = {:?}] mod tpch_runtime;\n#[path = {source:?}] mod query;\n#[path = {:?}] mod comparison;\ninclude!({:?});\n",
        std::time::SystemTime::now(),
        root.join("workloads/src/tpch_runtime.rs"),
        root.join("xtask/src/tpch/comparison.rs"),
        root.join("xtask/src/tpch/worker.rs"),
    );
    std::fs::write(directory.join("src/main.rs"), source_text)?;
    let target = cache.join("build");
    let mut lock = metadata(&directory, &target, true)?;
    if !lock.status.success() {
        // The oracle does not itself need Rust's Parquet reader. On a fresh
        // checkout its sources may still need downloading; retain the fast
        // offline path for warm caches without making a prefetch mandatory.
        eprintln!("fetching missing benchmark dependencies for {name}...");
        lock = metadata(&directory, &target, false)?;
    }
    if !lock.status.success() {
        return Err(format!(
            "per-query dependency resolution: {}",
            String::from_utf8_lossy(&lock.stderr)
        )
        .into());
    }
    let status = crate::cargo::command()
        .current_dir(&directory)
        .args(["build", "--release", "--locked", "--offline", "--timings"])
        .env("CARGO_TARGET_DIR", &target)
        .status()?;
    if !status.success() {
        return Err(format!("release compilation failed for {name}: {status}").into());
    }
    let html = std::fs::read_to_string(target.join("cargo-timings/cargo-timing.html"))?;
    let unit_ms = unit_seconds(&html, &package)? * 1000.0;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let timing_report = cache.join("compile").join(format!("{name}-{stamp}.html"));
    std::fs::create_dir_all(timing_report.parent().unwrap())?;
    std::fs::write(&timing_report, html)?;
    Ok(Built {
        binary: target.join("release").join(package),
        unit_ms,
        timing_report,
    })
}

fn metadata(directory: &Path, target: &Path, offline: bool) -> Result<std::process::Output> {
    let mut command = crate::cargo::command();
    command
        .current_dir(directory)
        .args(["metadata", "--format-version", "1"])
        .env("CARGO_TARGET_DIR", target);
    if offline {
        command.arg("--offline");
    }
    Ok(command.output()?)
}

fn unit_seconds(html: &str, package: &str) -> Result<f64> {
    let start = html
        .find("const UNIT_DATA = ")
        .ok_or("Cargo timing report has no UNIT_DATA")?
        + "const UNIT_DATA = ".len();
    let tail = &html[start..];
    let end = tail
        .find(";\n")
        .ok_or("Cargo timing report has unterminated UNIT_DATA")?;
    let units: Value = serde_json::from_str(&tail[..end])?;
    units
        .as_array()
        .ok_or("Cargo timing units are not an array")?
        .iter()
        .find(|unit| unit["name"].as_str() == Some(package))
        .and_then(|unit| unit["duration"].as_f64())
        .ok_or_else(|| format!("Cargo timing report has no fresh unit for {package}").into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_only_the_query_unit_not_dependencies() {
        let html = "const UNIT_DATA = [{\"name\":\"dependency\",\"duration\":99.0},{\"name\":\"query\",\"duration\":1.25}];\n";
        assert_eq!(unit_seconds(html, "query").unwrap(), 1.25);
        assert!(unit_seconds(html, "absent").is_err());
    }
}
