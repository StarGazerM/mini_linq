//! Erase a Trace annotation, preserving the Physical section exactly.
pub fn compile(
    source: &cq_lang_trace::Staged<cq_lang_physical::Section, cq_lang_trace::Section>,
) -> syn::Result<cq_lang_physical::Section> {
    Ok(source.prefix.clone())
}
