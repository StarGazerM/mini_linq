//! Checked RA′ → RA′ rewrites that remove planned semijoins and expansions
//! already witnessed by exact named-result lineage. They are not part of VO →
//! RA′ lowering: lowering realizes every recorded reduction and expansion, and
//! only planners that opt in (Hypergraph, Transfer, LE) apply these rewrites
//! afterwards. Every rewrite is checked with [`ra::check_rewrite`].

use cq_lang_ra as ra;

mod reductions;

/// Simplify already elaborated reductions using exact named-result lineage.
///
/// In either operand of a semijoin or natural join, `R semijoin T`
/// may be replaced by `R` when the other operand is contained in `T` on
/// every column shared by `R` and `T`. Right-side support pruning takes
/// precedence; a left-side bypass must be justified by the surviving RHS.
/// This is a local pass, not a maximal fixed-point guarantee.
/// Containment follows only the same
/// named result through projections, selections, semijoin left inputs and
/// natural-join inputs; it never identifies independent projected witnesses
/// or infers facts from types, data, predicates or source occurrence numbers.
/// A shared restricted support is bypassed only when every bypassed result
/// becomes unreachable. Otherwise its consumers keep the same materialized
/// row witness, preserving independent projections' correlation obligations.
///
/// Opaque Rust operators and negative-operand ancestry are not rewritten.
/// Reductions containing Rust are not bypassed, and all remaining operator
/// payloads, result names and block dependencies are retained. Only definitions
/// made unreachable by these identities are pruned. The result is checked with
/// [`ra::check_rewrite`]; the VO-to-RA named-reduction obligation of
/// `cq_elaborate::check` applies to the lowered program, not to this rewrite.
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
