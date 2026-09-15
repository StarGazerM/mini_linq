//! Equality-index selection on exact named RA operand versions.
use crate::analysis::indexes;
use cq_ir::{index_requirements as ir, relational_plan as ra, symbol_name};
use quote::quote;
use std::collections::BTreeSet;

pub fn compile_result_indexes(source: &ra::Module) -> syn::Result<ir::Module> {
    ra::contract::check(source)?;
    let info = ra::check_in(source.program.inputs(), &source.plan)?;
    let mut seen = BTreeSet::new();
    let mut indexes = Vec::new();
    for definition in &source.plan.definitions {
        if let Some((left, right)) = indexes::operands(&definition.operator) {
            let key = indexes::columns(&info, left, right);
            if !key.is_empty() && seen.insert((symbol_name(right), key.clone())) {
                let key = key.into_iter().map(syn::Index::from);
                indexes.push(quote! { @#right[#(#key),*]; });
            }
        }
    }
    let target = syn::parse2(quote! { #source indexes { #(#indexes)* } })?;
    if !super::check::result::result_indexes_are_exact(source, &target) {
        return Err(syn::Error::new_spanned(
            source,
            "result-index pass violated its source-target contract",
        ));
    }
    Ok(target)
}
