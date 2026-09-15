//! RelationalPlan -> IndexRequirements: input-column and named-result policies.
mod input;
pub(crate) mod linear;
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

pub(crate) fn uses_result_index_route(source: &ra::Module) -> bool {
    !source.plan.output.head.is_normalized()
        || source.plan.definitions.iter().any(|d| {
            matches!(
                d.operator,
                ra::Operator::Select(_) | ra::Operator::Extend(_) | ra::Operator::Unit(_)
            )
        })
}

/// Logical dataflow domain of the default index edge; independent of storage.
pub(crate) fn is_linear_prefix(source: &ra::Module) -> bool {
    if !ra::contract::well_formed(source) {
        return false;
    }
    let mut current = None::<String>;
    let mut pending = BTreeSet::new();
    let mut projected = false;
    for d in &source.plan.definitions {
        if projected {
            return false;
        }
        let name = symbol_name(&d.result);
        match &d.operator {
            ra::Operator::Unit(_) if current.is_none() => current = Some(name),
            ra::Operator::Rename(_) if current.is_none() => current = Some(name),
            ra::Operator::Rename(_) => {
                pending.insert(name);
            }
            ra::Operator::NaturalJoin(j)
                if current.as_deref() == Some(&symbol_name(&j.left))
                    && pending.remove(&symbol_name(&j.right)) =>
            {
                current = Some(name)
            }
            ra::Operator::Select(s) if current.as_deref() == Some(&symbol_name(&s.input)) => {
                current = Some(name)
            }
            ra::Operator::Extend(e) if current.as_deref() == Some(&symbol_name(&e.input)) => {
                current = Some(name)
            }
            ra::Operator::Project(p)
                if current.as_deref() == Some(&symbol_name(&p.input)) && pending.is_empty() =>
            {
                projected = true;
                current = Some(name);
            }
            _ => return false,
        }
    }
    projected && current.as_deref() == Some(&symbol_name(&source.plan.output.input))
}

/// Derive equality indexes for the default linear physical route.
#[contracts::contract(
    debug_ensures(
        crate::pass_contracts::indexes::supported_linear_plan(source)
            -> ret.as_ref().is_ok_and(ir::contract::well_formed)
    ),
    debug_ensures(
        crate::pass_contracts::indexes::supported_linear_plan(source)
            -> ret.as_ref().is_ok_and(|target| crate::pass_contracts::indexes::indexes_are_exact(source, target))
    ),
    debug_ensures(
        !crate::pass_contracts::indexes::supported_linear_plan(source) -> ret.is_err()
    ),
)]
pub fn compile_relational_plan(source: &ra::Module) -> syn::Result<ir::Module> {
    ra::contract::check(source)?;
    if uses_result_index_route(source) {
        if !is_linear_prefix(source) {
            return Err(syn::Error::new_spanned(
                source,
                "the lazy backend requires a linear relational prefix",
            ));
        }
        compile(source)
    } else {
        input::compile(source)
    }
}

/// RelationalPlan -> IndexRequirements on exact named operand versions.
pub fn compile_result_indexes(source: &ra::Module) -> syn::Result<ir::Module> {
    ra::contract::check(source)?;
    let target = compile(source)?;
    if !crate::pass_contracts::indexes::result_indexes_are_exact(source, &target) {
        return Err(syn::Error::new_spanned(
            source,
            "result-index pass violated its source-target contract",
        ));
    }
    Ok(target)
}
