use std::path::PathBuf;

fn main() -> cq_gpu_tpch::manifest::Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let output = arguments.next().map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../compare/gpu_tpch.results.json")
    });
    if arguments.next().is_some() {
        return Err("usage: cq-gpu-tpch [results.json]".into());
    }
    if !cq_gpu_tpch::run_all(&output)? {
        return Err(format!("GPU execution/conformance failed; see {}", output.display()).into());
    }
    Ok(())
}
