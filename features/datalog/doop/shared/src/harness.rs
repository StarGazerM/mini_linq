//! Common CLI, full-answer comparison, timing and resident-memory reporting.
//! Engine names, evaluators, thread-count reporting and execution context belong
//! to callers. Loading remains outside compute timing and happens per engine.
use crate::{Facts, Results};
use std::{
    error::Error,
    path::{Path, PathBuf},
    time::Instant,
};

pub type Result<T, E = Box<dyn Error>> = std::result::Result<T, E>;

pub struct Engine {
    pub name: &'static str,
    pub evaluate: fn(Facts) -> Results,
    pub threads: fn() -> usize,
}

/// Engine lists define the default run and the full-set comparison obligations.
/// The special CLI selection `both` executes `defaults` in the recorded order.
pub struct Harness<'a> {
    pub engines: &'a [Engine],
    pub defaults: &'a [&'a str],
    pub reference: &'a str,
    pub checked: &'a [&'a str],
    pub options: &'a [&'a str],
    pub usage: &'a str,
}

#[derive(Debug)]
pub struct Options {
    pub directory: PathBuf,
    pub engine: String,
    pub check: bool,
    pub threads: Option<usize>,
}

impl Harness<'_> {
    fn engine(&self, name: &str) -> Result<&Engine> {
        self.engines
            .iter()
            .find(|engine| engine.name == name)
            .ok_or_else(|| {
                format!("engine unavailable: {name}; enable Cargo feature `{name}`").into()
            })
    }

    pub fn parse(&self, args: impl IntoIterator<Item = String>) -> Result<Options> {
        let mut args = args.into_iter();
        let directory = PathBuf::from(args.next().ok_or(self.usage)?);
        let mut options = Options {
            directory,
            engine: "both".into(),
            check: false,
            threads: None,
        };
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--engine" => options.engine = args.next().ok_or("--engine requires a name")?,
                "--check" => options.check = true,
                "--threads" if self.options.contains(&"--threads") => {
                    let count = args
                        .next()
                        .ok_or("--threads requires a count")?
                        .parse::<usize>()?;
                    if count == 0 {
                        return Err("--threads must be positive".into());
                    }
                    options.threads = Some(count);
                }
                _ => return Err(format!("unknown argument: {arg}").into()),
            }
        }
        if options.engine != "both" {
            self.engine(&options.engine)?;
        }
        if options.check {
            for name in std::iter::once(&self.reference).chain(self.checked) {
                self.engine(name)?;
            }
        }
        if !options.directory.is_dir() {
            return Err(format!(
                "facts directory does not exist: {}",
                options.directory.display()
            )
            .into());
        }
        Ok(options)
    }

    /// Execute within the context installed by the caller. String errors can
    /// cross an execution-context boundary without transferring boxed errors.
    pub fn execute(&self, options: &Options) -> std::result::Result<(), String> {
        let run = |name: &str| -> Result<Results> { run(self.engine(name)?, &options.directory) };
        if options.check {
            let mut expected = run(self.reference).map_err(|error| error.to_string())?;
            for &name in self.checked {
                let mut actual = run(name).map_err(|error| error.to_string())?;
                actual.compare(&mut expected)?;
                let engine = self.engine(name).map_err(|error| error.to_string())?;
                println!(
                    "check=ok engine={name} threads={} relations=26 comparison=full_tuple_sets",
                    (engine.threads)()
                );
            }
        } else if options.engine == "both" {
            if self.defaults.is_empty() {
                let engine = self.engines.first().ok_or("no engine features enabled")?;
                drop(run(engine.name).map_err(|error| error.to_string())?);
            }
            for &name in self.defaults {
                drop(run(name).map_err(|error| error.to_string())?);
            }
        } else {
            drop(run(&options.engine).map_err(|error| error.to_string())?);
        }
        Ok(())
    }
}

/// Process high-water RSS in bytes, or no measurement on unsupported systems.
pub fn peak_rss_bytes() -> Option<u64> {
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

pub fn run(engine: &Engine, directory: &Path) -> Result<Results> {
    let start = Instant::now();
    let facts = Facts::load(directory)?;
    let load = start.elapsed();
    let start = Instant::now();
    let result = (engine.evaluate)(facts);
    let compute = start.elapsed();
    println!(
        "engine={} threads={} load_seconds={:.6} compute_seconds={:.6} peak_rss_bytes={}",
        engine.name,
        (engine.threads)(),
        load.as_secs_f64(),
        compute.as_secs_f64(),
        peak_rss_bytes().map_or_else(|| "unavailable".into(), |rss| rss.to_string())
    );
    result.print_sizes();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUSTOM: &[Engine] = &[
        Engine {
            name: "custom",
            evaluate: |_| panic!("parser test must not evaluate an engine"),
            threads: || 7,
        },
        Engine {
            name: "reference",
            evaluate: |_| panic!("parser test must not evaluate an engine"),
            threads: || 1,
        },
    ];

    fn harness<'a>(options: &'a [&'static str]) -> Harness<'a> {
        Harness {
            engines: CUSTOM,
            defaults: &["custom"],
            reference: "reference",
            checked: &["custom"],
            options,
            usage: "usage: custom FACTS",
        }
    }

    fn args(options: &[&str]) -> Vec<String> {
        std::iter::once(env!("CARGO_MANIFEST_DIR"))
            .chain(options.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn caller_descriptors_define_engine_names_and_options() {
        let harness = harness(&["--threads"]);
        let options = harness
            .parse(args(&["--engine", "custom", "--check", "--threads", "7"]))
            .unwrap();
        assert_eq!(options.engine, "custom");
        assert!(options.check);
        assert_eq!(options.threads, Some(7));
        assert_eq!((harness.engine("custom").unwrap().threads)(), 7);
        assert!(harness.parse(args(&["--engine", "mini"])).is_err());
        assert_eq!(harness.parse(args(&[])).unwrap().engine, "both");
    }

    #[test]
    fn check_requires_every_comparison_engine_before_loading() {
        let mut harness = harness(&[]);
        harness.checked = &["custom", "missing"];
        assert!(
            harness
                .parse(args(&["--check"]))
                .unwrap_err()
                .to_string()
                .contains("feature `missing`")
        );
        harness.checked = &["custom"];
        harness.reference = "missing";
        assert!(
            harness
                .parse(args(&["--check"]))
                .unwrap_err()
                .to_string()
                .contains("feature `missing`")
        );
        assert!(harness.parse(args(&["--engine", "custom"])).is_ok());
    }

    #[test]
    fn malformed_and_unavailable_options_are_rejected() {
        let harness = harness(&["--threads"]);
        for words in [
            &["--engine"][..],
            &["--threads"],
            &["--threads", "0"],
            &["--threads", "bad"],
            &["--unknown"],
        ] {
            assert!(harness.parse(args(words)).is_err());
        }
        assert!(harness.parse(Vec::new()).is_err());
        assert!(self::harness(&[]).parse(args(&["--threads", "1"])).is_err());
    }
}
