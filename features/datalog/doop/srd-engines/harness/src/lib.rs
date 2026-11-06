use srd_doop_shared::{SrdFacts, SrdResults};
use std::{error::Error, path::PathBuf, time::Instant};

pub type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

pub struct Options {
    pub directory: PathBuf,
    pub export: Option<PathBuf>,
    pub threads: Option<usize>,
    #[cfg(feature = "planner-calibration")]
    pub calibration: Option<CalibrationOptions>,
}

#[cfg(feature = "planner-calibration")]
pub struct CalibrationOptions {
    pub plans: PathBuf,
    pub evidence: PathBuf,
    pub request: PathBuf,
    pub output: PathBuf,
    pub run_id: String,
}

impl Options {
    pub fn parse(parallel: bool, gpu: bool) -> Result<Self> {
        let mut args = std::env::args_os().skip(1);
        let directory = args
            .next()
            .ok_or("usage: <prepared-directory> [--export directory] [--threads N | --gpu-only] [--calibrate PLANS EVIDENCE REQUEST OUTPUT RUN-ID (feature planner-calibration)]")?;
        let mut options = Self {
            directory: directory.into(),
            export: None,
            threads: None,
            #[cfg(feature = "planner-calibration")]
            calibration: None,
        };
        while let Some(arg) = args.next() {
            if arg == "--export" {
                options.export = Some(args.next().ok_or("--export requires a directory")?.into());
            } else if arg == "--threads" && parallel {
                let value = args.next().ok_or("--threads requires a count")?;
                let count = value
                    .to_str()
                    .ok_or("invalid thread count")?
                    .parse::<usize>()?;
                if count == 0 {
                    return Err("--threads must be positive".into());
                }
                options.threads = Some(count);
            } else if arg == "--gpu-only" && gpu {
                // This standalone executable never runs a CPU reference.
            } else if arg == "--calibrate" {
                if gpu { return Err("planner calibration requires host operator counters".into()); }
                #[cfg(feature = "planner-calibration")]
                {
                    if options.calibration.is_some() { return Err("repeated --calibrate".into()); }
                    options.calibration = Some(CalibrationOptions {
                        plans: args.next().ok_or("--calibrate requires PLANS EVIDENCE REQUEST OUTPUT RUN-ID")?.into(),
                        evidence: args.next().ok_or("--calibrate requires EVIDENCE REQUEST OUTPUT RUN-ID")?.into(),
                        request: args.next().ok_or("--calibrate requires REQUEST OUTPUT RUN-ID")?.into(),
                        output: args.next().ok_or("--calibrate requires OUTPUT RUN-ID")?.into(),
                        run_id: args.next().ok_or("--calibrate requires RUN-ID")?.into_string().map_err(|_| "run identity must be UTF-8")?,
                    });
                }
                #[cfg(not(feature = "planner-calibration"))]
                { return Err("--calibrate requires feature planner-calibration".into()); }
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
    #[cfg(feature = "planner-calibration")]
    if options.calibration.is_some() {
        return calibrate(options, engine, threads, run);
    }
    eprintln!("phase\tload\tstart");
    let start = Instant::now();
    let facts = SrdFacts::load(&options.directory)?;
    let load = start.elapsed();
    eprintln!("phase\tload\tdone\t{:.6}", load.as_secs_f64());
    eprintln!("phase\tevaluation\tstart");
    let start = Instant::now();
    let mut results = run(facts);
    let evaluation = start.elapsed();
    eprintln!("phase\tevaluation\tdone\t{:.6}", evaluation.as_secs_f64());
    println!("counts\t{}", serde_json::to_string(&results.counts())?);
    println!(
        "runtime\t{}",
        serde_json::json!({
            "engine": engine,
            "dataset": options.directory,
            "threads": threads,
            "load_seconds": load.as_secs_f64(),
            "evaluation_seconds": evaluation.as_secs_f64(),
        })
    );
    if let Some(directory) = options.export {
        eprintln!("phase\texport\tstart");
        let start = Instant::now();
        results.export(&directory)?;
        eprintln!("phase\texport\tdone\t{:.6}", start.elapsed().as_secs_f64());
    } else if let Some(path) = std::env::var_os("BENCH_CHECK_WITNESS") {
        let start = Instant::now();
        let witness = results.witness(bench_check::Policy::from_env(0)?);
        bench_check::write_witness(&PathBuf::from(path), &witness)?;
        eprintln!("phase\tbench_check_witness\tdone\t{:.6}", start.elapsed().as_secs_f64());
    }
    Ok(())
}

/// Count-only route: no benchmark clock, no automatic advice adoption, and one
/// ordinary load followed by one complete invocation of the selected engine.
#[cfg(feature = "planner-calibration")]
fn calibrate(options: Options, engine: &str, threads: usize, run: impl FnOnce(SrdFacts) -> SrdResults) -> Result<()> {
    let config = options.calibration.as_ref().expect("selected calibration route");
    let plans = cq_cost::export::parse_json(&std::fs::read_to_string(&config.plans)?)?;
    let evidence = cq_syntax::evidence::parse_json(&std::fs::read_to_string(&config.evidence)?)?;
    let request = cq_load::stats_manifest::DatasetRequest::parse_json(&std::fs::read_to_string(&config.request)?)?;
    let expected = match engine {
        "mini" => cq_cost::export::Target::Cpu,
        "mini-par" => cq_cost::export::Target::CpuPar,
        _ => return Err("this engine has no host calibration contract".into()),
    };
    if plans.configuration.target != expected { return Err("calibration plans target a different engine".into()); }
    let mut measured = cq_cost::calibration::try_measure(&plans, &evidence, &request, &config.run_id, || {
        let facts = SrdFacts::load(&options.directory).map_err(|error| cq_cost::export::ExportError(error.to_string()))?;
        Ok(run(facts))
    })?;
    println!("calibration_context engine={engine:?} threads={threads} [source=declared; provenance=standalone CLI/Rayon pool; dataset={}]", measured.evidence.dataset.hash);
    println!("{}", measured.report.render(&measured.evidence));
    measured.write_artifacts(&config.output)?;
    if let Some(directory) = options.export {
        measured.value.export(&directory)?;
    } else if let Some(path) = std::env::var_os("BENCH_CHECK_WITNESS") {
        let witness = measured.value.witness(bench_check::Policy::from_env(0)?);
        bench_check::write_witness(&PathBuf::from(path), &witness)?;
    }
    if measured.report.physical_steps.iter().any(|step|
        step.planned_comparison.as_ref().is_some_and(|bound| !bound.within_bound)
        || step.measured.comparison.as_ref().is_some_and(|bound| !bound.within_bound))
    {
        return Err("calibration found exceeded plan bounds; diagnostics/evidence were emitted, and no advice was adopted".into());
    }
    Ok(())
}
