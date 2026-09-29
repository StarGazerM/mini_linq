//! RA → IX: name exact index demands for joins and semijoins.
//!
//! This analysis preserves the relational plan and makes no representation,
//! physical algorithm, or continuation choice. [`check`] independently derives
//! the ordered first-use demands and their rule-mandated names; fixtures
//! separately pin the expansion.

use std::collections::BTreeSet;

use cq_lang_ix as ix;
use cq_lang_ra as ra;
use cq_lang_ra::syntax::symbol_name;

/// Visit the fixed RA definitions in order, sharing identical descriptors.
pub fn compile(source: &ra::Program) -> syn::Result<ix::Program> {
    let info = ra::contract::check(source)?;
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
        let columns: Vec<usize> = info[&symbol_name(right)]
            .heading
            .iter()
            .enumerate()
            .filter_map(|(i, c)| info[&symbol_name(left)].heading.contains(c).then_some(i))
            .collect();
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

    let target = ix::Program {
        ra: source.clone(),
        indexes_token: Default::default(),
        indexes_brace: Default::default(),
        indexes,
    };
    ix::contract::check(&target)?;
    Ok(target)
}

mod correspondence;
pub use correspondence::check;
