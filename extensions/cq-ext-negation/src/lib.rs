//! Stratified anti-membership contexts over materialized projected CQ answers.
//! `neg!{ S(x, _); depends { S -> Derived; Derived -> answer; } }`
//! optionally records host-relation dependencies for the stratification check.
//! Opaque Rust leaf expressions cannot reveal dependency graphs; callers must
//! declare those edges. Every hole continues at `mini_linq::negation_hole!`.
//!
//! Surface: Ordinary CQ with `neg!{ S(x, _) }` clauses
//!
//! Result and openings: Iterator; projected negative holes collected as `GroupedAnswers`, membership guards in the closing environment
//!
//! Negation arguments are earlier bound variables or `_`; repeated variables test
//! equal columns. A negated input may carry `neg!{ S(x,_); depends { S -> T;
//! T -> Base; } }`. All declared edges from negative clauses are combined, and a
//! path from any negated input to the current head is rejected. Positive dependency
//! cycles in a completed lower stratum are allowed. Dependency edges hidden in
//! arbitrary Rust are caller premises, not inferred from leaf expressions.
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
pub fn negation(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::stage(input.into(), None, |source: &cq_lang_cq::Program| {
        expansion::compile(source).map(|plan| expansion::render(&plan))
    })
    .into()
}
