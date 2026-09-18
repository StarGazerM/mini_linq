//! First-use equality requirements on declared input columns.

use cq_ir::{index_requirements, relational_plan, symbol_name};
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};
type IndexSignature = (String, Vec<usize>);

mod domain;
pub use domain::check as check_domain;

/// Preserve a relational plan and derive its canonical right-Rename keys.
pub fn compile(source: &relational_plan::Module) -> syn::Result<index_requirements::Module> {
    check_domain(source)?;
    let info = relational_plan::check_in(source.program.inputs(), &source.plan)?;
    let declarations = source
        .program
        .inputs()
        .map(|input| (symbol_name(&input.name), input))
        .collect::<BTreeMap<_, _>>();
    let definitions = source
        .plan
        .definitions
        .iter()
        .map(|definition| (symbol_name(&definition.result), &definition.operator))
        .collect::<BTreeMap<_, _>>();
    let mut seen = BTreeSet::<IndexSignature>::new();
    let mut requirements = Vec::new();
    for definition in &source.plan.definitions {
        let relational_plan::Operator::NaturalJoin(join) = &definition.operator else {
            continue;
        };
        let relational_plan::Operator::Rename(rename) = definitions[&symbol_name(&join.right)]
        else {
            unreachable!("the input-index domain requires a direct right Rename");
        };
        let declaration = declarations[&symbol_name(&rename.relation)];
        let mapping = rename
            .mapping
            .iter()
            .map(|mapping| (symbol_name(&mapping.source), symbol_name(&mapping.target)))
            .collect::<BTreeMap<_, _>>();
        let bound = &info[&symbol_name(&join.left)].heading;
        let columns = declaration
            .columns
            .iter()
            .enumerate()
            .filter_map(|(column, declaration)| {
                bound
                    .contains(&mapping[&symbol_name(&declaration.name)])
                    .then_some(column)
            })
            .collect::<Vec<_>>();
        if columns.is_empty() || !seen.insert((symbol_name(&rename.relation), columns.clone())) {
            continue;
        }
        let relation = &rename.relation;
        let columns = columns.into_iter().map(syn::Index::from);
        requirements.push(quote!(#relation[#(#columns),*]));
    }

    let target = syn::parse2(quote! {
        #source
        indexes { #(#requirements;)* }
    })?;
    if !check(source, &target) {
        return Err(syn::Error::new_spanned(
            source,
            "input-index pass violated its source-target contract",
        ));
    }
    Ok(target)
}

mod correspondence;
pub use correspondence::indexes_are_exact as check;
