use datalog_doop::{Facts, Results, ascent_engine, mini_engine};
use std::{env, error::Error, path::Path, time::Instant};

fn peak_rss_bytes() -> Option<u64> {
    #[cfg(unix)]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: getrusage initializes this correctly aligned writable rusage on
        // success; the value is read only after checking its return status.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
            return None;
        }
        let rss = unsafe { usage.assume_init() }.ru_maxrss;
        let bytes = u64::try_from(rss).ok()?;
        Some(if cfg!(target_os = "macos") {
            bytes
        } else {
            bytes * 1024
        })
    }
    #[cfg(not(unix))]
    None
}

fn run(engine: &str, directory: &Path) -> Result<Results, Box<dyn Error>> {
    let start = Instant::now();
    let facts = Facts::load(directory)?;
    let load = start.elapsed();
    let start = Instant::now();
    let result = match engine {
        "mini" => mini_engine::run(facts),
        "ascent" => ascent_engine::run(facts),
        _ => unreachable!(),
    };
    let compute = start.elapsed();
    println!(
        "engine={engine} load_seconds={:.6} compute_seconds={:.6} peak_rss_bytes={}",
        load.as_secs_f64(),
        compute.as_secs_f64(),
        peak_rss_bytes().map_or_else(|| "unavailable".into(), |rss| rss.to_string())
    );
    result.print_sizes();
    Ok(result)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args().skip(1);
    let directory = args
        .next()
        .ok_or("usage: doop <facts_dir> [--engine mini|ascent|both] [--check]")?;
    let mut engine = String::from("both");
    let mut check = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--engine" => {
                engine = args
                    .next()
                    .ok_or("--engine requires mini, ascent, or both")?
            }
            "--check" => check = true,
            _ => return Err(format!("unknown argument: {arg}").into()),
        }
    }
    if !matches!(engine.as_str(), "mini" | "ascent" | "both") {
        return Err(format!("unknown engine: {engine}").into());
    }
    let directory = Path::new(&directory);
    if !directory.is_dir() {
        return Err(format!("facts directory does not exist: {}", directory.display()).into());
    }
    if check {
        let mut mini = run("mini", directory)?;
        let mut ascent = run("ascent", directory)?;
        mini.compare(&mut ascent)?;
        println!("check=ok relations=26 comparison=full_tuple_sets");
    } else {
        if engine == "mini" || engine == "both" {
            drop(run("mini", directory)?);
        }
        if engine == "ascent" || engine == "both" {
            drop(run("ascent", directory)?);
        }
    }
    Ok(())
}
