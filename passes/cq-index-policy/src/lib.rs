//! RA → IX: name exact index demands for joins and semijoins.
//!
//! This analysis preserves the relational plan and makes no representation,
//! physical algorithm, or continuation choice.

use std::collections::BTreeSet;

use cq_lang_ix as ix;
use cq_lang_ra as ra;
use cq_lang_ra::syntax::symbol_name;

mod correspondence;
pub use correspondence::check;

/// Visit the fixed RA definitions in order, sharing identical descriptors.
pub fn compile(source: &ra::Section, env: &impl ra::Env) -> syn::Result<ix::Section> {
    ra::contract::check(source, env)?;
    let info = ra::check_in(env.inputs(), &source.plan)?;
    let mut seen = BTreeSet::new();
    let mut indexes = cq_lang_ra::syntax::SemicolonList::new();

    for definition in &source.plan.definitions {
        let (left, right) = match &definition.operator {
            ra::Operator::NaturalJoin(join) => (&join.left, &join.right),
            ra::Operator::Semijoin(join) => (&join.left, &join.right),
            ra::Operator::Unit(_)
            | ra::Operator::Rename(_)
            | ra::Operator::Project(_)
            | ra::Operator::Select(_)
            | ra::Operator::Extend(_) => continue,
        };
        let columns = ix::result_join_columns(
            &info[&symbol_name(left)]
                .heading
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            &info[&symbol_name(right)]
                .heading
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
        );
        let descriptor = (symbol_name(right), columns.clone());
        if seen.insert(descriptor) {
            let name = syn::Ident::new(&format!("i{}", indexes.len()), right.span());
            indexes.push(ix::IndexDefinition {
                name,
                eq_token: Default::default(),
                requirement: ix::IndexRequirement {
                    at_token: Default::default(),
                    relation: right.clone(),
                    bracket_token: Default::default(),
                    columns: columns.into_iter().map(syn::Index::from).collect(),
                },
            });
            indexes.push_punct(Default::default());
        }
    }

    let section = ix::Section {
        indexes_token: Default::default(),
        indexes_brace: Default::default(),
        indexes,
    };
    if !check(source, env, &section) {
        return Err(syn::Error::new_spanned(
            source,
            "index analysis violated the RA → IX contract",
        ));
    }
    Ok(section)
}

/// IX facts derived from a checked RA section and its source environment.
pub struct Env<'a, E> {
    source: &'a ra::Section,
    env: &'a E,
    headings: std::collections::BTreeMap<String, Vec<String>>,
}
impl<'a, E: ra::Env> Env<'a, E> {
    pub fn new(source: &'a ra::Section, env: &'a E) -> syn::Result<Self> {
        ra::contract::check(source, env)?;
        let headings = ra::check_in(env.inputs(), &source.plan)?
            .into_iter()
            .map(|(n, i)| (n, i.heading.into_iter().collect()))
            .collect();
        Ok(Self {
            source,
            env,
            headings,
        })
    }
}
impl<E: ra::Env> ix::RaEnv for Env<'_, E> {
    fn heading(&self, result: &str) -> Option<&[String]> {
        self.headings.get(result).map(Vec::as_slice)
    }
    fn joins(&self) -> Vec<ix::Join> {
        self.source
            .plan
            .definitions
            .iter()
            .filter_map(|d| {
                let (left, right, semi) = match &d.operator {
                    ra::Operator::NaturalJoin(j) => (&j.left, &j.right, false),
                    ra::Operator::Semijoin(j) => (&j.left, &j.right, true),
                    _ => return None,
                };
                Some(ix::Join {
                    result: symbol_name(&d.result),
                    left: symbol_name(left),
                    right: symbol_name(right),
                    semi,
                })
            })
            .collect()
    }
}
impl<E: ra::Env> ix::Env for Env<'_, E> {
    fn inputs(&self) -> Vec<ra::syntax::RelationDecl> {
        self.env.inputs().to_vec()
    }
    fn operations(&self) -> Vec<(syn::Ident, ix::Operation)> {
        self.source
            .plan
            .definitions
            .iter()
            .map(|d| {
                use ix::Operation as I;
                use ra::Operator as R;
                let op = match &d.operator {
                    R::Unit(_) => I::Unit,
                    R::Rename(r) => I::Rename {
                        host: r.relation.clone(),
                        mapping: r
                            .mapping
                            .iter()
                            .map(|m| (m.target.clone(), m.source.clone()))
                            .collect(),
                    },
                    R::NaturalJoin(j) => I::Join {
                        left: j.left.clone(),
                        right: j.right.clone(),
                    },
                    R::Semijoin(j) => I::Semijoin {
                        left: j.left.clone(),
                        right: j.right.clone(),
                    },
                    R::Select(s) => I::Select {
                        input: s.input.clone(),
                        condition: s.condition.clone(),
                    },
                    R::Extend(e) => I::Extend {
                        input: e.input.clone(),
                        columns: e.columns.iter().cloned().collect(),
                        value: e.value.clone(),
                    },
                    R::Project(p) => I::Project {
                        input: p.input.clone(),
                    },
                };
                (d.result.clone(), op)
            })
            .collect()
    }
    fn returned(&self) -> syn::Ident {
        self.source.plan.output.input.clone()
    }
    fn head(&self) -> &ra::syntax::Atom {
        &self.source.plan.output.head
    }
}
#[cfg(feature = "reference")]
pub mod reference {
    use super::*;
    pub fn compile(source: &ra::Module) -> syn::Result<ix::Section> {
        ra::contract::check_module(source)?;
        super::compile(source, &ra::reference::Env::new(&source.program))
    }
    pub fn check(source: &ra::Module, target: &ix::Section) -> bool {
        ra::contract::check_module(source).is_ok()
            && super::check(source, &ra::reference::Env::new(&source.program), target)
    }
}
