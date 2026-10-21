use doop_shared::harness::{Engine, Harness, Result};

pub const ENGINES: &[Engine] = &[
    #[cfg(feature = "mini")]
    Engine {
        name: "mini",
        evaluate: doop_mini::run,
        threads: || 1,
    },
    #[cfg(feature = "mini-fx")]
    Engine {
        name: "mini-fx",
        evaluate: doop_mini_fx::run,
        threads: || 1,
    },
    #[cfg(feature = "ascent")]
    Engine {
        name: "ascent",
        evaluate: doop_ascent::run,
        threads: || 1,
    },
    #[cfg(feature = "mini-par")]
    Engine {
        name: "mini-par",
        evaluate: doop_mini_par::run,
        threads: rayon::current_num_threads,
    },
    #[cfg(feature = "mini-par-fx")]
    Engine {
        name: "mini-par-fx",
        evaluate: doop_mini_par_fx::run,
        threads: rayon::current_num_threads,
    },
    #[cfg(feature = "ascent-par")]
    Engine {
        name: "ascent-par",
        evaluate: doop_ascent_par::run,
        threads: rayon::current_num_threads,
    },
];

pub fn main(args: impl IntoIterator<Item = String>) -> Result<()> {
    let harness = Harness {
        engines: ENGINES,
        defaults: &[
            #[cfg(feature = "mini")]
            "mini",
            #[cfg(feature = "ascent")]
            "ascent",
        ],
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
        || (options.engine == "both" && harness.defaults.is_empty())
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
