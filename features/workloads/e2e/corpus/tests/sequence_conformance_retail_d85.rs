//! Sequence conformance for the retail_d85 corpus configuration list.
//! One crate per retail case keeps each generated test crate small enough to
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

mod retail_d85 {
    case_configurations!(retail_d85, run_retail_d85);
}

#[test]
fn sequential_and_tasks_agree_on_every_corpus_configuration_and_scenario() {
    let cases: Vec<_> = cq_workloads::catalog::RETAIL
        .iter()
        .map(|case| case.name)
        .filter(|name| name.replace('-', "_") == "retail_d85")
        .collect();
    assert_eq!(cases.len(), 1, "registered retail case");
    conformance::run(retail_d85::CONFIGURATIONS.len(), &cases);
}
