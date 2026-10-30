use cq_lang_sequence::{Catalog, Evidence, Machine, Program};

use crate::Planned;

/// A Sequence advice generator, independent of the target's macro adapter.
///
/// Implementations return checked advice: replaying `Planned::trace` over the
/// input must reproduce `Planned::program`. Authored decisions retain precedence,
/// and key-dependent rewrites are conditional on the author's asserted keys.
/// A target-owned ordinary pass supplies its machine and catalog, then forwards
/// the resulting program through the checked pipeline.
pub trait Planner {
    fn plan(
        &self,
        program: &Program,
        machine: &Machine,
        catalog: &Catalog,
        evidence: &Evidence,
    ) -> syn::Result<Planned>;
}
