//! Group-by contexts over the set of rows produced by one core hole.
//! `aggregate! { by (d); n=count(); s=sum(a); lo=min(a); hi=max(a); struct Q; ... }`
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
//! Select `pipeline { surface = [cq_ext_aggregate::aggregate]; }` inside
//! `mini_linq::compile!`. Unowned quotations continue unchanged. The row query
//! resumes the remaining pipeline. Its row stream is deduplicated before folding,
//! so distinct witnesses count once even with a different downstream printer.
//! Generated Rust records a HashSet witness boundary, HashMap groups and
//! GroupedAnswers output. Dedup retains the row stream's first-occurrence order.
//! Authored leaf inputs are evaluated once in the fold context and borrowed by
//! its row query; key and measure clones close their lifetimes before return.
//! Finite immutable hosts, Leibniz equality and pure Rust expressions are premises.
//! Rust arithmetic/order and Clone laws are trusted; overflow denotes bottom.
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
    cq_ext_support::surface(input.into(), |tokens, pipeline| {
        let Some(tokens) = cq_ext_support::quotation(tokens, "aggregate")? else {
            return Ok(None);
        };
        let source = syn::parse2(tokens)?;
        let plan = expansion::compile(&source)?;
        expansion::check(&source, &plan)?;
        expansion::render(&plan, pipeline).map(Some)
    })
    .into()
}
