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
//! The default order is `plan, ix, lower, select, represent, stage, sequence, grade, print`.
//! Physical rewrites run before `select`; Structures rewrites run before
//! `represent`. Representations remain tuples unless `represent(positions)` is
//! requested. Staging preserves built-probe lookahead by default;
//! `stage(batch(64))` also batches total scan predicates before partial Rust.
//! Sequence lowering runs after `stage`, followed by optional `decompose`
//! rewrites, `sequential` grading, and `translate_pull`. This host-only path
//! returns a pull iterator with optional runtime profiling counters.
//! An explicit Structures printer such as `own_push::push` replaces the default
//! Sequence target; custom Sequence terminals select `sequence` and `grade` too.
//! ```
//! let rows = [(1_i32,), (1,), (2,)];
//! let answers: Vec<_> = mini_linq::compile! {
//!     pipeline {
//!         plan = mini_linq::passes::plan;
//!         print = mini_linq::passes::translate_pull;
//!     }
//!     struct Q;
//!     relation R(x: i32) = (rows);
//!     answer(x) :- R(x);
//! }.map(|(x,)| *x).collect();
//! assert_eq!(answers, vec![1, 2]);
//! ```

//! Core body blocks `{ B1 } | { B2 }` union answers and `{ B1 } else { B2 }`
//! select the first nonempty arm. Two nonempty branches are required; nesting
//! handles mixed operators. Common interface names escape; other binders stay
//! local. Join distributes over union, and fallback is `B1 union (B2 anti_F B1)`.
//! The complete IR records hoisting or per-row fusion, canon/witness decisions,
//! negative references and Rust source paths with inherited Before facts.
//!
pub use mini_linq_macros::{compile, mini_linq, relation_loaders, workload_query};

/// Core default passes. Customization is an external pass or an authored IR.
pub mod passes {
    pub use mini_linq_macros::{
        index_demands, lower, plan, represent, select, sequence, sequential, stage, translate_pull,
    };
}

#[cfg(doctest)]
mod positional_misuse;
