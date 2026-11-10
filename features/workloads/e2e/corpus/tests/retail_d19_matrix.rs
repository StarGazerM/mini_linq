//! Pairwise pipeline coverage for retail_d19; all existing scenarios and configurations.
#![recursion_limit = "512"]
#[macro_use]
mod support;
#[macro_use]
#[path = "support/configurations.rs"]
mod configurations;
#[path = "../../../src/queries/retail/retail_d19.rs"]
mod retail_d19;

const _: () = assert!(!retail_d19::CONFIGURATIONS.is_empty());
