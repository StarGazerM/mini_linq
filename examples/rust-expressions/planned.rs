//! Run with: cargo run -p mini-linq --example rust-expressions-planned --features compiled-workloads
mod wiring {
    #[allow(unused_imports)]
    pub use ::mini_linq::{
        collect_pipeline, graph_query, iterator_pipeline, optimized_query, prefix_query, pull,
        pull_set,
    };
    pub use ::mini_linq::{
        hash_materialized as choose_realization, result_indexes as choose_indexes,
    };
}
use mini_linq::optimized_query as query;
use wiring as mini_linq;

mod gallery;
mod plans;

fn main() {
    println!("Rust expressions through the planned materialized compiler:");
    gallery::run();
    plans::run();
}
