//! Executable Datalog reference semantics and pass conformance support.
//!
//! `cq_testkit::cq_semantics` is an integer-fragment interpreter: it rejects
//! computed bindings and generators. It cannot interpret opaque Rust soundly.
//! The test-pass evaluators instead generate typed, written-order nested loops,
//! keeping Rust expressions native. [`eval_source`] drives a naive fixpoint;
//! [`eval_incremental`] executes recorded semi-naive variants; [`eval_round`]
//! drives recorded CQ holes, admission, publication, and termination.
//! No production CQ compiler or printer participates in reference evaluation.
//! Relations use set union, EDBs are canonicalized once, and lower components
//! remain frozen while an upper component evaluates its immutable snapshot.
//!
//! Pure total Rust, lawful Eq/Hash/Clone, finite generators and termination
//! are caller premises, just as for the compiler. [`check_refinement`] rejects
//! a panic in either execution; divergence is not approximated with a round limit.
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
/// A panic in initialization or any rule propagates so that
/// `check_refinement(|| eval_source(...), target)` reports the violated premise.
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

/// Execute a recorded semi-naive covering round system in component order.
///
/// `round` evaluates seed variants when its last argument is true and delta
/// variants otherwise. It computes candidates from the old snapshot, then
/// assigns `delta = candidates \\ accumulated`, unions delta into accumulated,
/// and returns whether any delta is nonempty. Lower components remain frozen.
/// Initialization and rule panics propagate unchanged.
pub fn eval_incremental<S>(
    initialize: impl FnOnce() -> S,
    components: usize,
    mut round: impl FnMut(&mut S, usize, bool) -> bool,
) -> S {
    let mut state = initialize();
    for component in 0..components {
        let mut changed = round(&mut state, component, true);
        while changed {
            changed = round(&mut state, component, false);
        }
    }
    state
}

/// Execute one recorded Round component, with component-local key borrows.
///
/// The callback executes the seed schedule once (`true`), then the recursive
/// schedule (`false`) while the recorded termination test reports a nonempty
/// delta. Each invocation evaluates recorded holes, admits candidates, and
/// publishes in the recorded phase order before returning that test's result.
/// Both checked loop forms (`while_nonempty` and `loop_until_empty`) have this
/// control flow. The generated caller owns relation/layout bindings and freezes
/// lower components for the lifetime of its negation-key sets.
///
/// Panics propagate; there is no round cap or panic recovery.
pub fn eval_round(mut round: impl FnMut(bool) -> bool) {
    let mut nonempty = round(true);
    while nonempty {
        nonempty = round(false);
    }
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
        assert!(check_refinement(|| panic!("reference"), || result).is_err());
        assert!(check_refinement(|| 1, || 2).is_err());
    }
}
