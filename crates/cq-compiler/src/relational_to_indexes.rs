//! RelationalPlan -> IndexRequirements, for indexes on named set results.
use cq_ir::{index_requirements as ir, relational_plan as ra, symbol_name};
use quote::quote;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) type Signature = (bool, String, Vec<usize>);
pub(crate) fn signature(index: &ir::IndexRequirement) -> Signature {
    (
        index.result_token.is_some(),
        symbol_name(&index.relation),
        index.key_columns.iter().map(|c| c.index as usize).collect(),
    )
}

pub(crate) fn compile(source: &ra::Module) -> syn::Result<ir::Module> {
    let info = ra::check_in(source.program.inputs(), &source.plan)?;
    let mut seen = BTreeSet::new();
    let mut indexes = Vec::new();
    for definition in &source.plan.definitions {
        if let Some((left, right)) = operands(&definition.operator) {
            let key = columns(&info, left, right);
            if !key.is_empty() && seen.insert((symbol_name(right), key.clone())) {
                let key = key.into_iter().map(syn::Index::from);
                indexes.push(quote! { @#right[#(#key),*]; });
            }
        }
    }
    syn::parse2(quote! { #source indexes { #(#indexes)* } })
}

pub(crate) fn operands(operator: &ra::Operator) -> Option<(&syn::Ident, &syn::Ident)> {
    match operator {
        ra::Operator::NaturalJoin(j) => Some((&j.left, &j.right)),
        ra::Operator::Semijoin(j) => Some((&j.left, &j.right)),
        _ => None,
    }
}

pub(crate) fn columns(
    info: &BTreeMap<String, ra::RelationInfo>,
    left: &syn::Ident,
    right: &syn::Ident,
) -> Vec<usize> {
    let l = &info[&symbol_name(left)].heading;
    info[&symbol_name(right)]
        .heading
        .iter()
        .enumerate()
        .filter_map(|(i, x)| l.contains(x).then_some(i))
        .collect()
}

/// A domain/coverage diagnostic, not a request to regenerate a preferred plan.
pub(crate) fn check_coverage(source: &ir::Module) -> syn::Result<()> {
    let info = ra::check_in(
        source.relational_plan.program.inputs(),
        &source.relational_plan.plan,
    )?;
    let available: BTreeSet<_> = source.indexes.iter().map(signature).collect();
    for d in &source.relational_plan.plan.definitions {
        if let Some((left, right)) = operands(&d.operator) {
            let key = columns(&info, left, right);
            if !key.is_empty() && !available.contains(&(true, symbol_name(right), key)) {
                return Err(syn::Error::new_spanned(
                    d,
                    "missing required index on this exact right-hand RA result",
                ));
            }
        }
    }
    Ok(())
}
