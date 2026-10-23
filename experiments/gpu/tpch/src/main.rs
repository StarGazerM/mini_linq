use std::path::PathBuf;

fn main() -> cq_gpu_tpch::manifest::Result<()> {
    let mut arguments = std::env::args_os().skip(1);
    let mut output = None;
    let mut queries = Vec::new();
    let mut scales = Vec::new();
    let mut modes = Vec::new();
    let mut authored = None;
    let mut baseline = None;
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--mode") => modes.push(arguments.next()
                .ok_or("--mode requires automatic or advised")?.into_string()
                .map_err(|_| "mode must be UTF-8")?),
            Some("--authored") => authored = Some(PathBuf::from(arguments.next()
                .ok_or("--authored requires an H05 compiled results JSON path")?)),
            Some("--baseline") => baseline = Some(PathBuf::from(arguments.next()
                .ok_or("--baseline requires a GPU suite results JSON path")?)),
            Some("--query") => queries.push(arguments.next()
                .ok_or("--query requires a name")?.into_string()
                .map_err(|_| "query name must be UTF-8")?),
            Some("--scale") => {
                let scale = arguments.next().ok_or("--scale requires sf1 or sf10")?;
                scales.push(match scale.to_str() {
                    Some("sf1") => 1,
                    Some("sf10") => 10,
                    _ => return Err("--scale requires sf1 or sf10".into()),
                });
            }
            Some(value) if value.starts_with('-') => {
                return Err(format!("unknown option: {value}").into());
            }
            _ if output.is_none() => output = Some(PathBuf::from(argument)),
            _ => return Err("usage: cq-gpu-tpch [results.json] [--query NAME]... [--scale sf1|sf10]... [--mode automatic|advised]... [--authored H05_RESULTS.json] [--baseline SUITE_RESULTS.json]".into()),
        }
    }
    let output = output.unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../compare/gpu_tpch.results.json")
    });
    if !cq_gpu_tpch::run(
        &output,
        &queries,
        &scales,
        &modes,
        authored.as_deref(),
        baseline.as_deref(),
    )? {
        return Err(
            "GPU execution/conformance or H05 performance acceptance failed; see results".into(),
        );
    }
    Ok(())
}
