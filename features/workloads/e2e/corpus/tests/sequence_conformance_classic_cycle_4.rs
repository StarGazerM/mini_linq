//! Sequence conformance for the classic cycle_4 configuration list.
//! One crate per classic case keeps each generated test crate small enough to
//! compile in parallel without superlinear rustc time and memory.
#![recursion_limit = "512"]
#![allow(dead_code)]
#[macro_use]
#[path = "support/sequence_conformance.rs"]
mod conformance;
#[macro_use]
#[path = "support/le_configurations.rs"]
mod le_configurations;
#[macro_use]
#[path = "support/configurations.rs"]
mod configurations;

mod cycle_4 {
    case_configurations!(cycle_4, run_cycle_4);
}

#[test]
fn sequential_and_tasks_agree_on_every_corpus_configuration_and_scenario() {
    let cases: Vec<_> = cq_workloads::catalog::CLASSIC
        .iter()
        .map(|case| case.name)
        .filter(|name| name.replace('-', "_") == "cycle_4")
        .collect();
    assert_eq!(cases.len(), 1, "registered classic case");
    conformance::run(cycle_4::CONFIGURATIONS.len(), &cases);
}
