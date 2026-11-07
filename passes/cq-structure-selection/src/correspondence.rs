//! Check recorded Structures decisions, independently of default selection.
use cq_lang_physical as physical;
use cq_lang_structures as structures;

/// Check provider discharge against the separately supplied Physical program.
/// Formal rule CK-Structures: Check independently derived storage rules.
pub fn check(source: &physical::Program, target: &structures::Program) -> syn::Result<()> {
    physical::contract::check(source)?;
    structures::contract::check(source, target)?;
    Ok(())
}
