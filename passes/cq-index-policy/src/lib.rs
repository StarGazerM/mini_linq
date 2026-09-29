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
pub fn compile(source: &ra::Module) -> syn::Result<ix::Section> {
    ra::contract::check_module(source)?;
    let info = ra::check_in(source.program.inputs(), &source.plan)?;
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
    if !check(source, &section) {
        return Err(syn::Error::new_spanned(
            source,
            "index analysis violated the RA → IX contract",
        ));
    }
    Ok(section)
}
