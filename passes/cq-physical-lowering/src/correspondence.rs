//! Logical erasure and admissibility obligations, independent of lowering decisions.

use cq_lang_ix as ix;
use cq_lang_physical as physical;

/// Check source/target contracts, logical erasure, IX demands, and recorded decisions.
/// Materialization is free whenever the language premises hold.
/// Formal rule CK-Physical: Check local contracts, RA erasure, IX demands and decision admissibility.
pub fn check(source: &ix::Program, target: &physical::Program) -> syn::Result<()> {
    ix::contract::check(source)?;
    physical::contract::check(target)?;
    let erased = super::erase(target);
    if erased != source.ra {
        let construct = source
            .ra
            .plan
            .definitions
            .iter()
            .zip(&erased.plan.definitions)
            .find(|(a, b)| a != b)
            .map(|(a, _)| &a.result)
            .unwrap_or(&target.program.returned);
        return Err(syn::Error::new_spanned(
            construct,
            format!("result `{construct}`: physical erasure differs from the source RA program"),
        ));
    }
    super::check_demands(source, target)?;
    Ok(())
}
