//! Clause scheduling policy; legal transitions remain owned by the rule kernel.
use super::{error, rules::ClauseState};
use crate::JoinPolicy;

/// Choose among C-READY clauses. Written order uses the first legal clause;
/// other policies prefer a ready guard/binding, then connected atoms, then width.
pub(super) fn choose_clause(state: &ClauseState<'_>, policy: JoinPolicy) -> syn::Result<usize> {
    state
        .ready_clauses()
        .map(|(clause, facts)| {
            let rank = if policy == JoinPolicy::Written {
                (false, false, 0, clause)
            } else if let Some(arity) = facts.atom_arity {
                let shared = facts.provided.intersection(state.heading()).count()
                    + facts.literal_arguments.len();
                let width = if matches!(policy, JoinPolicy::GreedyWidth | JoinPolicy::Auto) {
                    state.heading().len() + arity - shared
                } else {
                    0
                };
                (true, shared == 0, width, clause)
            } else {
                (false, false, 0, clause)
            };
            (rank, clause)
        })
        .min_by_key(|(rank, _)| *rank)
        .map(|(_, clause)| clause)
        .ok_or_else(|| error("no pending clause satisfies C-READY"))
}
