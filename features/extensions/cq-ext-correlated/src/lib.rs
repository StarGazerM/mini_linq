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
//! Correlated results use a concrete HashSet boundary over answer values,
//! including equal computed heads from different valuations.
//! Input hosts are borrowed into the context before the generated move closures;
//! explicit leaf expressions are evaluated once, even across inner invocations.
//! Select `pipeline { surface = [cq_ext_correlated::correlated]; }` inside
//! `mini_linq::compile!`. Unowned clauses continue to later surface passes; every
//! per-row quotation resumes the caller's remaining pipeline, including its
//! arguments. Inputs are finite and immutable; equality is Leibniz equality and
//! opaque Rust is a partial function of shared inputs. The outer Rust chain
//! retains subqueries at their written positions. An exists query may stop once
//! a witness is found; scalar checks consume its complete distinct answer set.
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
    cq_ext_support::surface(input.into(), |tokens, pipeline| {
        let quoted = cq_ext_support::quotation(tokens, "correlated")?;
        let owned = quoted.is_some();
        if !owned && !cq_ext_support::is_cq(tokens) {
            return Ok(None);
        }
        let source: cq_lang_cq::Program = syn::parse2(quoted.unwrap_or_else(|| tokens.clone()))?;
        if !owned && !source.query.body.iter().any(|item| {
            matches!(item, cq_lang_cq::BodyItem::Extension { name, .. } if name == "exists" || name == "scalar")
        }) {
            return Ok(None);
        }
        let plan = expansion::compile(&source, pipeline)?;
        expansion::check(&source, &plan, pipeline)?;
        expansion::render(&plan, pipeline).map(Some)
    })
    .into()
}
