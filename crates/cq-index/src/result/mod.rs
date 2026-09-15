//! Equality-index selection on exact named RA operand versions.
use cq_ir::{index_requirements as ir, relational_plan as ra, symbol_name};
use quote::quote;
use std::collections::BTreeSet;

pub fn compile(source: &ra::Module) -> syn::Result<ir::Module> {
    ra::contract::check(source)?;
    let info = ra::check_in(source.program.inputs(), &source.plan)?;
    let mut seen = BTreeSet::new();
    let mut indexes = Vec::new();
    for definition in &source.plan.definitions {
        let (left, right) = match &definition.operator {
            ra::Operator::NaturalJoin(join) => (&join.left, &join.right),
            ra::Operator::Semijoin(join) => (&join.left, &join.right),
            _ => continue,
        };
        let key = ra::shared_columns(&info[&symbol_name(left)], &info[&symbol_name(right)]);
        if !key.is_empty() && seen.insert((symbol_name(right), key.clone())) {
            let key = key.into_iter().map(syn::Index::from);
            indexes.push(quote! { @#right[#(#key),*]; });
        }
    }
    let target = syn::parse2(quote! { #source indexes { #(#indexes)* } })?;
    if !check(source, &target) {
        return Err(syn::Error::new_spanned(
            source,
            "result-index pass violated its source-target contract",
        ));
    }
    Ok(target)
}

mod correspondence;
pub use correspondence::result_indexes_are_exact as check;
