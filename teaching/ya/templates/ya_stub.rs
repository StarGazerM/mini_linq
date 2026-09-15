//! Exercise 1: schedule both reductions over the supplied checked join tree.
use super::{JoinTree, Rule, rules::JoinState};

pub(super) fn reduce(
    tree: &JoinTree,
    state: &mut JoinState<'_>,
    derivation: &mut Vec<Rule>,
) -> syn::Result<()> {
    let _ = (tree, state, derivation);
    // Use graph::apply with Rule::Up and Rule::Down. The caller owns Freeze.
    Err(super::error("YA scheduling exercise"))
}
