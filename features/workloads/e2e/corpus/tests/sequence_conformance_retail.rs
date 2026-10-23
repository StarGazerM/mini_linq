//! Sequence conformance for the retail corpus configuration list.
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

mod retail_h03 {
    case_configurations!(retail_h03, run_retail_h03);
}
mod retail_h05 {
    case_configurations!(retail_h05, run_retail_h05);
}
mod retail_h07 {
    case_configurations!(retail_h07, run_retail_h07);
}
mod retail_h08 {
    case_configurations!(retail_h08, run_retail_h08);
}
mod retail_h09 {
    case_configurations!(retail_h09, run_retail_h09);
}
mod retail_h21_positive {
    case_configurations!(retail_h21_positive, run_retail_h21_positive);
}
mod retail_d19 {
    case_configurations!(retail_d19, run_retail_d19);
}
mod retail_d27 {
    case_configurations!(retail_d27, run_retail_d27);
}
mod retail_d72_inner {
    case_configurations!(retail_d72_inner, run_retail_d72_inner);
}
mod retail_d85 {
    case_configurations!(retail_d85, run_retail_d85);
}

#[test]
fn sequential_and_tasks_agree_on_every_corpus_configuration_and_scenario() {
    let configurations = retail_h03::CONFIGURATIONS.len()
        + retail_h05::CONFIGURATIONS.len()
        + retail_h07::CONFIGURATIONS.len()
        + retail_h08::CONFIGURATIONS.len()
        + retail_h09::CONFIGURATIONS.len()
        + retail_h21_positive::CONFIGURATIONS.len()
        + retail_d19::CONFIGURATIONS.len()
        + retail_d27::CONFIGURATIONS.len()
        + retail_d72_inner::CONFIGURATIONS.len()
        + retail_d85::CONFIGURATIONS.len();
    let cases: Vec<_> = cq_workloads::catalog::RETAIL
        .iter()
        .map(|case| case.name)
        .collect();
    conformance::run(configurations, &cases);
}
