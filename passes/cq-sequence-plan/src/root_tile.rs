//! Root tiles sized from the estimated root of a GPU store dispatch.
//!
//! A dispatch's grid is its root-tile count. A root estimated to span more
//! than one but fewer than `root_tiles` full tiles would otherwise launch
//! only a few blocks (a literal-restricted Part root of 1,451 rows becomes six
//! 256-row tiles). GR-RootTile cuts such a root into about `root_tiles`
//! tiles. A root that fits one tile keeps the width, so GPU-DistributeExpansion
//! can slice its children across blocks; a large root keeps full tiles.
use cq_lang_sequence::{Analysis, Machine, Program, Term};

/// Formal rule GR-RootTile: A GPU store dispatch whose root estimate lies strictly between its width and width times the machine's root_tiles partitions its root into ceil(estimate / root_tiles) rows per tile; other dispatches keep the width.
pub(crate) fn tile(
    program: &Program,
    analysis: &Analysis,
    machine: &Machine,
    store: &syn::Ident,
    width: usize,
) -> usize {
    let Some(gpu) = &machine.gpu else {
        return width;
    };
    let Some(definition) = program
        .sequence
        .definitions
        .iter()
        .find(|definition| definition.name == *store)
    else {
        return width;
    };
    let Term::Store { inner, .. } = &definition.term else {
        return width;
    };
    let Some(root) = crate::retain::stream(program, inner, &mut Vec::new()) else {
        return width;
    };
    let result = analysis.results.get(&root).cloned().unwrap_or(root);
    let Some(estimate) = analysis
        .prefix_bounds
        .get(&result)
        .and_then(|bound| bound.estimate)
    else {
        return width;
    };
    let full = (width as u128).saturating_mul(gpu.root_tiles as u128);
    if gpu.root_tiles == 0 || estimate <= width as u128 || estimate >= full {
        return width;
    }
    estimate.div_ceil(gpu.root_tiles as u128) as usize
}

/// GR-RootTile for a target that authors its own widths: the planner's
/// estimate decides the tile of each authored partition.
pub fn authored_tile(
    physical: &cq_lang_sequence::structures::physical::Program,
    structures: &cq_lang_sequence::structures::Program,
    program: &Program,
    machine: &Machine,
    store: &syn::Ident,
    width: usize,
) -> syn::Result<usize> {
    let analysis = cq_lang_sequence::contract::Checker::default().check_shared(physical, structures, program)?;
    Ok(tile(program, &analysis, machine, store, width))
}
