//! Check recorded Structures decisions, independently of default selection.
use cq_lang_physical as physical;
use cq_lang_structures as structures;

/// Check Physical preservation and discharge. Rust checks representation traits.
/// Formal rule CK-Structures: Check Physical preservation, independently derived storage rules.
pub fn check(source: &physical::Program, target: &structures::Program) -> syn::Result<()> {
    physical::contract::check(source)?;
    structures::contract::check(target)?;
    if source != &target.physical
        || quote::ToTokens::to_token_stream(source).to_string()
            != quote::ToTokens::to_token_stream(&target.physical).to_string()
    {
        return Err(syn::Error::new_spanned(
            &target.physical,
            "selection must preserve Physical AST and tokens exactly",
        ));
    }
    Ok(())
}
