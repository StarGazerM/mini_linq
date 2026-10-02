//! CQ compilation through an explicit, checked pass pipeline.
//!
//! `mini_linq! { <CQ> }` uses deterministic defaults. `compile! { pipeline {
//! ... } <CQ> }` selects external pass paths and forwards their argument tokens.
//! See [`cq_stage`](https://docs.rs/cq-stage) for the protocol and decision fields.
//! Queries denote `bottom + Set`: a panic aborts evaluation; relational equality
//! requires Leibniz-admissible Eq and ordered reads require total, consistent Ord.
//! Rust binds and guards preserve their written ancestry and free variables.
//!
//! Select pass paths in named phases; omitted phases use the defaults:
//! ```
//! let rows = [(1_i32,), (1,), (2,)];
//! let answers: Vec<_> = mini_linq::compile! {
//!     pipeline {
//!         plan = mini_linq::passes::plan_written;
//!         print = mini_linq::passes::print_pull;
//!     }
//!     struct Q;
//!     relation R(x: i32) = (rows);
//!     answer(x) :- R(x);
//! }.map(|(x,)| *x).collect();
//! assert_eq!(answers, vec![1, 2]);
//! ```

pub use mini_linq_macros::{compile, mini_linq, relation_loaders, workload_query};

/// Core default passes. Customization is an external pass or an authored IR.
pub mod passes {
    pub use mini_linq_macros::{index_demands, lower, plan_written, print_pull, select};
}
