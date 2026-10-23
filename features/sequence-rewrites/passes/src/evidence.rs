//! Evidence advice is metadata, never a proof of a data property.

use cq_lang_sequence::{Evidence, Program};
use proc_macro2::TokenStream;

/// Record a complete `evidence { ... }` advice block. The same syntax prints in
/// planner traces and in complete Sequence programs.
pub fn evidence(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    record_evidence(source, &syn::parse2(arguments)?)
}

/// Append estimates without removing previously authored facts. Contradictory
/// facts remain recorded; consumers may use them only to rank checked plans.
/// Passing the already recorded metadata is idempotent, so an explicit planner
/// evidence parameter may borrow `program.evidence` without duplicating it.
// Formal rule PL-Evidence: Untrusted evidence changes planning advice, never the checked construction or its answers.
pub fn record_evidence(source: &Program, evidence: &Evidence) -> syn::Result<Program> {
    record_evidence_in(&mut Default::default(), source, evidence)
}

pub fn record_evidence_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    evidence: &Evidence,
) -> syn::Result<Program> {
    let mut target = source.clone();
    if target.evidence != *evidence {
        target.evidence.facts.extend(evidence.facts.iter().cloned());
    }
    super::checked_in(checker, source, target)
}
