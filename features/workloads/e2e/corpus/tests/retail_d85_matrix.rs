//! Pairwise pipeline coverage for retail_d85; all existing scenarios and configurations.
#![recursion_limit = "512"]
#[macro_use]
mod support;
#[macro_use]
#[path = "support/configurations.rs"]
mod configurations;
#[path = "../../../src/queries/retail/retail_d85.rs"]
mod retail_d85;

const _: () = assert!(!retail_d85::CONFIGURATIONS.is_empty());
