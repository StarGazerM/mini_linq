//! External CQ→RA′ planning passes.
//!
//! [`order!`] consumes CQ with the cq-stage queue protocol and emits complete
//! RA′ text. [`compile_valuation`] exposes the chosen VO text for inspection.
//! Every choice is recorded there; subsequent passes receive only written IR.
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

pub use cq_planners_macros::order;
pub use cq_planners_plan::*;
