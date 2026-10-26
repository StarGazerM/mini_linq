//! Sequence conformance for the prefix_greedy_coarse kernel (26 pairwise configurations).
#![recursion_limit = "512"]
#![allow(dead_code)]
#[macro_use]
#[path = "support/sequence_conformance.rs"]
mod conformance;
#[macro_use]
#[path = "support/sequence_matrix.rs"]
mod sequence_matrix;

sequence_matrix!([prefix greedy, coarse]; merge_hosts: ordered);

#[test]
fn sequential_and_tasks_agree_on_every_corpus_configuration_and_scenario() {
    conformance::run(26, &["triangle"]);
}
