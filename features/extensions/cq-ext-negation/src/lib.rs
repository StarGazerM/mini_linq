//! Stratified anti-membership contexts over materialized projected CQ answers.
//! `neg!{ S(x, _); depends { S -> Derived; Derived -> answer; } }`
//! optionally records host-relation dependencies for the stratification check.
//! Opaque Rust leaf expressions cannot reveal dependency graphs; callers must
//! declare those edges. Every nested query resumes the remaining pipeline.
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
//! Select `pipeline { surface = [cq_ext_negation::negation]; }` in
//! `mini_linq::compile!`. Other clauses stay opaque for subsequent surface passes.
//! Finite immutable inputs and Leibniz equality are required. Each projected
//! negative set is completed before the main query; opaque Rust in the main
//! query keeps its written chain, and evaluation counts are outside the meaning.
//! Generated Rust records GroupedAnswers membership hosts and a HashSet answer
//! boundary. Owned leaves are evaluated once; returned values must outlive them.
//!
//! Each crate's `expansion::check(source, context)` is an independently callable
//! internal structural relation exercised by its unit tests. It checks rule/hole
//! bijections, schema and leaf preservation, projections, capture links and fold
//! specifications where defined; it never calls `compile`. Context emission is
//! Rust realization, validated by differential execution and ownership/laziness
//! tests, without a claim of deciding arbitrary Rust semantic equality.
//!
//! Core blocks forward through this surface.
//! For multiple rules, owned negatives expand per rule with shared leaves and fresh hosts before core union sugar, so opaque extension tokens never undergo another language's renaming.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Rust keeps a branch ancestry tree: union siblings unordered, fallback ordered;
//! each arm retains prefix Before facts and fallback anti steps. Rust `||` keeps
//! its existing recognition. Only differing literals on one interface column
//! prove union arms disjoint; no data keys or constraints are assumed.
//!
mod expansion;

#[proc_macro]
pub fn negation(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_ext_support::surface(input.into(), |tokens, pipeline| {
        let quoted = cq_ext_support::quotation(tokens, "negation")?;
        let owned = quoted.is_some();
        if !owned && !cq_ext_support::is_cq(tokens) {
            return Ok(None);
        }
        let source: cq_ext_support::Rules = syn::parse2(quoted.unwrap_or_else(|| tokens.clone()))?;
        if !owned
            && !source.queries().any(|query| {
                query.body.iter().any(|item| {
            matches!(item, cq_lang_cq::BodyItem::Extension { name, .. } if name == "neg")
        })
            })
        {
            return Ok(None);
        }
        let plan = if source.rest.is_empty() {
            let plan = expansion::compile(&source.first)?;
            expansion::check(&source.first, &plan)?;
            plan
        } else {
            expansion::compile_rules(&source)?
        };
        expansion::render(&plan, pipeline).map(Some)
    })
    .into()
}
