//! Trace extends the complete Physical program with an identity annotation.
pub use cq_lang_physical as physical;
pub mod kw {
    syn::custom_keyword!(trace);
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trace {
    pub trace: kw::trace,
    pub brace: syn::token::Brace,
}
impl syn::parse::Parse for Trace {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let trace = input.parse()?;
        let content;
        let brace = syn::braced!(content in input);
        if !content.is_empty() {
            return Err(content.error("trace is an empty identity annotation"));
        }
        Ok(Self { trace, brace })
    }
}
impl quote::ToTokens for Trace {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.trace.to_tokens(tokens);
        self.brace.surround(tokens, |_| {});
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub physical: physical::Program,
    pub trace: Trace,
}
impl syn::parse::Parse for Program {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        Ok(Self {
            physical: input.parse()?,
            trace: input.parse()?,
        })
    }
}
impl quote::ToTokens for Program {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.physical.to_tokens(tokens);
        self.trace.to_tokens(tokens);
    }
}
pub mod contract {
    pub fn check(program: &super::Program) -> syn::Result<super::physical::Analysis> {
        super::physical::contract::check(&program.physical)
    }
}
