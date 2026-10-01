//! Linear, finite active-domain recursion via semi-naive delta holes.
//! The surface is several rules with a declared result schema; heads copy
//! positive atom columns (`*x` or `(*x).clone()`). Every recursive rule has
//! exactly one result atom. No core recursion operator is introduced.
//! Optional `observe = (|round, delta_len| ...);` instruments nonempty rounds.
//!
//! Surface: Declared result, seed rules, and linear recursive rules: `reach(*x,*y) :- Edge(x,y); reach(*x,*z) :- reach(x,y), Edge(y,z);`
//!
//! Result and openings: Iterator of owned fixpoint rows; each round reads only delta `Answers` through an ordinary leaf
//!
//! Recursion supports several seeds and several linear steps. Each step has exactly
//! one recursive atom. Heads are projections of positive atom columns using `*x`
//! or `(*x).clone()`; arithmetic/function-generating heads and nonlinear recursion
//! are rejected. This finite active domain, immutable hosts and pure expressions
//! are the fixpoint premises. Optional `observe=(|round, delta_len| ...);` before
//! the CQ header observes nonempty rounds. Distinctness across seeds and rounds
//! uses the caller's distinct state, while delta is a canonical `Answers` host.
//!
//! Each crate's `expansion::check(source, context)` is an independently callable
//! internal structural relation exercised by its unit tests. It checks rule/hole
//! bijections, schema and leaf preservation, projections, capture links and fold
//! specifications where defined; it never calls `compile`. Context emission is
//! Rust realization, validated by differential execution and ownership/laziness
//! tests, without a claim of deciding arbitrary Rust semantic equality.
//!
mod expansion;

#[proc_macro]
pub fn recursion(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::stage(input.into(), None, |source: &expansion::Source| {
        expansion::compile(source).map(|plan| expansion::render(&plan))
    })
    .into()
}
