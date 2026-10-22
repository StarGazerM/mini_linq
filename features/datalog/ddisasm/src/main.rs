use datalog_ddisasm::{ascent_engine, loader, mini_engine};
use std::{path::PathBuf, time::Instant};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(
        args.next()
            .ok_or("usage: ddisasm DATA --engine mini|ascent|both [--check] [--meta JSON]")?,
    );
    let mut engine = "both".to_owned();
    let mut check = false;
    let mut metadata = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--engine" => engine = args.next().ok_or("missing --engine value")?,
            "--check" => check = true,
            "--meta" => metadata = Some(PathBuf::from(args.next().ok_or("missing --meta value")?)),
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    if !["mini", "ascent", "both"].contains(&engine.as_str()) {
        return Err("invalid engine".into());
    }
    let start = Instant::now();
    let facts = match metadata {
        Some(metadata) => loader::load_with_meta(&path, &metadata)?,
        None => loader::load(&path)?,
    };
    let load_seconds = start.elapsed().as_secs_f64();
    let mut report = serde_json::json!({"dataset":path,"load_seconds":load_seconds,"compute_boundary":"engine run including indexes and output collection; excludes parsing, input clone and exact comparison"});
    let mut mini = None;
    let mut ascent = None;
    if check || engine == "mini" || engine == "both" {
        let input = facts.clone();
        let start = Instant::now();
        let result = mini_engine::run(input);
        report["mini"] =
            serde_json::json!({"seconds":start.elapsed().as_secs_f64(),"counts":result.counts()});
        mini = Some(result);
        eprintln!("mini complete: {}", report["mini"]);
    }
    if check || engine == "ascent" || engine == "both" {
        let start = Instant::now();
        let result = ascent_engine::run(facts);
        report["ascent"] =
            serde_json::json!({"seconds":start.elapsed().as_secs_f64(),"counts":result.counts()});
        ascent = Some(result);
        eprintln!("ascent complete: {}", report["ascent"]);
    }
    if check {
        mini.as_mut().unwrap().check(ascent.as_mut().unwrap());
        report["full_tuple_check"] =
            serde_json::json!("passed: all 11 outputs sorted and deduplicated");
    }
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
