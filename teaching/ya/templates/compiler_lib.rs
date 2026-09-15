//! One selected compiler route; every implementation is a real source module.
mod analysis;
mod cq_to_relational;
mod physical;
mod pipeline;
mod pull;
mod relational_to_indexes;

pub use cq_to_relational::graph::{compile_cq_graph, explain_cq_graph};
pub use cq_to_relational::{
    GyoRule, JoinPlanReport, JoinPolicy, JoinTree, PlanningOptions, Rule as JoinRule,
};
pub use physical::materialized::{
    compile_materialized, compile_materialized_callable, compile_materialized_query,
    compile_materialized_with,
};
pub use pipeline::{compile_collect_pipeline, compile_iterator_pipeline, compile_stream_pipeline};
pub use pull::compile_pull;
pub use relational_to_indexes::result::compile_result_indexes;
