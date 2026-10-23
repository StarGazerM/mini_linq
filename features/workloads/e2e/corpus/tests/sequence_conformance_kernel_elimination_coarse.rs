//! Sequence conformance for the elimination_coarse kernel (224 configurations).
#![recursion_limit = "512"]
#![allow(dead_code)]
#[macro_use]
#[path = "support/sequence_conformance.rs"]
mod conformance;
#[macro_use]
#[path = "support/full_matrix.rs"]
mod full_matrix;

full_matrix!([elimination, coarse]; merge_hosts: ordered);

#[test]
fn sequential_and_tasks_agree_on_every_corpus_configuration_and_scenario() {
    conformance::run(224, &["triangle"]);
}
