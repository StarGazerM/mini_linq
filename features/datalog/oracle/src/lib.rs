//! Complete-answer differential tests against sequential Ascent.
//! `datalog-oracle golden --write` is the only golden regeneration command.
//! Owned-string differential matrices also compare each heterogeneous output
//! against sequential Ascent: literal selections and heads, computed descriptors,
//! renamed keyword variables, constant-filtered stratified negation, and shared
//! delta runs with distinct key orders, later zero-key probes, nonlinear and
//! mutual recursion, duplicate inputs, and guards before indexed delta reads.
//! These cases do not alter the existing integer CSV golden corpus.
//!
//! Run from `features/datalog`:
//! - Exactness (no counters): `cargo test -p datalog-oracle`.
//! - Uninstrumented Fx-default sweep: `cargo run --release -p datalog-oracle -- bench`.
//! - Keyed sweep: `cargo run --release -p datalog-oracle -- bench --engine mini-keyed`.
//!   This uses the same fixtures, five-sample medians and full-set Ascent checks
//!   as `bench` (or `bench --engine mini`), including reach and TC workloads.
//!   Keyed aHash replaces Fx in storage and Structures-stage hole deduplication,
//!   without changing the programs.
//! - Paired hasher sweep: `cargo run --release -p datalog-oracle -- bench --engine both`.
//!   Warm both engines, then compare three checked samples per hasher in AB/BA/AB
//!   order on every fixture. `ahash_us` measures `mini-keyed`, `fx_us` measures
//!   Fx-default `mini`, and `fx_speedup` is `ahash_us / fx_us`.
//! - Counter assertions: `cargo run --release -p datalog-oracle --features counters -- counters`.
//!
//! `hasher-summary.tsv` retains the original `dadaed6` measurements, when Fx was
//! opt-in and `mini` used keyed aHash. Its `ahash_us`, `fx_us` and `fx_speedup`
//! columns describe those historical implementations, not this default cutover.
//!
//! `oracle_shared::oracle_programs!` accepts a single engine selector (`mini` or
//! `ascent`) and arbitrary named pipeline configuration; unselected engines never expand.
//! Each timed result is compared as a complete canonical set.
//! `phase-timing` enables per-round stderr separately from timing runs.
//!
//! `counters` requires the explicit feature. `bench` refuses storage counters,
//! including when another selected package enables them through feature unification.
//! `oracle_shared::harness` owns CLI parsing, benchmark fixtures, checked median
//! timing and callback-based counter obligations. This crate's [`harness`] only
//! dispatches sequential comparisons; parallel consumers use shared helpers directly.
//! Engines compile independently; default features select `mini`, `mini-keyed`, and
//! `ascent`. A focused build uses `--no-default-features --features mini,ascent`
//! (or `mini-keyed,ascent`). Comparisons require Ascent plus the selected engine.
use cq_testkit::golden;
use oracle_shared::{Data, Result, Rows, SCENARIOS, generate, golden_root};
#[cfg(test)]
use oracle_shared::{RANDOM_SEEDS, naive, random_data};
use std::collections::BTreeSet;
#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;
pub mod harness;
#[cfg(test)]
mod strings;
pub struct Case {
    pub name: &'static str,
    pub arity: usize,
    pub mini: fn(&Data) -> (Rows, Duration),
    pub oracle: fn(&Data) -> (Rows, Duration),
}
macro_rules! cases {
    ($($name:ident : $arity:literal),* $(,)?) => {
        #[cfg(all(feature = "mini", feature = "ascent"))]
        pub const CASES: &[Case] = &[$(Case {name: stringify!($name), arity: $arity,
            mini: oracle_mini::$name::mini, oracle: oracle_ascent::$name::oracle}),*];
        #[cfg(all(feature = "mini-keyed", feature = "ascent"))]
        pub const KEYED_CASES: &[Case] = &[$(Case {name: stringify!($name), arity: $arity,
            mini: oracle_mini_keyed::$name::mini, oracle: oracle_ascent::$name::oracle}),*];
    }
}
cases! { reach:1, tc_left:2, tc_right:2, tc_nonlinear:2, same_generation:2,
mutual:3, multiple_seeds:1, guard:1, unreachable:1, nonedges:2,
recursive_upper:2, andersen:2, cspa:3 }

#[cfg(not(all(feature = "mini", feature = "ascent")))]
pub const CASES: &[Case] = &[];
#[cfg(not(all(feature = "mini-keyed", feature = "ascent")))]
pub const KEYED_CASES: &[Case] = &[];
fn selected_cases() -> Result<&'static [Case]> {
    if !CASES.is_empty() {
        Ok(CASES)
    } else if !KEYED_CASES.is_empty() {
        Ok(KEYED_CASES)
    } else {
        Err("comparisons require features ascent and mini or mini-keyed".into())
    }
}
pub fn goldens(write: bool, mini: bool) -> Result<BTreeSet<PathBuf>> {
    let mut checked = BTreeSet::new();
    for c in selected_cases()? {
        for scenario in SCENARIOS {
            let data = generate(scenario, 6, 0x5eed);
            let rows = if mini {
                (c.mini)(&data).0
            } else {
                (c.oracle)(&data).0
            };
            let path = golden_root().join(c.name).join(format!("{scenario}.csv"));
            if write {
                std::fs::create_dir_all(path.parent().unwrap())?;
                std::fs::write(&path, golden::canonical_bytes(&rows, c.arity)?)?;
            } else {
                golden::compare(&path, &rows, c.arity)?;
            }
            println!("{}\t{}\t{}", c.name, scenario, rows.len());
            checked.insert(path);
        }
    }
    Ok(checked)
}

#[cfg(all(
    test,
    feature = "ascent",
    any(feature = "mini", feature = "mini-keyed")
))]
mod tests {
    use super::*;
    #[test]
    fn differential_matrix() {
        for c in CASES {
            for scenario in SCENARIOS {
                check(c, scenario, 0x5eed);
            }
            for seed in 0..RANDOM_SEEDS {
                check(c, "random", seed);
            }
        }
        for c in KEYED_CASES {
            for scenario in SCENARIOS {
                check(c, scenario, 0x5eed);
            }
            for seed in 0..RANDOM_SEEDS {
                check(c, "random", seed);
            }
        }
    }
    fn check(c: &Case, scenario: &str, seed: u64) {
        let d = generate(scenario, 6, seed);
        check_data(c, &d, &format!("{scenario} / seed={seed}"));
    }
    fn check_data(c: &Case, d: &Data, context: &str) {
        let expected = (c.oracle)(d).0;
        assert_eq!((c.mini)(d).0, expected, "{} / {context}", c.name);
        if let Some(naive) = naive(c.name, d) {
            assert_eq!(expected, naive, "naive {} / {context}", c.name);
        }
    }
    #[test]
    fn larger_differential_matrix() {
        for c in CASES.iter().chain(KEYED_CASES) {
            for n in [20, 50] {
                for scenario in SCENARIOS {
                    let d = generate(scenario, n, 0x5eed);
                    check_data(c, &d, &format!("{scenario} / n={n}"));
                }
            }
            for n in [6, 20, 50] {
                for (num, den) in [(1, 5), (2, n as u64)] {
                    for seed in 0..2 {
                        let d = random_data(n, seed, num, den);
                        check_data(
                            c,
                            &d,
                            &format!("random / n={n} / p={num}/{den} / seed={seed}"),
                        );
                    }
                }
            }
        }
        for n in [6, 20, 50] {
            for (num, den) in [(1, 5), (2, n as u64)] {
                for seed in 0..2 {
                    strings::check(&random_data(n, seed, num, den));
                }
            }
        }
        for n in [20, 50] {
            for scenario in SCENARIOS {
                strings::check(&generate(scenario, n, 0x5eed));
            }
        }
    }
    #[test]
    fn checked_in_goldens() {
        goldens(false, true).unwrap();
    }
    #[test]
    fn oracle_goldens() {
        goldens(false, false).unwrap();
    }
    #[test]
    fn golden_inventory() {
        let checked = goldens(false, false).unwrap();
        // Packaged external consumers have no Git repository. Walk all files,
        // including ignored extras, rather than relying on the Git-only helper.
        fn files(directory: &Path, found: &mut BTreeSet<PathBuf>) {
            for entry in std::fs::read_dir(directory).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    files(&entry.path(), found);
                } else {
                    found.insert(entry.path());
                }
            }
        }
        let mut actual = BTreeSet::new();
        files(&golden_root(), &mut actual);
        assert_eq!(actual, checked);
    }
    #[test]
    fn owned_strings() {
        for scenario in SCENARIOS {
            strings::check(&generate(scenario, 6, 0x5eed));
        }
        for seed in 0..RANDOM_SEEDS {
            strings::check(&generate("random", 6, seed));
        }
    }
}
