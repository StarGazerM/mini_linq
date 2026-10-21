//! Sequential CLI dispatch over independently compiled engines.
use crate::{CASES, Result};
#[cfg(feature = "counters")]
use oracle_shared::harness::counters;
#[cfg(all(
    test,
    feature = "counters",
    feature = "mini",
    feature = "mini-keyed",
    feature = "ascent"
))]
use oracle_shared::harness::reach_cost;
use oracle_shared::harness::{Command, bench_inputs, command, median_us, require_bench};
use std::{hint::black_box, time::Duration};
#[cfg(feature = "counters")]
pub fn check_reach_cost(n: i32, cost: &cq_storage::counters::Snapshot) {
    assert_eq!(cost.index_builds, 0, "reach chain n={n}");
    // Raw EDB canonicalization visits n rows once; its borrowed adapter
    // adds no core cursor events. Grouped seed/delta/edge reads add 2*n+1.
    assert_eq!(cost.rows_enumerated, 3 * n as u64 + 1, "reach chain n={n}");
}

/// Three checked samples per hasher, interleaved AB/BA/AB on each fixture.
/// Run with `bench --engine both`; no counter or timing feature is enabled.
fn bench_hashers() {
    println!("program\tscenario\tnodes\tedges\trows\tahash_us\tfx_us\tfx_speedup");
    bench_inputs(|name, scenario, n, data| {
        let baseline = CASES.iter().find(|case| case.name == name).unwrap();
        let selected = crate::KEYED_CASES
            .iter()
            .find(|case| case.name == name)
            .unwrap();
        let expected = (baseline.oracle)(data).0;
        // Keep keyed aHash first and Fx second to match the reported columns.
        let evaluators = [selected.mini, baseline.mini];
        let mut samples = [[Duration::ZERO; 2]; 3];
        // Warm both implementations before comparing short microbenchmarks.
        for evaluate in evaluators {
            assert_eq!(evaluate(data).0, expected);
        }
        for (repetition, sample) in samples.iter_mut().enumerate() {
            let order = if repetition % 2 == 0 { [0, 1] } else { [1, 0] };
            for engine in order {
                let (actual, elapsed) = black_box(evaluators[engine](black_box(data)));
                assert_eq!(actual, expected, "{name} {scenario} n={n}");
                sample[engine] = elapsed;
            }
        }
        let mut ahash = samples.map(|sample| sample[0]);
        let mut fx = samples.map(|sample| sample[1]);
        ahash.sort_unstable();
        fx.sort_unstable();
        let ahash = ahash[1].as_secs_f64() * 1e6;
        let fx = fx[1].as_secs_f64() * 1e6;
        println!(
            "{name}\t{scenario}\t{n}\t{}\t{}\t{ahash:.3}\t{fx:.3}\t{:.3}",
            data.edge.len(),
            expected.len(),
            ahash / fx
        );
    });
}

pub fn run(args: &[String]) -> Result<()> {
    let (args, cases, engine) = bench_engine(args)?;
    match command(
        args,
        &["--write"],
        "usage: datalog-oracle golden [--write] | counters | bench [--engine mini|mini-keyed|both]",
    )? {
        Command::Golden { write } => {
            crate::goldens(write, false)?;
        }
        Command::Bench { .. } => {
            require_bench("datalog-oracle")?;
            if engine == "both" {
                bench_hashers();
                return Ok(());
            }
            let column = if engine == "mini-keyed" {
                "mini_keyed_us"
            } else {
                "mini_us"
            };
            println!("program\tscenario\tnodes\tedges\trows\t{column}\tascent_us");
            bench_inputs(|name, scenario, n, data| {
                let case = cases.iter().find(|case| case.name == name).unwrap();
                let expected = (case.oracle)(data).0;
                assert_eq!((case.mini)(data).0, expected);
                let label = format!("{name} {scenario} n={n}");
                let mini = median_us(case.mini, data, &expected, &label);
                let ascent = median_us(case.oracle, data, &expected, &label);
                println!(
                    "{name}\t{scenario}\t{n}\t{}\t{}\t{mini:.3}\t{ascent:.3}",
                    data.edge.len(),
                    expected.len()
                );
            });
        }
        Command::Counters => {
            #[cfg(feature = "counters")]
            counters(
                cases.iter().find(|case| case.name == "reach").unwrap().mini,
                check_reach_cost,
            );
            #[cfg(not(feature = "counters"))]
            return Err("counters requires --features counters; run cargo run --release -p datalog-oracle --features counters -- counters".into());
        }
    }
    Ok(())
}

fn bench_engine(args: &[String]) -> Result<(&[String], &'static [crate::Case], &'static str)> {
    if args.len() == 3 && args[0] == "bench" && args[1] == "--engine" {
        return match args[2].as_str() {
            "mini" if !CASES.is_empty() => Ok((&args[..1], CASES, "mini")),
            "mini-keyed" if !crate::KEYED_CASES.is_empty() => {
                Ok((&args[..1], crate::KEYED_CASES, "mini-keyed"))
            }
            "both" if !CASES.is_empty() && !crate::KEYED_CASES.is_empty() => {
                Ok((&args[..1], CASES, "both"))
            }
            name => Err(format!("unknown or disabled engine: {name}").into()),
        };
    }
    Ok((
        args,
        crate::selected_cases()?,
        if CASES.is_empty() {
            "mini-keyed"
        } else {
            "mini"
        },
    ))
}

#[cfg(all(test, feature = "mini", feature = "mini-keyed", feature = "ascent"))]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).into()).collect()
    }

    #[test]
    fn keyed_bench_selection_is_explicit() {
        for name in ["mini", "mini-keyed", "both"] {
            let words = args(&["bench", "--engine", name]);
            let (remaining, cases, selected) = bench_engine(&words).unwrap();
            assert_eq!(remaining, &words[..1]);
            assert_eq!(selected, name);
            assert_eq!(cases.len(), CASES.len());
        }
        assert_eq!(bench_engine(&args(&["bench"])).unwrap().2, "mini");
        assert!(bench_engine(&args(&["bench", "--engine", "unknown"])).is_err());
        assert!(bench_engine(&args(&["bench", "--engine", "mini-fx"])).is_err());
    }

    #[cfg(feature = "counters")]
    #[test]
    fn counter_measurement_invokes_supplied_evaluator_and_obligation() {
        use std::cell::Cell;
        fn evaluate(data: &crate::Data) -> (crate::Rows, Duration) {
            assert_eq!(data.nodes.len(), 8);
            (crate::selected_cases()
                .unwrap()
                .iter()
                .find(|case| case.name == "reach")
                .unwrap()
                .mini)(data)
        }
        let checked = Cell::new(false);
        let (rows, _) = reach_cost(8, evaluate, |n, cost| {
            check_reach_cost(n, cost);
            checked.set(true);
        });
        assert_eq!(rows, 8);
        assert!(checked.get());
    }
}

#[cfg(test)]
mod feature_tests {
    use super::*;

    #[test]
    fn engine_selection_matches_compiled_dependencies() {
        for (name, enabled) in [
            ("mini", !CASES.is_empty()),
            ("mini-keyed", !crate::KEYED_CASES.is_empty()),
            ("both", !CASES.is_empty() && !crate::KEYED_CASES.is_empty()),
        ] {
            let args = ["bench", "--engine", name].map(String::from);
            assert_eq!(bench_engine(&args).is_ok(), enabled, "{name}");
        }
        let args = ["bench".to_owned()];
        assert_eq!(
            bench_engine(&args).is_ok(),
            !CASES.is_empty() || !crate::KEYED_CASES.is_empty()
        );
    }
}
