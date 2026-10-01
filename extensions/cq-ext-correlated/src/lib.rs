//! Environment contexts: caller parameters, per-row `exists!{ CQ body }`, and
//! `scalar!{ v: Type = (default); select (expression); CQ body }`.
//! A scalar requires at most one distinct result; multiple results panic just
//! as an SQL scalar subquery errors. Inner occurrences of outer variables are
//! fresh binders constrained by equality guards in the closing environment.
//!
//! Surface: Caller values in guards, `exists!{ S(x,y), if *y > limit }`, or `scalar!{ v:i32=(0); select(*y); S(x,y) }`
//!
//! Result and openings: Iterator; inner holes evaluated per outer row through the environment interface
//!
//! Correlated bare occurrences of outer columns in inner atoms become fresh
//! binders plus equality guards, retaining relational correlation rather than
//! shadowing. Parameters from Rust follow normal borrowing/ownership rules. In
//! embedded expressions logical values have type `&T`, including scalar results:
//! use `if *v > limit`. A scalar uses its default only when empty and rejects more
//! than one distinct row by a runtime assertion, matching SQL scalar cardinality.
//! Its selected answers are collected through `Answers` before checking cardinality.
//! Final union, negation and correlated results use caller-selected distinct state
//! over answer values, including equal computed heads from different valuations.
//! Input hosts are borrowed into the context before the generated move closures;
//! explicit leaf expressions are evaluated once, even across inner invocations.
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
pub fn correlated(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::stage(input.into(), None, |source: &cq_lang_cq::Program| {
        expansion::compile(source).map(|plan| expansion::render(&plan))
    })
    .into()
}
