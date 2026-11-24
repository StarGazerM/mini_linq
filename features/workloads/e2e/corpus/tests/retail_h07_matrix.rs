//! Pairwise pipeline coverage for retail_h07; all existing scenarios and configurations.
#![recursion_limit = "512"]
#[macro_use]
mod support;
#[macro_use]
#[path = "support/configurations.rs"]
mod configurations;
#[path = "../../../src/queries/retail/retail_h07.rs"]
mod retail_h07;

const _: () = assert!(!retail_h07::CONFIGURATIONS.is_empty());
