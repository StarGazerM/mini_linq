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

pub use passes::{
    check_cq_plan, compile_composed, compile_cq, compile_cq_derivation, compile_cq_planned,
    compile_cq_with_policy, compile_deferred_indexes, compile_index_requirements,
    compile_materialized, compile_materialized_with, compile_relational_plan,
    compile_result_indexes, explain_cq_plan,
};
pub use physical::composed::{
    BlockChoices, BlockOptions, BlockSemijoin, PhysicalOptions, check_blocked_resources,
    check_blocked_with, check_physical, check_zonemap_resources, compile_blocked,
    compile_blocked_with, compile_physical, compile_zonemap,
};
pub use physical::joins::{compile_hash_join, compile_nested_loop_join};
pub use physical::options::{ProviderOptions, RecipeOptions};
pub use physical::providers::{
    compile_block_provider, compile_hash_provider, compile_sorted_provider,
};
pub use physical::semijoin::{compile_exact_semijoin, compile_zone_semijoin};
pub use pipeline::{compile_collect_pipeline, compile_iterator_pipeline};
pub use pull::compile_pull;
