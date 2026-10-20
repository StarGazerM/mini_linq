//! Executable DL-Source reference semantics and pass conformance support.
//!
//! `cq_testkit::cq_semantics` is an integer-fragment interpreter: it rejects
//! computed bindings and generators. It cannot interpret opaque Rust soundly.
//! Instead, `cq_datalog_test_passes::eval_source!` generates typed, written-order
//! nested loops, keeping Rust expressions native, and calls this generic driver.
//! No storage planner, incremental compiler, CQ compiler, or production printer
//! participates in reference evaluation. Relations use set union; the generated
//! step reads the old state and publishes candidates only after every rule in
//! its stratum has run. EDBs are canonicalized once and lower strata are frozen.
//!
//! Pure partial Rust, lawful Eq/Hash/Clone, finite generators and termination
//! are caller premises, just as for the compiler. Panic denotes bottom, observed
//! by [`check_refinement`]; divergence is not approximated with a round limit.
//! Only unwinding panics are observable. Observers are operational diagnostics,
//! not part of the returned relation denotation, and are not invoked here.

pub use cq_testkit::PassCase;
pub use cq_testkit::harness::{
    Outcome, Refinement, RefinementFailure, check_refinement, check_refinement_on,
};

/// Evaluate a naive stratified set fixpoint using typed rule callbacks.
///
/// `initialize` supplies canonical EDB sets and empty IDB sets. `step` evaluates
/// all rules in the selected stratum against one immutable round snapshot,
/// unions candidates into that stratum's IDBs, and returns whether any new row
/// was admitted. It must not mutate EDBs or other strata. Generated callbacks
/// enforce this discipline without copying the database each round.
///
/// A panic in initialization or any rule propagates as semantic bottom so that
/// `check_refinement(|| eval_source(...), target)` observes it exactly once.
pub fn eval_source<S>(
    initialize: impl FnOnce() -> S,
    strata: usize,
    mut step: impl FnMut(&mut S, usize) -> bool,
) -> S {
    let mut state = initialize();
    for stratum in 0..strata {
        while step(&mut state, stratum) {}
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn synchronous_fixpoint_and_panic_refinement() {
        let result = eval_source(
            || BTreeSet::from([0]),
            1,
            |rows, _| {
                let candidates: Vec<_> = rows.iter().filter(|&&n| n < 4).map(|n| n + 1).collect();
                let before = rows.len();
                rows.extend(candidates);
                before != rows.len()
            },
        );
        assert_eq!(result, BTreeSet::from([0, 1, 2, 3, 4]));
        assert!(check_refinement(|| result.clone(), || panic!("target")).is_err());
        assert!(check_refinement(|| panic!("reference"), || result).is_ok());
        assert!(check_refinement(|| 1, || 2).is_err());
    }
}
