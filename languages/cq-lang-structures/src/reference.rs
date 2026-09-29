//! The checked reference prefix, optional for consumers of this section.
use super::*;
use crate as structures;
use physical::Operator;

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
    definitions: Vec<execution::Definition>,
    rows: BTreeMap<String, execution::Row>,
}
impl<'a> Env<'a> {
    pub fn new(source: &'a physical::Module) -> syn::Result<Self> {
        let a = physical::contract::check_module(source)?;
        let facts = PhysicalFacts {
            scanned: a.scanned,
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
        Ok(Self {
            source,
            facts,
            definitions: definitions(source),
            rows: rows(&physical::reference::Env::new(&source.index_requirements)?)?,
        })
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

impl super::Env for Env<'_> {
    fn inputs(&self) -> Vec<cq_syntax::RelationDecl> {
        self.source
            .index_requirements
            .relational_plan
            .program
            .inputs()
            .cloned()
            .collect()
    }
    fn definitions(&self) -> &[execution::Definition] {
        &self.definitions
    }
    fn row(&self, result: &str) -> &execution::Row {
        &self.rows[result]
    }
    fn returned(&self) -> &Ident {
        &self.source.program.returned
    }
    fn head(&self) -> &cq_syntax::Atom {
        &self
            .source
            .index_requirements
            .relational_plan
            .plan
            .output
            .head
    }
}
fn mode(m: physical::ModeKind) -> structures::ModeKind {
    match m {
        physical::ModeKind::Pipe => structures::ModeKind::Pipe,
        physical::ModeKind::Let => structures::ModeKind::Let,
        physical::ModeKind::Alias => structures::ModeKind::Alias,
    }
}
fn definitions(source: &physical::Section) -> Vec<structures::execution::Definition> {
    use structures::execution::{Definition, Keyed, Operator as S};
    source
        .program
        .definitions
        .iter()
        .map(|d| {
            let keyed = |k: &physical::Keyed| Keyed {
                columns: k.columns.clone(),
                left: k.left.clone(),
                right: k.right.clone(),
            };
            let operator = match &d.operator {
                Operator::Unit(_) => S::Unit,
                Operator::Rename(_) => S::Rename,
                Operator::Select(_) => S::Select,
                Operator::Extend(_) => S::Extend,
                Operator::Project(_) => S::Project,
                Operator::Range(_, r) => S::Range(r.column.clone()),
                Operator::Probe(_, k) => S::Probe(keyed(k)),
                Operator::Semi(_, k) => S::Semi(keyed(k)),
                Operator::Merge(_, k) => S::Merge(keyed(k)),
            };
            Definition {
                name: d.name.clone(),
                mode: mode((&d.mode).into()),
                canon: d.canon.is_some(),
                operator,
            }
        })
        .collect()
}
fn rows(env: &impl physical::Env) -> syn::Result<BTreeMap<String, structures::execution::Row>> {
    use physical::LogicalOp as L;
    use structures::execution::{Extend, Row, Select};
    env.results()
        .into_iter()
        .map(|(name, op)| {
            let input_ident = |input: &str| {
                syn::parse_str::<syn::Ident>(input)
                    .or_else(|_| syn::parse_str(&format!("r#{input}")))
            };
            let mut row = Row {
                bindings: env.bindings(&name),
                computed: env.computed(&name),
                ..Row::default()
            };
            match op {
                L::Select { input } => {
                    row.select = Some(Select {
                        input: input_ident(&input)?,
                        condition: env
                            .select_condition(&name)
                            .ok_or_else(|| {
                                syn::Error::new(
                                    proc_macro2::Span::call_site(),
                                    "missing select condition",
                                )
                            })?
                            .clone(),
                    })
                }
                L::Extend { input } => {
                    let (columns, value) = env.extension(&name).ok_or_else(|| {
                        syn::Error::new(proc_macro2::Span::call_site(), "missing extend expression")
                    })?;
                    row.extend = Some(Extend {
                        input: input_ident(&input)?,
                        columns,
                        value,
                    });
                }
                L::Project { input, .. } => row.project = Some(input_ident(&input)?),
                _ => (),
            }
            Ok((name, row))
        })
        .collect()
}
