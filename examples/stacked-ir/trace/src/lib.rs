//! A new language owns only its section; it does not know Physical or Structures.
pub use cq_syntax::Staged;
pub mod kw {
    syn::custom_keyword!(trace);
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub trace: kw::trace,
    pub brace: syn::token::Brace,
}
impl syn::parse::Parse for Section {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let trace = input.parse()?;
        let content;
        let brace = syn::braced!(content in input);
        if !content.is_empty() {
            return Err(content.error("trace is an empty identity section"));
        }
        Ok(Self { trace, brace })
    }
}
impl quote::ToTokens for Section {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.trace.to_tokens(tokens);
        self.brace.surround(tokens, |_| {});
    }
}
