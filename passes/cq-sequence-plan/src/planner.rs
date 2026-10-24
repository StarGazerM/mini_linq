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

/// The deterministic, rule-based planner used by the default target adapters.
#[derive(Clone, Copy, Debug, Default)]
pub struct RuleBased;

impl Planner for RuleBased {
    fn plan(
        &self,
        program: &Program,
        machine: &Machine,
        catalog: &Catalog,
        evidence: &Evidence,
    ) -> syn::Result<Planned> {
        if evidence == &program.evidence {
            return crate::plan(program, machine, catalog);
        }
        let recorded = cq_sequence_rewrite_passes::record_evidence(program, evidence)?;
        let changed = recorded != *program;
        let mut planned = crate::plan(&recorded, machine, catalog)?;
        if changed {
            planned
                .trace
                .insert(0, crate::Advice::Evidence(evidence.clone()));
        }
        Ok(planned)
    }
}
