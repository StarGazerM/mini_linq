//! Group-by contexts over the set of rows produced by one core hole.
//! `by (d); n = count(); s = sum(a); lo = min(a); hi = max(a); struct Q; ...`
//! The CQ head specifies the distinct rows being aggregated (include an ID to
//! count distinct sales with equal amounts). Output is `GroupedAnswers` with
//! group columns leading and measure columns in specification order.
//!
//! Surface: `by(d); n=count(); s=sum(a); lo=min(a); hi=max(a);` followed by a CQ `rows(d,id,a) :- Sale(d,id,a);`
//!
//! Result and openings: `GroupedAnswers<(d,), (n,s,lo,hi)>`; distinct row output folded outside the core and reusable as a leading-key host
//!
//! Aggregation is over a set of the CQ head rows, rather than bags of base-table
//! occurrences. Include an identity column in `rows` to count different sales with
//! the same amount. Group keys must be distinct head columns, and measure operands
//! must be head columns. At least one key and measure are required. `count` returns
//! `usize`; other measures use the column's Rust arithmetic/order. Empty input has
//! no groups. The key tuple leads the host layout; the measure tuple follows in
//! written order. Group keys and retained measure values are cloned explicitly by
//! the extension fold; the core never copies logical bindings.
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
pub fn aggregate(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_stage::stage(input.into(), None, |source: &expansion::Source| {
        expansion::compile(source).map(|plan| expansion::render(&plan))
    })
    .into()
}
