//! Exact written-order RA for partial Rust reference executions.
//! Both the driver tests and the generated protocol pass use this public-API code.
pub fn written(source: &cq_lang_cq::Program) -> syn::Result<cq_lang_ra::Program> {
    let normal = cq_lang_cq::normal::normalize(source)?;
    let valuation = cq_order::compile(&normal, &cq_order::Options::default())?;
    cq_elaborate::compile_written(&valuation)
}
