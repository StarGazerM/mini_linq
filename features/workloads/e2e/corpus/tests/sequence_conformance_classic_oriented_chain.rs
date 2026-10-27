//! Sequence conformance for the classic oriented_chain configuration list.
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

mod oriented_chain {
    case_configurations!(oriented_chain, run_oriented_chain);
}

#[test]
fn sequential_and_tasks_agree_on_every_corpus_configuration_and_scenario() {
    let cases: Vec<_> = cq_workloads::catalog::CLASSIC
        .iter()
        .map(|case| case.name)
        .filter(|name| name.replace('-', "_") == "oriented_chain")
        .collect();
    assert_eq!(cases.len(), 1, "registered classic case");
    conformance::run(oriented_chain::CONFIGURATIONS.len(), &cases);
}
