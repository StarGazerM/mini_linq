//! Executable set refinement under total Rust, separate from planning correspondence.
use cq_planners::Planner;
use cq_testkit::harness::{Outcome, check_refinement_on};
use cq_testkit::{Rows, fuzz};

type Executable = fn(&[Vec<Vec<i32>>]) -> Rows;

struct ExecutableCase {
    seed: u64,
    reference: Executable,
    targets: [Executable; Planner::ALL.len()],
}

trait Integer {
    fn integer(self) -> i32;
}

impl Integer for i32 {
    fn integer(self) -> i32 {
        self
    }
}

impl Integer for &i32 {
    fn integer(self) -> i32 {
        *self
    }
}

fn integer(value: impl Integer) -> i32 {
    value.integer()
}

include!(concat!(env!("OUT_DIR"), "/refinement.rs"));

#[test]
fn every_planner_refines_200_rust_seeds_in_compiled_rust() {
    assert_eq!(EXECUTABLES.len(), 200);
    let mut coverage = fuzz::Coverage::default();
    let mut returned = 0;
    let mut comparisons = 0;
    for executable in EXECUTABLES {
        let case = fuzz::generate(executable.seed);
        coverage.observe(&case);
        for (planner, target) in Planner::ALL.iter().zip(executable.targets) {
            let result = check_refinement_on(case.data.as_slice(), executable.reference, target)
                .unwrap_or_else(|error| {
                    panic!("seed {}, {planner:?}: {error}\n{}", case.seed, case.cq())
                });
            match result.reference {
                Outcome::Returned(_) => returned += 1,
                Outcome::Panicked(message) => panic!("total reference panicked: {message}"),
            }
            comparisons += 1;
        }
    }
    assert_eq!(comparisons, 200 * Planner::ALL.len());
    assert_eq!(coverage.count("cases"), 200);
    assert_eq!(coverage.count("rust-term"), 200);
    for feature in [
        "rust-divide",
        "rust-remainder",
        "rust-bounded-add",
        "rust-bounded-subtract",
        "rust-bounded-multiply",
        "rust-unwrap",
        "rust-guard",
        "rust-let",
        "rust-argument",
        "rust-head",
        "relational-constraint",
        "relational-parameter",
        "empty-used-relation",
        "duplicate-rows",
    ] {
        assert!(coverage.count(feature) > 0, "missing feature {feature}");
    }
    assert_eq!(returned, comparisons, "every reference must return");
    eprintln!(
        "200 opaque Rust seeds, {comparisons} planner comparisons: {returned} returning references; coverage {:?}",
        coverage.counts()
    );
}
