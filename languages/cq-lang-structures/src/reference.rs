//! The checked reference prefix, optional for consumers of this section.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub struct Module {
    pub physical: physical::Module,
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
    pub fn new(physical: physical::Module, section: Section) -> Self {
        Self { physical, section }
    }
}

pub struct Env<'a> {
    source: &'a physical::Module,
    facts: PhysicalFacts,
}
impl<'a> Env<'a> {
    pub fn new(source: &'a physical::Module) -> syn::Result<Self> {
        let a = physical::contract::check_module(source)?;
        let facts = PhysicalFacts {
            headings: a.headings,
            hosts: a.hosts,
            renames: a.renames,
            key_reads: a.key_reads,
            merge_reads: a.merge_reads,
            range_reads: a.range_reads,
            canon: a.canon,
            modes: a
                .modes
                .into_iter()
                .map(|(name, mode)| {
                    (
                        name,
                        match mode {
                            physical::ModeKind::Pipe => ModeKind::Pipe,
                            physical::ModeKind::Let => ModeKind::Let,
                            physical::ModeKind::Alias => ModeKind::Alias,
                        },
                    )
                })
                .collect(),
        };
        Ok(Self { source, facts })
    }
}
impl PhysicalEnv for Env<'_> {
    fn physical(&self) -> &PhysicalFacts {
        &self.facts
    }
    fn host_layouts(&self) -> BTreeMap<String, Vec<String>> {
        self.source
            .index_requirements
            .relational_plan
            .program
            .inputs()
            .map(|r| {
                (
                    symbol_name(&r.name),
                    r.columns.iter().map(|c| symbol_name(&c.name)).collect(),
                )
            })
            .collect()
    }
    fn results(&self) -> Vec<(String, UniqueOp)> {
        self.source
            .index_requirements
            .relational_plan
            .plan
            .definitions
            .iter()
            .map(|d| {
                let op = match &d.operator {
                    ra::Operator::Unit(_) => UniqueOp::Unit,
                    ra::Operator::Rename(r) => UniqueOp::Rename {
                        host: symbol_name(&r.relation),
                    },
                    ra::Operator::NaturalJoin(j) => UniqueOp::Join {
                        left: symbol_name(&j.left),
                        right: symbol_name(&j.right),
                    },
                    ra::Operator::Semijoin(j) => UniqueOp::Preserve {
                        input: symbol_name(&j.left),
                    },
                    ra::Operator::Select(s) => UniqueOp::Preserve {
                        input: symbol_name(&s.input),
                    },
                    ra::Operator::Extend(e) => UniqueOp::Preserve {
                        input: symbol_name(&e.input),
                    },
                    ra::Operator::Project(p) => UniqueOp::Project {
                        input: symbol_name(&p.input),
                    },
                };
                (symbol_name(&d.result), op)
            })
            .collect()
    }
    fn merge_keys(&self) -> BTreeMap<String, Vec<String>> {
        self.source
            .program
            .definitions
            .iter()
            .filter_map(|d| match &d.operator {
                physical::Operator::Merge(_, k) => Some((symbol_name(&d.name), names(&k.columns))),
                _ => None,
            })
            .collect()
    }
    fn output(&self) -> String {
        symbol_name(&self.source.program.returned)
    }
}

impl syn::parse::Parse for Module {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let staged: cq_syntax::Staged<physical::Module, Section> = input.parse()?;
        Ok(Self::new(staged.prefix, staged.section))
    }
}
