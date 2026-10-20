//! Engine descriptors and local pool setup; all DOOP harness operations use
//! the base library, including CLI parsing, loading, comparison, timing and RSS.
use crate::{ascent_engine, mini_engine};
use datalog_doop::harness::{Engine, Harness, Result};

pub const ENGINES: &[Engine] = &[
    Engine {
        name: "mini",
        evaluate: mini_engine::run,
        threads: || 1,
    },
    Engine {
        name: "mini-par",
        evaluate: mini_engine::run_parallel,
        threads: rayon::current_num_threads,
    },
    Engine {
        name: "mini-fx",
        evaluate: mini_engine::run_fx,
        threads: || 1,
    },
    Engine {
        name: "mini-par-fx",
        evaluate: mini_engine::run_parallel_fx,
        threads: rayon::current_num_threads,
    },
    Engine {
        name: "ascent",
        evaluate: ascent_engine::run,
        threads: || 1,
    },
    Engine {
        name: "ascent-par",
        evaluate: ascent_engine::run_parallel,
        threads: rayon::current_num_threads,
    },
];

pub fn main(args: impl IntoIterator<Item = String>) -> Result<()> {
    let harness = Harness {
        engines: ENGINES,
        defaults: &["mini", "ascent"],
        reference: "ascent",
        checked: &["mini", "mini-fx", "mini-par", "mini-par-fx", "ascent-par"],
        options: &["--threads"],
        usage: "usage: doop-parallel <facts_dir> [--engine mini|mini-fx|mini-par|mini-par-fx|ascent|ascent-par|both] [--threads N] [--check]",
    };
    let options = harness.parse(args)?;
    let execute = || harness.execute(&options);
    // Baselines stay on the caller thread: dormant worker stacks must not
    // contaminate their peak RSS, even if --threads was supplied.
    let result = if options.check
        || matches!(
            options.engine.as_str(),
            "mini-par" | "mini-par-fx" | "ascent-par"
        ) {
        let mut builder = rayon::ThreadPoolBuilder::new();
        if let Some(count) = options.threads {
            builder = builder.num_threads(count);
        }
        builder.build()?.install(execute)
    } else {
        execute()
    };
    result.map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptors_report_their_own_execution_width() {
        rayon::ThreadPoolBuilder::new()
            .num_threads(3)
            .build()
            .unwrap()
            .install(|| {
                for engine in ENGINES {
                    let expected =
                        if matches!(engine.name, "mini-par" | "mini-par-fx" | "ascent-par") {
                            3
                        } else {
                            1
                        };
                    assert_eq!((engine.threads)(), expected, "{}", engine.name);
                }
            });
    }
}
