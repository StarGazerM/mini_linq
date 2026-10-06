//! Check recorded Structures decisions, independently of default selection.
use cq_lang_physical as physical;
use cq_lang_structures as structures;

/// Check Physical preservation and discharge. Rust checks representation traits.
/// Formal rule CK-Structures: Check Physical preservation, independently derived storage rules.
pub fn check(source: &physical::Program, target: &structures::Program) -> syn::Result<()> {
    physical::contract::check(source)?;
    structures::contract::check(target)?;
    physical::check_rewrite(source, &target.physical)?;
    Ok(())
}
