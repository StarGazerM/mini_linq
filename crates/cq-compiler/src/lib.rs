//! MiniLinq's typed compiler passes and Rust-shaped physical boundary.
//!
//! Token parsing and macro routing live in the `mini-linq` proc-macro crate.
//! This crate accepts typed IR values; their semantic well-formedness belongs
//! to the corresponding `compile_*` pass contracts.

mod cq_to_relational;
mod indexes_to_rust;
mod pass_contracts;
mod passes;
mod relational_to_indexes;
pub use cq_to_relational::{
    GyoRule, JoinPlanReport, JoinPolicy, JoinTree, PlanningOptions, Rule as JoinRule,
};

pub use passes::{
    check_cq_plan, compile_cq, compile_cq_derivation, compile_cq_planned, compile_cq_with_policy,
    compile_index_requirements, compile_iterator_pipeline, compile_materialized, compile_pull,
    compile_relational_plan, compile_result_indexes, explain_cq_plan,
};
