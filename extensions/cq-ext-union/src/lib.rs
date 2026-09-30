//! UCQ contexts. Each rule is a hole forwarded to `mini_linq::union_hole!`.
//! The caller binds that slot to another surface extension or the CQ entry.
//! One caller-selected distinct state filters all disjuncts, lazily.
//!
//! Surface: One CQ header followed by `answer(x) :- A(x); answer(x) :- B(x);`
//!
//! Result and openings: Iterator; one disjunct per hole, shared `distinct_state!`, pass-through surface and leaf/environment bindings
//!
//! Stacking: bind `union_hole` to `cq_ext_negation::negation` and `negation_hole` to the CQ entry; the crates share no clause knowledge and forward unrecognized clauses.
//!
//! Each crate's `expansion::check(source, context)` is an independently callable
//! internal structural relation exercised by its unit tests. It checks rule/hole
//! bijections, schema and leaf preservation, projections, capture links and fold
//! specifications where defined; it never calls `compile`. Context emission is
//! Rust realization, validated by differential execution and ownership/laziness
//! tests, without a claim of deciding arbitrary Rust equivalence.
//!
mod expansion;

#[proc_macro]
pub fn union(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::stage(input.into(), None, |source: &cq_ext_support::Rules| {
        expansion::compile(source).map(|plan| expansion::render(&plan))
    })
    .into()
}
