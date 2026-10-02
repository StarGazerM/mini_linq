//! Extend Physical with an identity annotation.
pub fn compile(source: &cq_lang_physical::Program) -> syn::Result<cq_lang_trace::Program> {
    let target = cq_lang_trace::Program {
        physical: source.clone(),
        trace: syn::parse_str("trace {}")?,
    };
    cq_lang_trace::contract::check(&target)?;
    Ok(target)
}
