use srd_doop_shared::{SrdFacts, SrdResults};
use std::{error::Error, path::PathBuf, time::Instant};

pub type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

pub struct Options {
    pub directory: PathBuf,
    pub export: Option<PathBuf>,
    pub threads: Option<usize>,
}

impl Options {
    pub fn parse(parallel: bool, gpu: bool) -> Result<Self> {
        let mut args = std::env::args_os().skip(1);
        let directory = args.next().ok_or("usage: <prepared-directory> [--export directory] [--threads N | --gpu-only]")?;
        let mut options = Self {
            directory: directory.into(),
            export: None,
            threads: None,
        };
        while let Some(arg) = args.next() {
            if arg == "--export" {
                options.export = Some(args.next().ok_or("--export requires a directory")?.into());
            } else if arg == "--threads" && parallel {
                let value = args.next().ok_or("--threads requires a count")?;
                let count = value.to_str().ok_or("invalid thread count")?.parse::<usize>()?;
                if count == 0 {
                    return Err("--threads must be positive".into());
                }
                options.threads = Some(count);
            } else if arg == "--gpu-only" && gpu {
                // This standalone executable never runs a CPU reference.
            } else {
                return Err(format!("unknown argument: {}", arg.to_string_lossy()).into());
            }
        }
        Ok(options)
    }

    pub fn parallel_threads(&self) -> Result<usize> {
        if let Some(count) = self.threads {
            return Ok(count);
        }
        match std::env::var("RAYON_NUM_THREADS") {
            Ok(value) => {
                let count = value.parse::<usize>()?;
                if count == 0 {
                    return Err("RAYON_NUM_THREADS must be positive".into());
                }
                Ok(count)
            }
            Err(std::env::VarError::NotPresent) => Ok(15),
            Err(error) => Err(error.into()),
        }
    }
}

/// Loading and exports are outside evaluation; engine output materialization is inside.
pub fn execute(
    options: Options,
    engine: &str,
    threads: usize,
    run: impl FnOnce(SrdFacts) -> SrdResults,
) -> Result<()> {
    eprintln!("phase\tload\tstart");
    let start = Instant::now();
    let facts = SrdFacts::load(&options.directory)?;
    let load = start.elapsed();
    eprintln!("phase\tload\tdone\t{:.6}", load.as_secs_f64());
    eprintln!("phase\tevaluation\tstart");
    let start = Instant::now();
    let results = run(facts);
    let evaluation = start.elapsed();
    eprintln!("phase\tevaluation\tdone\t{:.6}", evaluation.as_secs_f64());
    println!("counts\t{}", serde_json::to_string(&results.counts())?);
    println!("runtime\t{}", serde_json::json!({
        "engine": engine,
        "dataset": options.directory,
        "threads": threads,
        "load_seconds": load.as_secs_f64(),
        "evaluation_seconds": evaluation.as_secs_f64(),
    }));
    if let Some(directory) = options.export {
        eprintln!("phase\texport\tstart");
        let start = Instant::now();
        results.export(&directory)?;
        eprintln!("phase\texport\tdone\t{:.6}", start.elapsed().as_secs_f64());
    }
    Ok(())
}
