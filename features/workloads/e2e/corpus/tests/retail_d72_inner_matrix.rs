//! Pairwise pipeline coverage for retail_d72_inner; all existing scenarios and configurations.
#![recursion_limit = "512"]
#[macro_use]
mod support;
#[macro_use]
#[path = "support/configurations.rs"]
mod configurations;
#[path = "../../../src/queries/retail/retail_d72_inner.rs"]
mod retail_d72_inner;

const _: () = assert!(!retail_d72_inner::CONFIGURATIONS.is_empty());
