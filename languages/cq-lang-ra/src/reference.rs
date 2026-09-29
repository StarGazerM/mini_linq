//! Optional reference CQ prefix composition.
use super::*;
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub struct Module {
    pub program: cq::Program,
    pub section: Section,
}
impl std::ops::Deref for Module {
    type Target = Section;
    fn deref(&self) -> &Section {
        &self.section
    }
}
impl std::ops::DerefMut for Module {
    fn deref_mut(&mut self) -> &mut Section {
        &mut self.section
    }
}
impl Module {
    pub fn new(program: cq::Program, section: Section) -> Self {
        Self { program, section }
    }
}

pub struct Env<'a> {
    program: &'a cq::Program,
    inputs: Vec<cq_syntax::RelationDecl>,
}
impl<'a> Env<'a> {
    pub fn new(program: &'a cq::Program) -> Self {
        Self {
            program,
            inputs: program.inputs().cloned().collect(),
        }
    }
}
impl CqEnv for Env<'_> {
    fn inputs(&self) -> &[cq_syntax::RelationDecl] {
        &self.inputs
    }
    fn head(&self) -> &cq_syntax::Atom {
        &self.program.query.head
    }
}

impl syn::parse::Parse for Module {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let staged: cq_syntax::Staged<cq::Program, Section> = input.parse()?;
        Ok(Self::new(staged.prefix, staged.section))
    }
}
