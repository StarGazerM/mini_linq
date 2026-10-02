//! Trace → Physical: erase only the empty identity annotation.
//!
//! The independent [`check`] requires the complete Physical AST unchanged,
//! including all Rust payloads, their order, declarations and decisions.

pub fn compile(source: &cq_lang_trace::Program) -> syn::Result<cq_lang_physical::Program> {
    let target = source.physical.clone();
    check(source, &target)?;
    Ok(target)
}

/// Check both contracts and exact erasure, without compiling the source.
pub fn check(
    source: &cq_lang_trace::Program,
    target: &cq_lang_physical::Program,
) -> syn::Result<()> {
    cq_lang_trace::contract::check(source)?;
    cq_lang_physical::contract::check(target)?;
    if &source.physical != target {
        return Err(syn::Error::new_spanned(
            target,
            "Trace-to-Physical changes the written Physical program",
        ));
    }
    Ok(())
}
