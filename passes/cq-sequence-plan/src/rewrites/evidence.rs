//! Evidence records sourced planning advice and explicitly trusted authored keys.

use cq_lang_sequence::{Evidence, Program};
use proc_macro2::TokenStream;

/// Record a complete sourced evidence advice block without changing its origins.
/// The same syntax prints in planner traces and complete Sequence programs.
pub fn evidence(source: &Program, arguments: TokenStream) -> syn::Result<Program> {
    record_evidence(source, &syn::parse2(arguments)?)
}

/// Append sourced advice and trusted keys without removing prior facts or origins.
/// Estimates rank plans; semantic rewrites require explicitly trusted authored keys.
/// Passing the already recorded metadata is idempotent, so an explicit planner
/// evidence parameter may borrow `program.evidence` without duplicating it.
// Formal rule PL-Evidence: Sourced estimates and planning keys rank checked plans; only explicitly trusted authored keys supply semantic premises.
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
