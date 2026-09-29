//! RA → IX: name exact index demands for joins and semijoins.
//!
//! This analysis preserves the relational plan and makes no representation,
//! physical algorithm, or continuation choice.

use std::collections::BTreeSet;

use cq_ir::{index_requirements as ix, relational_plan as ra, symbol_name};

mod correspondence;
pub use correspondence::check;

/// Visit the fixed RA definitions in order, sharing identical descriptors.
pub fn compile(source: &ra::Module) -> syn::Result<ix::Module> {
    ra::contract::check(source)?;
    let info = ra::check_in(source.program.inputs(), &source.plan)?;
    let mut seen = BTreeSet::new();
    let mut indexes = cq_ir::SemicolonList::new();

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
        let columns =
            ix::result_join_columns(&info[&symbol_name(left)], &info[&symbol_name(right)]);
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

    let target = ix::Module {
        relational_plan: source.clone(),
        indexes_token: Default::default(),
        indexes_brace: Default::default(),
        indexes,
    };
    if !check(source, &target) {
        return Err(syn::Error::new_spanned(
            source,
            "index analysis violated the RA → IX contract",
        ));
    }
    Ok(target)
}
