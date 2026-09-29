//! Reference composition. The standalone language checks its own section against an Env.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub struct Module {
    pub index_requirements: ix::Module,
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
    pub fn new(index_requirements: ix::Module, section: Section) -> Self {
        Self {
            index_requirements,
            section,
        }
    }
}

/// Reference IX facts, with headings cached once after prefix validation.
pub struct Env<'a> {
    source: &'a ix::Module,
    ra: ix::reference::Env<'a>,
    rows: BTreeMap<String, ix::RowFacts>,
}
impl<'a> Env<'a> {
    pub fn new(source: &'a ix::Module) -> syn::Result<Self> {
        let ra = ix::reference::Env::new(&source.relational_plan)?;
        ix::contract::check(&source.section, &ra)?;
        let rows = ix::rows(&ra);
        Ok(Self { source, ra, rows })
    }
}
impl IxEnv for Env<'_> {
    fn results(&self) -> Vec<(String, LogicalOp)> {
        self.source
            .relational_plan
            .plan
            .definitions
            .iter()
            .map(|d| (symbol_name(&d.result), logical_op(&d.operator)))
            .collect()
    }
    fn heading(&self, result: &str) -> Option<&[String]> {
        ix::RaEnv::heading(&self.ra, result)
    }
    fn output(&self) -> String {
        symbol_name(&self.source.relational_plan.plan.output.input)
    }
    fn demands(&self) -> BTreeSet<(String, Vec<usize>)> {
        self.source
            .indexes
            .iter()
            .map(|i| ix::signature(&i.requirement))
            .collect()
    }
    fn host_of(&self, result: &str) -> Option<(String, BTreeMap<String, String>)> {
        let d = self
            .source
            .relational_plan
            .plan
            .definitions
            .iter()
            .find(|d| symbol_name(&d.result) == result)?;
        let ra::Operator::Rename(r) = &d.operator else {
            return None;
        };
        Some((
            symbol_name(&r.relation),
            r.mapping
                .iter()
                .map(|m| (symbol_name(&m.target), symbol_name(&m.source)))
                .collect(),
        ))
    }
    fn select_condition(&self, result: &str) -> Option<&syn::Expr> {
        let d = self
            .source
            .relational_plan
            .plan
            .definitions
            .iter()
            .find(|d| symbol_name(&d.result) == result)?;
        let ra::Operator::Select(s) = &d.operator else {
            return None;
        };
        Some(&s.condition)
    }
}
/// Forget RA syntax, retaining only the Physical contract's logical facts.
pub fn logical_op(op: &ra::Operator) -> LogicalOp {
    match op {
        ra::Operator::Unit(_) => LogicalOp::Unit,
        ra::Operator::Rename(_) => LogicalOp::Rename,
        ra::Operator::Select(s) => LogicalOp::Select {
            input: symbol_name(&s.input),
        },
        ra::Operator::Extend(e) => LogicalOp::Extend {
            input: symbol_name(&e.input),
        },
        ra::Operator::Project(p) => LogicalOp::Project {
            input: symbol_name(&p.input),
            keep: p.attributes.iter().map(symbol_name).collect(),
        },
        ra::Operator::NaturalJoin(j) => LogicalOp::Join {
            left: symbol_name(&j.left),
            right: symbol_name(&j.right),
        },
        ra::Operator::Semijoin(j) => LogicalOp::Semijoin {
            left: symbol_name(&j.left),
            right: symbol_name(&j.right),
        },
    }
}

impl syn::parse::Parse for Module {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let staged: cq_syntax::Staged<ix::Module, Section> = input.parse()?;
        Ok(Self::new(staged.prefix, staged.section))
    }
}

impl super::Env for Env<'_> {
    fn inputs(&self) -> Vec<cq_syntax::RelationDecl> {
        ix::Env::inputs(&self.ra)
    }
    fn head(&self) -> &cq_syntax::Atom {
        ix::Env::head(&self.ra)
    }
    fn bindings(&self, result: &str) -> BTreeMap<String, Ident> {
        self.rows[result].bindings.clone()
    }
    fn computed(&self, result: &str) -> BTreeSet<String> {
        self.rows[result].computed.clone()
    }
    fn extension(&self, result: &str) -> Option<(Vec<cq_syntax::ColumnDecl>, syn::Expr)> {
        ix::Env::operations(&self.ra)
            .into_iter()
            .find_map(|(name, op)| {
                if symbol_name(&name) != result {
                    return None;
                }
                if let ix::Operation::Extend { columns, value, .. } = op {
                    Some((columns, value))
                } else {
                    None
                }
            })
    }
}
