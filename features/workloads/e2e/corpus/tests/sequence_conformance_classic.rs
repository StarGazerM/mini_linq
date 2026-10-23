//! Sequence conformance for the classic corpus configuration list.
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

mod intersection {
    case_configurations!(intersection, run_intersection);
}
mod cartesian_product {
    case_configurations!(cartesian_product, run_cartesian_product);
}
mod oriented_chain {
    case_configurations!(oriented_chain, run_oriented_chain);
}
mod claw {
    case_configurations!(claw, run_claw);
}
mod fact_star {
    case_configurations!(fact_star, run_fact_star);
}
mod triangle {
    case_configurations!(triangle, run_triangle);
}
mod cycle_4 {
    case_configurations!(cycle_4, run_cycle_4);
}
mod spade {
    case_configurations!(spade, run_spade);
}
mod barbell {
    case_configurations!(barbell, run_barbell);
}
mod loomis_whitney_4 {
    case_configurations!(loomis_whitney_4, run_loomis_whitney_4);
}

#[test]
fn sequential_and_tasks_agree_on_every_corpus_configuration_and_scenario() {
    let configurations = intersection::CONFIGURATIONS.len()
        + cartesian_product::CONFIGURATIONS.len()
        + oriented_chain::CONFIGURATIONS.len()
        + claw::CONFIGURATIONS.len()
        + fact_star::CONFIGURATIONS.len()
        + triangle::CONFIGURATIONS.len()
        + cycle_4::CONFIGURATIONS.len()
        + spade::CONFIGURATIONS.len()
        + barbell::CONFIGURATIONS.len()
        + loomis_whitney_4::CONFIGURATIONS.len();
    let cases: Vec<_> = cq_workloads::catalog::CLASSIC
        .iter()
        .map(|case| case.name)
        .collect();
    conformance::run(configurations, &cases);
}
