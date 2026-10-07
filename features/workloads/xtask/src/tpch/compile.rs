use crate::Result;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(super) struct Built {
    pub binary: PathBuf,
    pub count_binary: PathBuf,
    pub unit_ms: f64,
    pub timing_report: PathBuf,
}

pub(super) fn build(root: &Path, source: &Path, name: &str) -> Result<Built> {
    let cache = root.join("target/bench");
    let directory = cache.join("crates").join(name);
    std::fs::create_dir_all(directory.join("src"))?;
    let package = format!("tpch-derived-{name}");
    let mut manifest = format!(
        "[package]\nname = {package:?}\nversion = \"0.0.0\"\nedition = \"2024\"\nrust-version = \"1.85.1\"\n[workspace]\n"
    );
    manifest.push_str("[features]\ncounters = [\"cq-storage/counters\"]\n[dependencies]\n");
    manifest.push_str(&crate::cargo::dependencies(&[
        "mini-linq",
        "cq-load",
        "cq-storage",
        "cq-planners",
        "cq-rewrites",
        "cq-ext-aggregate",
        "cq-ext-correlated",
        "cq-ext-negation",
    ])?);
    manifest.push_str("cq-load = { version = \"0.1.0\" }\n");
    // Add Parquet to the resolved public cq-load dependency, retaining its source.
    manifest = manifest
        .lines()
        .filter(|line| *line != "cq-load = { version = \"0.1.0\" }")
        .map(|line| {
            if line.starts_with("cq-load = ") {
                line.trim_end_matches('}').to_owned() + ", features = [\"parquet\"] }"
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    manifest.push_str("serde_json = \"1\"\n");
    std::fs::write(directory.join("Cargo.toml"), manifest)?;
    // Preserve the repository's dependency versions. Cargo may only adapt the
    // root-package entries here; the actual measured build always uses --locked.
    std::fs::copy(root.join("Cargo.lock"), directory.join("Cargo.lock"))?;
    let source_text = format!(
        "// Measurement build {:?}\n#[allow(unused_imports, unused_macros, dead_code)] #[path = {:?}] mod tpch_runtime;\n#[path = {source:?}] mod query;\n#[path = {:?}] mod comparison;\ninclude!({:?});\n",
        std::time::SystemTime::now(),
        root.join("src/tpch_runtime.rs"),
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
    let binary = directory.join("timing-worker");
    std::fs::copy(target.join("release").join(&package), &binary)?;
    let status = crate::cargo::command()
        .current_dir(&directory)
        .args([
            "build",
            "--release",
            "--locked",
            "--offline",
            "--features",
            "counters",
        ])
        .env("CARGO_TARGET_DIR", &target)
        .status()?;
    if !status.success() {
        return Err(format!("counting worker compilation failed for {name}: {status}").into());
    }
    let count_binary = directory.join("counting-worker");
    std::fs::copy(target.join("release").join(&package), &count_binary)?;
    Ok(Built {
        binary,
        count_binary,
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
