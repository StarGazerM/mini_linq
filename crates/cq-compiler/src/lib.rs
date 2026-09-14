//! MiniLinq's typed compiler passes and Rust-shaped physical boundary.
//!
//! Token parsing and macro routing live in the `mini-linq` proc-macro crate.
//! This crate accepts typed IR values; their semantic well-formedness belongs
//! to the corresponding `compile_*` pass contracts.

mod cq_to_relational;
mod pass_contracts;
mod passes;
mod physical;
mod pipeline;
mod pull;
mod relational_to_indexes;
pub use cq_to_relational::{
    GyoRule, JoinPlanReport, JoinPolicy, JoinTree, PlanningOptions, Rule as JoinRule,
};

pub use passes::compile_physical;
pub use passes::{
    check_cq_plan, compile_cq, compile_cq_derivation, compile_cq_planned, compile_cq_with_policy,
    compile_index_requirements, compile_materialized, compile_materialized_with,
    compile_relational_plan, compile_result_indexes, explain_cq_plan,
};
pub use physical::access::{
    compile_exact_keys, compile_hash_witnesses, compile_nested_witnesses, compile_prune_rows,
    compile_scan_rows, compile_zone_keys,
};
pub use physical::direct::{PhysicalOptions, check_physical};
pub use physical::options::{AccessOptions, ProviderOptions};
pub use physical::providers::{
    compile_block_provider, compile_hash_provider, compile_sorted_provider,
};
pub use pipeline::{compile_collect_pipeline, compile_iterator_pipeline, compile_stream_pipeline};
pub use pull::compile_pull;
