//! Yannakakis scheduling over a supplied, validated join tree.
//! Each command uses the checked rule kernel; no relation is executed here.
use super::{JoinTree, Rule, plan::apply, rules::JoinState};

pub(super) fn reduce(
    tree: &JoinTree,
    state: &mut JoinState<'_>,
    derivation: &mut Vec<Rule>,
) -> syn::Result<()> {
    let mut breadth_first = vec![tree.root()];
    let mut i = 0;
    while i < breadth_first.len() {
        breadth_first.extend(tree.children(breadth_first[i]));
        i += 1;
    }
    for &child in breadth_first.iter().skip(1).rev() {
        apply(state, derivation, Rule::Up { child })?;
    }
    for &child in breadth_first.iter().skip(1) {
        apply(state, derivation, Rule::Down { child })?;
    }
    Ok(())
}
