//! Add a Trace annotation without altering the Physical prefix.
pub fn compile(_: &cq_lang_physical::Section) -> syn::Result<cq_lang_trace::Section> {
    syn::parse_str("trace {}")
}
