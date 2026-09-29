//! Adapt checked IX facts to the target's own environment contract.
use super::*;

pub struct Env<'a, E> {
    source: &'a ix::Section,
    env: &'a E,
    operations: Vec<(Ident, Op)>,
    rows: BTreeMap<String, ix::RowFacts>,
}
impl<'a, E: ix::Env> Env<'a, E> {
    pub fn new(source: &'a ix::Section, env: &'a E) -> syn::Result<Self> {
        ix::contract::check(source, env)?;
        Ok(Self {
            source,
            env,
            operations: env.operations(),
            rows: ix::rows(env),
        })
    }
    fn operation(&self, result: &str) -> Option<&Op> {
        self.operations
            .iter()
            .find(|(name, _)| symbol_name(name) == result)
            .map(|(_, op)| op)
    }
}
impl<E: ix::Env> physical::IxEnv for Env<'_, E> {
    fn results(&self) -> Vec<(String, physical::LogicalOp)> {
        use physical::LogicalOp as L;
        self.operations
            .iter()
            .map(|(name, op)| {
                let name = symbol_name(name);
                let op = match op {
                    Op::Unit => L::Unit,
                    Op::Rename { .. } => L::Rename,
                    Op::Join { left, right } => L::Join {
                        left: symbol_name(left),
                        right: symbol_name(right),
                    },
                    Op::Semijoin { left, right } => L::Semijoin {
                        left: symbol_name(left),
                        right: symbol_name(right),
                    },
                    Op::Select { input, .. } => L::Select {
                        input: symbol_name(input),
                    },
                    Op::Extend { input, .. } => L::Extend {
                        input: symbol_name(input),
                    },
                    Op::Project { input } => L::Project {
                        input: symbol_name(input),
                        keep: self.env.heading(&name).expect("checked heading").to_vec(),
                    },
                };
                (name, op)
            })
            .collect()
    }
    fn heading(&self, result: &str) -> Option<&[String]> {
        self.env.heading(result)
    }
    fn output(&self) -> String {
        symbol_name(&self.env.returned())
    }
    fn demands(&self) -> BTreeSet<(String, Vec<usize>)> {
        self.source
            .indexes
            .iter()
            .map(|i| ix::signature(&i.requirement))
            .collect()
    }
    fn host_of(&self, result: &str) -> Option<(String, BTreeMap<String, String>)> {
        let Op::Rename { host, mapping } = self.operation(result)? else {
            return None;
        };
        Some((
            symbol_name(host),
            mapping
                .iter()
                .map(|(target, source)| (symbol_name(target), symbol_name(source)))
                .collect(),
        ))
    }
    fn select_condition(&self, result: &str) -> Option<&syn::Expr> {
        let Op::Select { condition, .. } = self.operation(result)? else {
            return None;
        };
        Some(condition)
    }
}
impl<E: ix::Env> physical::Env for Env<'_, E> {
    fn inputs(&self) -> Vec<cq::RelationDecl> {
        self.env.inputs()
    }
    fn head(&self) -> &cq::Atom {
        self.env.head()
    }
    fn bindings(&self, result: &str) -> BTreeMap<String, Ident> {
        self.rows[result].bindings.clone()
    }
    fn computed(&self, result: &str) -> BTreeSet<String> {
        self.rows[result].computed.clone()
    }
    fn extension(&self, result: &str) -> Option<(Vec<cq::ColumnDecl>, syn::Expr)> {
        let Op::Extend { columns, value, .. } = self.operation(result)? else {
            return None;
        };
        Some((columns.clone(), value.clone()))
    }
}
