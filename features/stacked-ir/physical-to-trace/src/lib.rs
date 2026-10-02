//! Physical → Trace: add an empty annotation, preserving every written decision.
//!
//! [`check`] compares the entire Physical AST after checking both contracts;
//! it does not re-run [`compile`]. Identifiers and embedded Rust are cloned
//! with their spans, keeping the caller's hygiene across this boundary.

pub fn compile(source: &cq_lang_physical::Program) -> syn::Result<cq_lang_trace::Program> {
    let target = cq_lang_trace::Program {
        physical: source.clone(),
        trace: syn::parse_str("trace {}")?,
    };
    check(source, &target)?;
    Ok(target)
}

/// Require an exact Physical embedding, including modes, keys and Rust steps.
pub fn check(
    source: &cq_lang_physical::Program,
    target: &cq_lang_trace::Program,
) -> syn::Result<()> {
    cq_lang_physical::contract::check(source)?;
    cq_lang_trace::contract::check(target)?;
    if source != &target.physical {
        return Err(syn::Error::new_spanned(
            target,
            "Physical-to-Trace changes the written Physical program",
        ));
    }
    Ok(())
}
