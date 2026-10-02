//! UCQ surface contexts. Each rule resumes the caller's remaining pipeline.
//! A concrete HashSet in the generated Rust filters all disjuncts, lazily.
//!
//! Surface: `union! { struct U; relation A(x:i32); relation B(x:i32);
//! answer(x) :- A(x); answer(x) :- B(x); }`.
//! Select it with `pipeline { surface = [cq_ext_union::union]; }` inside
//! `mini_linq::compile!`. A direct pass call supplies an explicit remaining queue.
//!
//! Result: an iterator with one nested query per disjunct and a shared set
//! boundary. Unowned clauses are forwarded unchanged; leaves are evaluated once.
//!
//! Stacking lists `cq_ext_union::union, cq_ext_negation::negation` in that order.
//! The crates share no clause knowledge; negation expands each disjunct later.
//! Inputs are finite immutable sets under Leibniz equality. Rust guards and binds
//! are opaque partial functions pinned to the written chain. Evaluation counts
//! are outside the denotation. Without owned leaves, disjuncts are lazy; owned
//! leaves require collection before their context ends. Answers must be Clone,
//! Eq and Hash; cloning a borrowed answer does not clone its referent.
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
pub fn union(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    cq_ext_support::surface(input.into(), |tokens, pipeline| {
        let Some(tokens) = cq_ext_support::quotation(tokens, "union")? else {
            return Ok(None);
        };
        let source = syn::parse2(tokens)?;
        let plan = expansion::compile(&source)?;
        expansion::check(&source, &plan)?;
        expansion::render(&plan, pipeline).map(Some)
    })
    .into()
}
