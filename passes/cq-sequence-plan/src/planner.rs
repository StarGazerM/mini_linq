use cq_lang_sequence::{Catalog, Evidence, Machine, Program};

use crate::Planned;

/// A Sequence advice generator, independent of the target's macro adapter.
///
/// Implementations return checked advice: replaying `Planned::trace` over the
/// input stack must reproduce its distinct Physical, Structures and Sequence programs.
/// Authored decisions retain precedence, and key-dependent rewrites require
/// explicitly trusted authored key premises.
/// A target-owned ordinary pass supplies its machine and catalog, then forwards
/// the resulting program through the checked pipeline.
pub trait Planner {
    fn plan(
        &self,
        physical: &cq_lang_sequence::structures::physical::Program,
        structures: &cq_lang_sequence::structures::Program,
        program: &Program,
        machine: &Machine,
        catalog: &Catalog,
        evidence: &Evidence,
    ) -> syn::Result<Planned>;
}
