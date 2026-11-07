/// Independently owned IRs for authored GPU Sequence fixtures.
#[derive(Clone)]
pub(crate) struct Stack {
    pub physical: cq_lang_structures::physical::Program,
    pub structures: cq_lang_structures::Program,
    pub program: cq_lang_sequence::Program,
}

impl syn::parse::Parse for Stack {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let physical = input.parse()?;
        let structures = cq_lang_structures::Program::parse(input, &physical)?;
        let program = input.parse()?;
        Ok(Self { physical, structures, program })
    }
}

impl quote::ToTokens for Stack {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        quote::ToTokens::to_tokens(&self.physical, tokens);
        quote::ToTokens::to_tokens(&self.structures, tokens);
        quote::ToTokens::to_tokens(&self.program, tokens);
    }
}
