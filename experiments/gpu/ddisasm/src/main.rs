use datalog_ddisasm::{loader, mini_engine};
use std::{path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let directory = PathBuf::from(args.next().ok_or("usage: ddisasm-gpu DATA [--meta JSON] [--gpu-only]")?);
    let mut meta = None;
    let mut gpu_only = false;
    while let Some(arg) = args.next() {
        if arg == "--gpu-only" {
            gpu_only = true;
        } else if arg == "--meta" {
            meta = Some(PathBuf::from(args.next().ok_or("missing --meta JSON")?));
        } else {
            return Err(format!("unknown argument {arg:?}").into());
        }
    }
    let facts = match meta {
        Some(path) => loader::load_with_meta(&directory, &path)?,
        None => loader::load(&directory)?,
    };
    let reference = (!gpu_only).then(|| {
        let input = facts.clone();
        let start = Instant::now();
        let expected = mini_engine::run(input);
        (expected, start.elapsed())
    });
    let _pool = gpu_datalog_runtime::thread_pool()?;
    let start = Instant::now();
    let mut actual = ddisasm_gpu::run(facts)?;
    let elapsed = start.elapsed();
    println!("dataset\t{}", directory.display());
    if let Some((mut expected, cpu_elapsed)) = reference {
        expected.check(&mut actual);
        println!("comparison\texact full-tuple sets, all 11 outputs");
        println!("mini_evaluation_seconds\t{:.6}", cpu_elapsed.as_secs_f64());
    } else {
        println!("comparison\tskipped (--gpu-only)");
    }
    println!("gpu_evaluation_seconds\t{:.6}", elapsed.as_secs_f64());
    for (name, count) in actual.counts() {
        println!("{name}\t{count}");
    }
    Ok(())
}
