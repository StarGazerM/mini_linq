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

impl super::Env for Env<'_> {
    fn inputs(&self) -> Vec<cq_syntax::RelationDecl> {
        self.source.program.inputs().cloned().collect()
    }
    fn operations(&self) -> Vec<(syn::Ident, Operation)> {
        self.source
            .plan
            .definitions
            .iter()
            .map(|d| (d.result.clone(), operation(&d.operator)))
            .collect()
    }
    fn returned(&self) -> syn::Ident {
        self.source.plan.output.input.clone()
    }
    fn head(&self) -> &cq_syntax::Atom {
        &self.source.plan.output.head
    }
}
/// Translate reference syntax once at the IX boundary.
pub fn operation(op: &relational_plan::Operator) -> Operation {
    use relational_plan::Operator as R;
    match op {
        R::Unit(_) => Operation::Unit,
        R::Rename(r) => Operation::Rename {
            host: r.relation.clone(),
            mapping: r
                .mapping
                .iter()
                .map(|m| (m.target.clone(), m.source.clone()))
                .collect(),
        },
        R::NaturalJoin(j) => Operation::Join {
            left: j.left.clone(),
            right: j.right.clone(),
        },
        R::Semijoin(j) => Operation::Semijoin {
            left: j.left.clone(),
            right: j.right.clone(),
        },
        R::Select(s) => Operation::Select {
            input: s.input.clone(),
            condition: s.condition.clone(),
        },
        R::Extend(e) => Operation::Extend {
            input: e.input.clone(),
            columns: e.columns.iter().cloned().collect(),
            value: e.value.clone(),
        },
        R::Project(p) => Operation::Project {
            input: p.input.clone(),
        },
    }
}
