//! Shared source corpus instantiated with hybrid parallel execution and Ascent.
//! Run `cargo run --release -p datalog-parallel-oracle -- bench [--threads N]`.
//! Every timed answer is compared as a canonical set; golden bytes stay in base.
//! CLI parsing, fixtures, timing and counter measurement delegate to the base
//! [`datalog_oracle::harness`]; this crate supplies evaluators and local pools.
//! `counters` measures this configured extension on one local worker, with
//! reset/evaluation/snapshot on that worker. Counts are not multiworker totals.
pub mod harness;
pub use datalog_oracle::{Data, Result, Rows, generate, random_data};
use std::time::Duration;

#[macro_export]
macro_rules! oracle_input {
    ($rows:expr) => {
        $rows.iter().cloned().collect()
    };
}
datalog_oracle::oracle_programs!(programs, ascent::ascent_par, crate::oracle_input, pipeline {
    storage_rewrites = [cq_datalog_parallel::shard_storage];
    round = [cq_datalog_parallel::partition];
    print = cq_datalog_parallel::print_rounds;
});

pub struct Case {
    pub name: &'static str,
    pub arity: usize,
    pub mini: fn(&Data) -> (Rows, Duration),
    pub oracle: fn(&Data) -> (Rows, Duration),
    pub mini_parallel: fn(&Data) -> (Rows, Duration),
    pub oracle_parallel: fn(&Data) -> (Rows, Duration),
}
macro_rules! cases {
    ($($name:ident : $arity:literal),* $(,)?) => {
        pub const CASES: &[Case] = &[$(Case {
            name: stringify!($name), arity: $arity,
            mini: datalog_oracle::programs::$name::mini,
            oracle: datalog_oracle::programs::$name::oracle,
            mini_parallel: programs::$name::mini,
            oracle_parallel: programs::$name::oracle,
        }),*];
    }
}
cases! { reach:1, tc_left:2, tc_right:2, tc_nonlinear:2, same_generation:2,
mutual:3, multiple_seeds:1, guard:1, unreachable:1, nonedges:2,
recursive_upper:2, andersen:2, cspa:3 }

/// Compare extension answers with the immutable base golden corpus.
pub fn goldens() -> Result<()> {
    for case in CASES {
        for scenario in datalog_oracle::SCENARIOS {
            let data = generate(scenario, 6, 0x5eed);
            let rows = (case.mini_parallel)(&data).0;
            assert_eq!(rows, (case.oracle_parallel)(&data).0);
            let path = datalog_oracle::golden_root()
                .join(case.name)
                .join(format!("{scenario}.csv"));
            cq_testkit::golden::compare(&path, &rows, case.arity)?;
            println!("{}\t{scenario}\t{}", case.name, rows.len());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn compare(case: &Case, data: &Data) {
        let expected = (case.oracle)(data).0;
        assert_eq!((case.mini)(data).0, expected, "{} sequential", case.name);
        assert_eq!(
            (case.mini_parallel)(data).0,
            expected,
            "{} extension",
            case.name
        );
        assert_eq!(
            (case.oracle_parallel)(data).0,
            expected,
            "{} Ascent",
            case.name
        );
        if let Some(reference) = datalog_oracle::naive(case.name, data) {
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
                        for scenario in datalog_oracle::SCENARIOS {
                            let data = generate(scenario, 6, 0x5eed);
                            compare(case, &data);
                            let path = datalog_oracle::golden_root()
                                .join(case.name)
                                .join(format!("{scenario}.csv"));
                            cq_testkit::golden::compare(
                                &path,
                                &(case.mini_parallel)(&data).0,
                                case.arity,
                            )
                            .unwrap();
                        }
                        for seed in 0..datalog_oracle::RANDOM_SEEDS {
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
                        for scenario in datalog_oracle::SCENARIOS {
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
