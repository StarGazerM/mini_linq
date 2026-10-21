//! Feature-selected sequential and parallel evaluators for the shared oracle corpus.
//! Run `cargo run --release -p datalog-parallel-oracle -- bench [--threads N]`.
//! Every timed answer is compared as a canonical set; golden bytes stay in
//! `features/datalog/oracle/golden`. Fixtures and measurement use `oracle-shared`,
//! never the sequential CLI.
//!
//! Default features `mini`, `ascent`, `mini-par`, and `ascent-par` retain the
//! four-engine benchmark. `--no-default-features` builds only selected engines:
//! `golden` requires `mini-par,ascent-par`; `bench` requires all four; `counters`
//! requires `mini-par,counters`. Missing comparisons return an explicit error.
//! `phase-timing` and `counters` forward only to selected MiniLinq engines.
//! Counters use one local worker, with reset/evaluation/snapshot on that worker;
//! counts are not multiworker totals.
pub mod harness;
use oracle_shared::Result;
#[cfg(all(feature = "mini-par", feature = "ascent-par"))]
use oracle_shared::generate;

pub struct Case {
    pub name: &'static str,
    pub arity: usize,
    #[cfg(feature = "mini")]
    pub mini: oracle_shared::harness::Evaluate,
    #[cfg(feature = "ascent")]
    pub oracle: oracle_shared::harness::Evaluate,
    #[cfg(feature = "mini-par")]
    pub mini_parallel: oracle_shared::harness::Evaluate,
    #[cfg(feature = "ascent-par")]
    pub oracle_parallel: oracle_shared::harness::Evaluate,
}
macro_rules! cases {
    ($($name:ident : $arity:literal),* $(,)?) => {
        pub const CASES: &[Case] = &[$(Case {
            name: stringify!($name), arity: $arity,
            #[cfg(feature = "mini")]
            mini: oracle_mini::$name::mini,
            #[cfg(feature = "ascent")]
            oracle: oracle_ascent::$name::oracle,
            #[cfg(feature = "mini-par")]
            mini_parallel: oracle_mini_par::$name::mini,
            #[cfg(feature = "ascent-par")]
            oracle_parallel: oracle_ascent_par::$name::oracle,
        }),*];
    }
}
cases! { reach:1, tc_left:2, tc_right:2, tc_nonlinear:2, same_generation:2,
mutual:3, multiple_seeds:1, guard:1, unreachable:1, nonedges:2,
recursive_upper:2, andersen:2, cspa:3 }

/// Compare parallel answers with the immutable shared golden corpus.
#[cfg(all(feature = "mini-par", feature = "ascent-par"))]
pub fn goldens() -> Result<()> {
    for case in CASES {
        for scenario in oracle_shared::SCENARIOS {
            let data = generate(scenario, 6, 0x5eed);
            let rows = (case.mini_parallel)(&data).0;
            assert_eq!(rows, (case.oracle_parallel)(&data).0);
            let path = oracle_shared::golden_root()
                .join(case.name)
                .join(format!("{scenario}.csv"));
            cq_testkit::golden::compare(&path, &rows, case.arity)?;
            println!("{}\t{scenario}\t{}", case.name, rows.len());
        }
    }
    Ok(())
}
#[cfg(not(all(feature = "mini-par", feature = "ascent-par")))]
pub fn goldens() -> Result<()> {
    Err("golden requires engine features mini-par and ascent-par".into())
}

#[cfg(all(test, feature = "mini-par", feature = "ascent-par"))]
mod tests {
    use super::*;
    use oracle_shared::{Data, random_data};
    fn compare(case: &Case, data: &Data) {
        let expected = (case.oracle_parallel)(data).0;
        #[cfg(feature = "mini")]
        assert_eq!((case.mini)(data).0, expected, "{} sequential", case.name);
        assert_eq!(
            (case.mini_parallel)(data).0,
            expected,
            "{} extension",
            case.name
        );
        #[cfg(feature = "ascent")]
        assert_eq!(
            (case.oracle)(data).0,
            expected,
            "{} sequential Ascent",
            case.name
        );
        if let Some(reference) = oracle_shared::naive(case.name, data) {
            assert_eq!(expected, reference, "{} dense reference", case.name);
        }
    }
    #[test]
    fn differential_matrix_and_unchanged_goldens() {
        for threads in [1, 2, 4] {
            rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap()
                .install(|| {
                    for case in CASES {
                        for scenario in oracle_shared::SCENARIOS {
                            let data = generate(scenario, 6, 0x5eed);
                            compare(case, &data);
                            let path = oracle_shared::golden_root()
                                .join(case.name)
                                .join(format!("{scenario}.csv"));
                            cq_testkit::golden::compare(
                                &path,
                                &(case.mini_parallel)(&data).0,
                                case.arity,
                            )
                            .unwrap();
                        }
                        for seed in 0..oracle_shared::RANDOM_SEEDS {
                            compare(case, &generate("random", 6, seed));
                        }
                    }
                });
        }
    }
    #[test]
    fn larger_sparse_and_dense_inputs() {
        rayon::ThreadPoolBuilder::new()
            .num_threads(4)
            .build()
            .unwrap()
            .install(|| {
                for case in CASES {
                    for n in [20, 50] {
                        for scenario in oracle_shared::SCENARIOS {
                            compare(case, &generate(scenario, n, 0x5eed));
                        }
                        for (num, den) in [(1, 5), (2, n as u64)] {
                            for seed in 0..2 {
                                compare(case, &random_data(n, seed, num, den));
                            }
                        }
                    }
                }
            });
    }
}
