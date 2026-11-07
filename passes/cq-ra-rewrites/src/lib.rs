//! Checked RA′ → RA′ rewrites that remove planned semijoins and expansions
//! already witnessed by exact named-result lineage. They are not part of VO →
//! RA′ lowering: lowering realizes every recorded reduction and expansion, and
//! only planners that opt in (Hypergraph, Transfer, LE) apply these rewrites
//! afterwards. Every rewrite is checked with [`ra::check_rewrite`].

use cq_lang_ra as ra;

mod reductions;

/// Simplify already elaborated reductions using exact named-result lineage.
///
/// Apply direct-consumer RA-Absorb to a fixed point on the current RA DAG.
/// A reduction `P semijoin L` is bypassed only when every use is an operand
/// of a semijoin or natural join whose other operand witnesses the same named
/// `L`, retaining every shared column. All uses are bypassed together.
/// No intervening operator is crossed: un-pushing a filter is a separate cost decision.
/// The independent checker accepts the broader legal key-retaining pipe law.
///
/// The witness lineage may follow projections, selections, semijoin left
/// inputs and either natural-join input, retaining all constrained columns.
/// Independent projected witnesses are never identified. No type, data,
/// source occurrence or former VO position participates in this decision.
///
/// Opaque Rust operators and negative-operand ancestry are not rewritten.
/// Reductions containing Rust are not bypassed, and all remaining operator
/// payloads, result names and block dependencies are retained. Only definitions
/// made unreachable by these identities are pruned. The result is checked with
/// [`ra::check_rewrite`]; the VO-to-RA named-reduction obligation of
/// `cq_relational_lowering::check` applies to the lowered program, not to this rewrite.
pub fn simplify_reductions(source: &ra::Program) -> syn::Result<ra::Program> {
    reductions::simplify(source)
}

/// Eliminate expansions already witnessed by an earlier named-result lookup.
///
/// In positive relational scopes, an expansion whose introduced columns have
/// no observer is replaced by a semijoin. Under canon this preserves answers,
/// not intermediate bag multiplicities. Exact named-result lineage can then
/// remove a semijoin whose support is already witnessed by its left input.
/// This explicit pass is used by LE, not by the default or Hypergraph pipeline.
///
/// The identity does not cross Rust, negative, or block boundaries. Only
/// definitions made unreachable by checked identities are pruned. Each local
/// rewrite is checked with [`ra::check_rewrite`].
pub fn simplify_expansions(source: &ra::Program) -> syn::Result<ra::Program> {
    reductions::eliminate_expansions(source.clone())
}
