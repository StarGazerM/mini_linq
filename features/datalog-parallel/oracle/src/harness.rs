//! Extension execution contexts and labels for the shared oracle harness.
//! Counters run the configured extension evaluator on one local Rayon worker.
//! Reset, evaluation and snapshot occur on that worker, so these thread-local
//! costs include all its tasks; they are not multiworker aggregate measurements.
#[cfg(all(
    feature = "mini",
    feature = "ascent",
    feature = "mini-par",
    feature = "ascent-par"
))]
use crate::CASES;
use crate::Result;
use oracle_shared::harness::{self, Command};

#[cfg(all(feature = "counters", feature = "mini-par"))]
fn check_reach_cost(n: i32, cost: &cq_storage::counters::Snapshot) {
    let n = n as u64;
    assert_eq!(cost.index_builds, 0, "reach chain n={n}");
    // The extension replaces grouped outer scans with borrowed slice scans.
    // EDB canonicalization still visits n inputs; total enumerated work is
    // bounded by the base grouped-read budget, rather than a quadratic rescan.
    assert!(
        (n..=3 * n + 1).contains(&cost.rows_enumerated),
        "reach chain n={n}: {cost:?}"
    );
    assert!(
        (n - 1..=3 * n + 1).contains(&cost.lookups),
        "edge probe budget: {cost:?}"
    );
    // Generated standard maps/sets do not report retained bytes or insertion
    // attempts. Obligations cover observed scan/probe work, not those zeros.
}

#[cfg(all(
    feature = "mini",
    feature = "ascent",
    feature = "mini-par",
    feature = "ascent-par"
))]
fn bench(threads: Option<usize>) -> Result<()> {
    harness::require_bench("datalog-parallel-oracle")?;
    let counts = threads.as_slice();
    let counts = if counts.is_empty() {
        &[1, 2, 4, 5, 8, 15][..]
    } else {
        counts
    };
    let pools = counts
        .iter()
        .map(|&n| rayon::ThreadPoolBuilder::new().num_threads(n).build())
        .collect::<std::result::Result<Vec<_>, _>>()?;
    println!(
        "program\tscenario\tnodes\tedges\trows\tthreads\tmini_us\tascent_us\tmini_par_us\tascent_par_us\tmini_speedup"
    );
    harness::bench_inputs(|name, scenario, n, data| {
        let case = CASES.iter().find(|case| case.name == name).unwrap();
        let expected = (case.oracle)(data).0;
        assert_eq!((case.mini)(data).0, expected);
        let label = format!("{name} {scenario} n={n}");
        let mini = harness::median_us(case.mini, data, &expected, &label);
        let ascent = harness::median_us(case.oracle, data, &expected, &label);
        for pool in &pools {
            pool.install(|| {
                let mini_par = harness::median_us(case.mini_parallel, data, &expected, &label);
                let ascent_par = harness::median_us(case.oracle_parallel, data, &expected, &label);
                println!("{name}\t{scenario}\t{n}\t{}\t{}\t{}\t{mini:.3}\t{ascent:.3}\t{mini_par:.3}\t{ascent_par:.3}\t{:.3}", data.edge.len(), expected.len(), rayon::current_num_threads(), mini / mini_par);
            });
        }
    });
    Ok(())
}

#[cfg(not(all(
    feature = "mini",
    feature = "ascent",
    feature = "mini-par",
    feature = "ascent-par"
)))]
fn bench(_: Option<usize>) -> Result<()> {
    Err("bench requires engine features mini, ascent, mini-par, and ascent-par".into())
}

pub fn run(args: &[String]) -> Result<()> {
    match harness::command(
        args,
        &["--threads"],
        "usage: datalog-parallel-oracle golden | counters (one-worker configured evaluator; not aggregate costs) | bench [--threads N]",
    )? {
        Command::Golden { .. } => crate::goldens()?,
        Command::Bench { threads } => bench(threads)?,
        Command::Counters => {
            #[cfg(all(feature = "counters", feature = "mini-par"))]
            {
                eprintln!(
                    "counter_scope=one_worker configured_engine=mini-par aggregation=thread_local"
                );
                rayon::ThreadPoolBuilder::new()
                    .num_threads(1)
                    .build()?
                    .install(|| {
                        harness::counters(oracle_mini_par::reach::mini, check_reach_cost);
                    });
            }
            #[cfg(not(all(feature = "counters", feature = "mini-par")))]
            return Err("counters requires features mini-par and counters; run cargo run --release -p datalog-parallel-oracle --no-default-features --features mini-par,counters -- counters (one-worker configured evaluator; not aggregate costs)".into());
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "counters", feature = "mini-par"))]
mod tests {
    use super::*;

    #[test]
    fn configured_extension_costs_are_observed_on_its_only_worker() {
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap()
            .install(|| {
                assert_eq!(rayon::current_num_threads(), 1);
                for n in [8, 32] {
                    let (rows, _) =
                        harness::reach_cost(n, oracle_mini_par::reach::mini, check_reach_cost);
                    assert_eq!(rows, n as usize);
                }
            });
    }
}

#[cfg(test)]
mod feature_tests {
    use super::*;

    #[cfg(not(all(
        feature = "mini",
        feature = "ascent",
        feature = "mini-par",
        feature = "ascent-par"
    )))]
    #[test]
    fn benchmark_requires_every_compared_engine() {
        let error = run(&["bench".into()]).unwrap_err().to_string();
        assert!(error.contains("requires engine features mini, ascent, mini-par, and ascent-par"));
    }

    #[cfg(not(all(feature = "mini-par", feature = "ascent-par")))]
    #[test]
    fn goldens_require_both_parallel_engines() {
        let error = run(&["golden".into()]).unwrap_err().to_string();
        assert!(error.contains("requires engine features mini-par and ascent-par"));
    }

    #[cfg(not(all(feature = "counters", feature = "mini-par")))]
    #[test]
    fn counters_require_the_configured_engine_and_instrumentation() {
        let error = run(&["counters".into()]).unwrap_err().to_string();
        assert!(error.contains("requires features mini-par and counters"));
    }

    #[test]
    fn zero_threads_are_rejected() {
        let error = run(&["bench".into(), "--threads".into(), "0".into()])
            .unwrap_err()
            .to_string();
        assert!(error.contains("--threads must be positive"));
    }
}
