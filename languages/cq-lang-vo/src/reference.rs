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

impl CqEnv for cq::Module {
    fn clauses(&self) -> syn::Result<Vec<Clause>> {
        cq::normal::check(self)?;
        Ok(self
            .program
            .query
            .body
            .iter()
            .filter_map(|item| match item {
                cq::BodyItem::Positive { atom } => Some(Clause::Atom(atom.clone())),
                cq::BodyItem::Filter(f) => Some(Clause::Guard(f.clone())),
                cq::BodyItem::Let(l) => Some(Clause::Let(l.clone())),
                _ => None,
            })
            .collect())
    }
}

impl syn::parse::Parse for Module {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let staged: cq_syntax::Staged<cq::Program, Section> = input.parse()?;
        Ok(Self::new(staged.prefix, staged.section))
    }
}
