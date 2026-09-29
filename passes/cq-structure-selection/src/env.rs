//! Translate Physical facts to the Structures contract and printer interface.
use super::*;

pub struct Env<'a, E> {
    source: &'a physical::Section,
    env: &'a E,
    facts: structures::PhysicalFacts,
    definitions: Vec<structures::execution::Definition>,
    rows: BTreeMap<String, structures::execution::Row>,
}
impl<'a, E: physical::Env> Env<'a, E> {
    pub fn new(source: &'a physical::Section, env: &'a E) -> syn::Result<Self> {
        Ok(Self {
            source,
            env,
            facts: facts(physical::contract::check(source, env)?),
            definitions: definitions(source),
            rows: rows(env)?,
        })
    }
}
impl<E: physical::Env> structures::PhysicalEnv for Env<'_, E> {
    fn physical(&self) -> &structures::PhysicalFacts {
        &self.facts
    }
    fn host_layouts(&self) -> BTreeMap<String, Vec<String>> {
        self.env
            .inputs()
            .iter()
            .map(|r| {
                (
                    symbol_name(&r.name),
                    r.columns.iter().map(|c| symbol_name(&c.name)).collect(),
                )
            })
            .collect()
    }
    fn results(&self) -> Vec<(String, structures::UniqueOp)> {
        use physical::LogicalOp as L;
        use structures::UniqueOp as U;
        self.env
            .results()
            .into_iter()
            .map(|(name, op)| {
                let op = match op {
                    L::Unit => U::Unit,
                    L::Rename => U::Rename {
                        host: self.facts.hosts[&name].clone(),
                    },
                    L::Join { left, right } => U::Join { left, right },
                    L::Semijoin { left: input, .. } | L::Select { input } | L::Extend { input } => {
                        U::Preserve { input }
                    }
                    L::Project { input, .. } => U::Project { input },
                };
                (name, op)
            })
            .collect()
    }
    fn merge_keys(&self) -> BTreeMap<String, Vec<String>> {
        self.source
            .program
            .definitions
            .iter()
            .filter_map(|d| match &d.operator {
                Operator::Merge(_, k) => Some((
                    symbol_name(&d.name),
                    sorted(&k.columns.iter().map(symbol_name).collect::<Vec<_>>()),
                )),
                _ => None,
            })
            .collect()
    }
    fn output(&self) -> String {
        symbol_name(&self.source.program.returned)
    }
}
impl<E: physical::Env> structures::Env for Env<'_, E> {
    fn inputs(&self) -> Vec<physical::syntax::RelationDecl> {
        self.env.inputs()
    }
    fn definitions(&self) -> &[structures::execution::Definition] {
        &self.definitions
    }
    fn row(&self, result: &str) -> &structures::execution::Row {
        &self.rows[result]
    }
    fn returned(&self) -> &syn::Ident {
        &self.source.program.returned
    }
    fn head(&self) -> &physical::syntax::Atom {
        self.env.head()
    }
}

fn facts(a: physical::Analysis) -> structures::PhysicalFacts {
    structures::PhysicalFacts {
        scanned: a.scanned,
        headings: a.headings,
        hosts: a.hosts,
        renames: a.renames,
        key_reads: a.key_reads,
        merge_reads: a.merge_reads,
        range_reads: a.range_reads,
        canon: a.canon,
        modes: a.modes.into_iter().map(|(n, m)| (n, mode(m))).collect(),
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
