//! CQ compilation through an explicit, checked pass pipeline.
//!
//! `mini_linq! { <CQ> }` uses deterministic defaults. `compile! { pipeline {
//! ... } <CQ> }` selects external pass paths and forwards their argument tokens.
//! `compile! { observe { boundary = grade; stage = ::core::stringify; }
//! pipeline { ... } <CQ> }` observes the checked output of the selected queue
//! prefix without executing the query. Boundaries are `cq`, `vo`,
//! `relational_plan`, `index_requirements`, `physical`, `structures`,
//! `represent`, `stage`, `sequence`, `decompose`, `grade`, and `final_rust`.
//! VO exposes the shared default planner's valuation prefix; custom planners
//! expose their own internal boundaries. Observation preserves pass arguments
//! and preceding rewrites; terminal Rust is captured after translation.
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
//! Sequence lowering runs after `stage`, followed by robust Lookup/Expand
//! planning, `sequential` grading, and `translate_pull`. Authored `decompose`
//! advice replaces that default; this host-only path returns a pull iterator.
//! A Structures-consuming terminal declares `print_input = structures;`.
//! Sequence terminals retain the configured/default Sequence phases regardless
//! of their macro names; aliases do not change a pipeline's language endpoints.
//! To fill unauthored Sequence decisions, end `decompose` with
//! `mini_linq::passes::plan(target::plan)` (for Sequential, the target is
//! `mini_linq::passes::plan_sequential`). Earlier advice retains precedence;
//! the target-owned macro supplies its exported machine and catalog.
//! With a complete Sequence program as input, `compile!` starts at Sequence
//! decomposition instead: `plan = ::mini_linq::passes::plan` preserves its
//! relational order, while `plan = ::other::custom_plan` selects an ordinary
//! checked Sequence pass implementing `cq_sequence_plan::Planner`. That pass
//! supplies its machine/catalog and returns a program with a replayable advice
//! trace; evidence is planning advice, never a semantic premise. `decompose`
//! advice runs before planning, then `grade` and `print` consume the result.
//! Earlier lowering phases are rejected for complete Sequence input.
//! With CQ input, `plan` retains its CQ-to-RA ordering/reduction meaning;
//! `plan(order(R, S), no_reduce)` selects an explicit checked relation order.
//! `plan(target::plan)` remains available in either Sequence planning position.
//! Sequence planning preserves authored relational order; performance plans
//! are authored through checked advice. Explicit ordering, selection,
//! Physical/Structures, realization and staging decisions remain recorded.
//! Target adapters use `cq_sequence_plan::decompose` to preserve authored order.
//! `plan(preserve_order)` and `plan_sequential(preserve_order)` retain the explicit
//! authored policy; it is also the default.
//! A Structures printer declares `print_input = structures` to replace the
//! default Sequence target; its path or alias does not select a language.
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
//! [`relation_loaders!`] generates plain typed source readers.
//! [`interned_relation_loaders!`] explicitly opts the same declarations into
//! load-time string interning through `cq_load::InternedField`, adding encoded
//! loader methods without changing query syntax or the plain macro expansion.
//!
pub use mini_linq_macros::{
    compile, interned_relation_loaders, mini_linq, relation_loaders, workload_query,
};

/// Shared checked default planning composition and its authored options.
pub use cq_planning as planning;

/// Core default passes. Customization is an external pass or an authored IR.
pub mod passes {
    pub use mini_linq_macros::{
        index_demands, lower, observe_cq, plan, plan_sequential, prefix_valuation, represent,
        select, sequence, sequential, stage, translate_pull,
    };
}

#[cfg(doctest)]
mod positional_misuse;
