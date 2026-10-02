//! Executable bottom + Set refinement, separate from planning correspondence.
use cq_planners::Planner;
use cq_testkit::harness::{Outcome, check_refinement_on};
use cq_testkit::{Rows, fuzz};

type Executable = fn(&[Vec<Vec<i32>>]) -> Rows;

struct ExecutableCase {
    seed: u64,
    reference: Executable,
    targets: [Executable; 9],
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
fn every_planner_refines_200_partial_seeds_in_compiled_rust() {
    assert_eq!(EXECUTABLES.len(), 200);
    let mut coverage = fuzz::Coverage::default();
    let mut returned = 0;
    let mut panicked = 0;
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
                Outcome::Panicked(_) => panicked += 1,
            }
            comparisons += 1;
        }
    }
    assert_eq!(comparisons, 1800);
    assert_eq!(coverage.count("cases"), 200);
    assert_eq!(coverage.count("partial-term"), 200);
    for feature in [
        "partial-divide",
        "partial-remainder",
        "partial-overflow-add",
        "partial-overflow-subtract",
        "partial-overflow-multiply",
        "partial-unwrap",
        "partial-guard",
        "partial-let",
        "partial-argument",
        "partial-head",
        "relational-constraint",
        "relational-parameter",
        "empty-used-relation",
        "duplicate-rows",
    ] {
        assert!(coverage.count(feature) > 0, "missing feature {feature}");
    }
    assert!(returned > 0 && panicked > 0, "both outcomes required");
    eprintln!(
        "200 partial seeds, 1800 planner comparisons: {returned} returning references, {panicked} bottom references; coverage {:?}",
        coverage.counts()
    );
}
