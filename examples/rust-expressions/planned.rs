//! Run with: cargo run -p mini-linq --example rust-expressions-planned --features compiled-workloads
mod wiring {
    #[allow(unused_imports)]
    pub use ::mini_linq::{
        access_to_distinct_iter, access_to_set, choose_graph_or_prefix, cq_to_ra_clause_prefix,
        cq_to_ra_hypergraph, pipeline_to_distinct_iter, pipeline_to_set,
    };
    pub use ::mini_linq::{
        materialized_hash_program as choose_relation_lowering,
        result_index_demands as choose_index_policy,
    };
}
use mini_linq::choose_graph_or_prefix as query;
use wiring as mini_linq;

mod gallery;
mod plans;

fn main() {
    println!("Rust expressions through the planned materialized compiler:");
    gallery::run();
    plans::run();
}
