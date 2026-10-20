//! Engine-independent command parsing, fixtures, timing and cost measurement.
//! Evaluators own their configuration and return canonical answers with their
//! internal elapsed time. Counter callers must keep evaluation and observation
//! on the same thread; the storage counters are thread-local.
use crate::{CASES, Data, Result, Rows, generate, random_data};
use std::{hint::black_box, time::Duration};

pub type Evaluate = fn(&Data) -> (Rows, Duration);

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Golden { write: bool },
    Bench { threads: Option<usize> },
    Counters,
}

/// Accepted options belong to the caller, not to an engine-selection switch.
pub fn command(args: &[String], options: &[&str], usage: &str) -> Result<Command> {
    match args.first().map(String::as_str) {
        Some("golden") if args.len() == 1 => Ok(Command::Golden { write: false }),
        Some("golden")
            if args.len() == 2 && args[1] == "--write" && options.contains(&"--write") =>
        {
            Ok(Command::Golden { write: true })
        }
        Some("bench") if args.len() == 1 => Ok(Command::Bench { threads: None }),
        Some("bench")
            if args.len() == 3 && args[1] == "--threads" && options.contains(&"--threads") =>
        {
            let count = args[2].parse::<usize>()?;
            if count == 0 {
                return Err("--threads must be positive".into());
            }
            Ok(Command::Bench {
                threads: Some(count),
            })
        }
        Some("counters") if args.len() == 1 => Ok(Command::Counters),
        _ => Err(usage.into()),
    }
}

pub fn require_bench(package: &str) -> Result<()> {
    if cq_storage::profile_select!(plain { false } counters { true }) {
        return Err(format!("bench requires counters disabled; run cargo run --release -p {package} -- bench without enabling counters in any selected package").into());
    }
    if cfg!(debug_assertions) {
        return Err("bench requires --release".into());
    }
    Ok(())
}

/// Visit the common sweep without choosing an evaluator or execution context.
pub fn bench_inputs(mut measure: impl FnMut(&str, &str, i32, &Data)) {
    for name in ["reach", "tc_left", "tc_nonlinear", "same_generation"] {
        let sizes = if name == "tc_nonlinear" {
            [128, 256, 512]
        } else {
            [256, 1024, 4096]
        };
        for n in sizes {
            measure(name, "chain", n, &generate("chain", n, 0));
        }
    }
    for n in [1000, 5000] {
        let mut data = random_data(n, 0x5eed, 2, n as u64);
        // A fixed seed set avoids benchmarking a single isolated source.
        data.seed = (0..16).map(|x| (x,)).collect();
        measure("reach", "random_degree2_seed16", n, &data);
    }
}

/// Input cloning precedes each evaluator's timer; canonicalization follows it.
/// Every sample is checked, not only the answer of a warm-up invocation.
pub fn median_us(evaluate: Evaluate, data: &Data, expected: &Rows, label: &str) -> f64 {
    let mut times = [Duration::ZERO; 5];
    for time in &mut times {
        let (actual, elapsed) = black_box(evaluate(black_box(data)));
        assert_eq!(&actual, expected, "{label}");
        *time = elapsed;
    }
    times.sort_unstable();
    times[2].as_secs_f64() * 1e6
}

/// Measure only the supplied evaluator; the independent reference precedes reset.
/// Cost obligations are supplied by the caller's execution policy.
#[cfg(feature = "counters")]
pub fn reach_cost(
    n: i32,
    evaluate: Evaluate,
    check: impl FnOnce(i32, &cq_storage::counters::Snapshot),
) -> (usize, cq_storage::counters::Snapshot) {
    let data = generate("chain", n, 0);
    let expected = crate::programs::reach::oracle(&data).0;
    cq_storage::counters::reset();
    let actual = evaluate(&data).0;
    let cost = cq_storage::counters::snapshot();
    assert_eq!(actual, expected, "reach chain n={n}");
    check(n, &cost);
    (actual.len(), cost)
}

#[cfg(feature = "counters")]
pub fn counters(evaluate: Evaluate, check: impl Fn(i32, &cq_storage::counters::Snapshot)) {
    println!(
        "program\tnodes\trows\thashes\tbuilds\tlookups\tcursor_steps\trows_enumerated\tdistinct_insertions\tbytes_peak"
    );
    for n in [2000, 4000, 8000] {
        let (rows, cost) = reach_cost(n, evaluate, &check);
        println!(
            "reach\t{n}\t{rows}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            cost.hash_computations,
            cost.index_builds,
            cost.lookups,
            cost.cursor_steps,
            cost.rows_enumerated,
            cost.distinct_insertions,
            cost.bytes_retained_peak
        );
    }
}

#[cfg(feature = "counters")]
pub fn check_reach_cost(n: i32, cost: &cq_storage::counters::Snapshot) {
    assert_eq!(cost.index_builds, 0, "reach chain n={n}");
    // Raw EDB canonicalization visits n rows once; its borrowed adapter
    // adds no core cursor events. Grouped seed/delta/edge reads add 2*n+1.
    assert_eq!(cost.rows_enumerated, 3 * n as u64 + 1, "reach chain n={n}");
}

pub fn run(args: &[String]) -> Result<()> {
    match command(
        args,
        &["--write"],
        "usage: datalog-oracle golden [--write] | counters | bench",
    )? {
        Command::Golden { write } => {
            crate::goldens(write, false)?;
        }
        Command::Bench { .. } => {
            require_bench("datalog-oracle")?;
            println!("program\tscenario\tnodes\tedges\trows\tmini_us\tascent_us");
            bench_inputs(|name, scenario, n, data| {
                let case = CASES.iter().find(|case| case.name == name).unwrap();
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
            counters(crate::programs::reach::mini, check_reach_cost);
            #[cfg(not(feature = "counters"))]
            return Err("counters requires --features counters; run cargo run --release -p datalog-oracle --features counters -- counters".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| (*word).into()).collect()
    }

    #[test]
    fn command_options_are_caller_owned() {
        assert_eq!(
            command(&args(&["golden", "--write"]), &["--write"], "usage").unwrap(),
            Command::Golden { write: true }
        );
        assert!(command(&args(&["golden", "--write"]), &[], "usage").is_err());
        assert_eq!(
            command(&args(&["bench", "--threads", "3"]), &["--threads"], "usage").unwrap(),
            Command::Bench { threads: Some(3) }
        );
        for count in ["0", "invalid"] {
            assert!(
                command(
                    &args(&["bench", "--threads", count]),
                    &["--threads"],
                    "usage"
                )
                .is_err()
            );
        }
        assert!(command(&args(&["bench", "--threads", "3"]), &[], "usage").is_err());
        assert!(command(&args(&["counters", "extra"]), &[], "usage").is_err());
    }

    #[test]
    fn median_uses_the_supplied_evaluator_and_internal_time() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static CALLS: AtomicUsize = AtomicUsize::new(0);
        fn evaluate(_: &Data) -> (Rows, Duration) {
            let micros = [9, 1, 7, 3, 5][CALLS.fetch_add(1, Ordering::Relaxed)];
            (vec![vec![42]], Duration::from_micros(micros))
        }
        assert_eq!(
            median_us(evaluate, &Data::default(), &vec![vec![42]], "custom engine"),
            5.0
        );
        assert_eq!(CALLS.load(Ordering::Relaxed), 5);
    }

    #[cfg(feature = "counters")]
    #[test]
    fn counter_measurement_invokes_supplied_evaluator_and_obligation() {
        use std::cell::Cell;
        fn evaluate(data: &Data) -> (Rows, Duration) {
            assert_eq!(data.nodes.len(), 8);
            crate::programs::reach::mini(data)
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
