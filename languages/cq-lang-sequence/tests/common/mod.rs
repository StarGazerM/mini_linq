use cq_lang_sequence::{Program, structures};
use quote::ToTokens;

/// Fixture stack: the selected Sequence syntax is parsed separately from its bases.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stack {
    pub physical: structures::physical::Program,
    pub structures: structures::Program,
    pub program: Program,
}

impl syn::parse::Parse for Stack {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let physical: structures::physical::Program = input.parse()?;
        let structures = structures::Program::parse(input, &physical)?;
        Ok(Self {
            physical,
            structures,
            program: input.parse()?,
        })
    }
}

impl ToTokens for Stack {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.physical.to_tokens(tokens);
        self.structures.to_tokens(tokens);
        self.program.to_tokens(tokens);
    }
}

impl std::ops::Deref for Stack {
    type Target = Program;
    fn deref(&self) -> &Program { &self.program }
}

impl std::ops::DerefMut for Stack {
    fn deref_mut(&mut self) -> &mut Program { &mut self.program }
}

pub fn check(stack: &Stack) -> syn::Result<cq_lang_sequence::Analysis> {
    cq_lang_sequence::contract::check(&stack.physical, &stack.structures, &stack.program)
}

pub fn check_grading(stack: &Stack, machine: &cq_lang_sequence::Machine) -> syn::Result<cq_lang_sequence::Analysis> {
    cq_lang_sequence::check_grading(&stack.physical, &stack.structures, &stack.program, machine)
}
