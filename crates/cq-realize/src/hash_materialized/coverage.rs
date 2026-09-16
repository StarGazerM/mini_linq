//! Named-result equality keys and coverage of an explicit index overlay.
use cq_ir::{index_requirements as ir, relational_plan as ra, symbol_name};
use ir::signature;
use std::collections::BTreeSet;

/// A domain/coverage diagnostic, not a request to regenerate a preferred plan.
pub fn check_coverage(source: &ir::Module) -> syn::Result<()> {
    let info = ra::check_in(
        source.relational_plan.program.inputs(),
        &source.relational_plan.plan,
    )?;
    let available: BTreeSet<_> = source.indexes.iter().map(signature).collect();
    for d in &source.relational_plan.plan.definitions {
        let operands = match &d.operator {
            ra::Operator::NaturalJoin(j) => Some((&j.left, &j.right)),
            ra::Operator::Semijoin(j) => Some((&j.left, &j.right)),
            _ => None,
        };
        if let Some((left, right)) = operands {
            let key = ra::shared_columns(&info[&symbol_name(left)], &info[&symbol_name(right)]);
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
