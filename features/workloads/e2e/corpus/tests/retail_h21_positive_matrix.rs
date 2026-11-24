//! Pairwise pipeline coverage for retail_h21_positive; all existing scenarios and configurations.
#![recursion_limit = "512"]
#[macro_use]
mod support;
#[macro_use]
#[path = "support/configurations.rs"]
mod configurations;
#[path = "../../../src/queries/retail/retail_h21_positive.rs"]
mod retail_h21_positive;

const _: () = assert!(!retail_h21_positive::CONFIGURATIONS.is_empty());
