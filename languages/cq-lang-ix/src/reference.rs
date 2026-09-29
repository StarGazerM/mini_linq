//! Reference composition. The standalone language checks its own section against an Env.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub struct Module {
    pub relational_plan: relational_plan::Module,
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
    pub fn new(relational_plan: relational_plan::Module, section: Section) -> Self {
        Self {
            relational_plan,
            section,
        }
    }
}

/// Cached checked headings let `RaEnv::heading` return a borrowed slice.
pub struct Env<'a> {
    pub source: &'a relational_plan::Module,
    headings: BTreeMap<String, Vec<String>>,
}
impl<'a> Env<'a> {
    pub fn new(source: &'a relational_plan::Module) -> syn::Result<Self> {
        relational_plan::contract::check_module(source)?;
        let headings = relational_plan::check_in(source.program.inputs(), &source.plan)?
            .into_iter()
            .map(|(name, info)| (name, info.heading.into_iter().collect()))
            .collect();
        Ok(Self { source, headings })
    }
}
impl RaEnv for Env<'_> {
    fn heading(&self, result: &str) -> Option<&[String]> {
        self.headings.get(result).map(Vec::as_slice)
    }
    fn joins(&self) -> Vec<Join> {
        self.source
            .plan
            .definitions
            .iter()
            .filter_map(|d| {
                let (left, right, semi) = match &d.operator {
                    relational_plan::Operator::NaturalJoin(j) => (&j.left, &j.right, false),
                    relational_plan::Operator::Semijoin(j) => (&j.left, &j.right, true),
                    _ => return None,
                };
                Some(Join {
                    result: symbol_name(&d.result),
                    left: symbol_name(left),
                    right: symbol_name(right),
                    semi,
                })
            })
            .collect()
    }
}

impl syn::parse::Parse for Module {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let staged: cq_syntax::Staged<relational_plan::Module, Section> = input.parse()?;
        Ok(Self::new(staged.prefix, staged.section))
    }
}
