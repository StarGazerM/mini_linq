//! Optional offline compiler-record export and static, evidence-bound advice.
//!
//! Export never executes a query or reconstructs a schedule. Compile helpers
//! invoke the production passes; record-only exporters consume checked IR.
//! The separate cost engine enumerates legal orders and emits authored advice,
//! never changing compiler defaults or a running plan. Consumers bind results
//! to the recorded source/compiler/dataset identity before adoption.
#![forbid(unsafe_code)]

pub mod export;
pub mod cost;
pub mod artifacts;

#[cfg(feature = "planner-calibration")]
pub mod calibration;
