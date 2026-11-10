//! Pairwise pipeline coverage for retail_h08; all existing scenarios and configurations.
#![recursion_limit = "512"]
#[macro_use]
mod support;
#[macro_use]
#[path = "support/configurations.rs"]
mod configurations;
#[path = "../../../src/queries/retail/retail_h08.rs"]
mod retail_h08;

const _: () = assert!(!retail_h08::CONFIGURATIONS.is_empty());
