//! Executable set refinement under total Rust, separate from planning correspondence.
use cq_planners::Planner;
use cq_testkit::harness::{Outcome, check_refinement_on};
use cq_testkit::{Rows, fuzz};

type Executable = fn(&[Vec<Vec<i32>>]) -> Rows;

pub(crate) struct ExecutableCase {
    pub(crate) seed: u64,
    pub(crate) reference: Executable,
    pub(crate) targets: [Executable; Planner::ALL.len()],
}

pub(crate) trait Integer {
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

pub(crate) fn integer(value: impl Integer) -> i32 {
    value.integer()
}

pub(crate) fn check_shard(executables: &[ExecutableCase], shard: usize) {
    assert_eq!(Planner::ALL.len(), 11);
    assert_eq!(executables.len(), 25);
    assert_eq!(
        executables.iter().map(|case| case.seed).collect::<Vec<_>>(),
        (0..200).filter(|ordinal| ordinal % 8 == shard).map(|ordinal| ordinal as u64 * 2).collect::<Vec<_>>(),
    );
    let mut coverage = fuzz::Coverage::default();
    let mut returned = 0;
    let mut comparisons = 0;
    for executable in executables {
        let case = fuzz::generate(executable.seed);
        coverage.observe(&case);
        for (planner, target) in Planner::ALL.iter().zip(executable.targets) {
            let result =
                check_refinement_on(case.i32_data().as_slice(), executable.reference, target)
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
    assert_eq!(comparisons, 25 * Planner::ALL.len());
    assert_eq!(coverage.count("cases"), 25);
    assert_eq!(coverage.count("rust-term"), 25);
    assert_eq!(returned, comparisons, "every reference must return");
    eprintln!(
        "shard {shard}: 25 opaque Rust seeds, {comparisons} planner comparisons: {returned} returning references; coverage {:?}",
        coverage.counts()
    );
}
