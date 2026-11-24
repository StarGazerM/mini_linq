//! An external, nonterminal language extending the complete Physical program.
//!
//! Text is `physical { ... } trace {}`. The empty annotation adds no execution
//! or representation decision; the base records every operator, mode and key.
//! Its denotation is the Physical Set denotation under Leibniz equality and
//! opaque, pure, total Rust. Rust placement follows free-variable binders only.
//! The local contract checks Physical; rewrites use Physical correspondence.
//! This language can be passed to [`cq_stage::pass`] or [`cq_stage::rewrite`].
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
    /// Check the complete base program; the annotation is empty by construction.
    pub fn check(program: &super::Program) -> syn::Result<super::physical::Analysis> {
        super::physical::contract::check(&program.physical)
    }
}

/// Trace rewrites preserve Physical's RA erasure and admissible decisions.
pub fn check_rewrite(source: &Program, target: &Program) -> syn::Result<()> {
    physical::check_rewrite(&source.physical, &target.physical)
}

impl cq_stage::Language for Program {
    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }

    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite(source, target)
    }
}
