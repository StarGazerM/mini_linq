//! Discard the Trace annotation, preserving Physical execution.
pub fn compile(source: &cq_lang_trace::Program) -> syn::Result<cq_lang_physical::Program> {
    cq_lang_trace::contract::check(source)?;
    Ok(source.physical.clone())
}
