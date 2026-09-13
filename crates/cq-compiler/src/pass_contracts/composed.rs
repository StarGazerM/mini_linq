//! Independent contracts for deferred index selection and composed realization.
//!
//! An empty overlay requests no named indexed resources. It does not assert that
//! the selected physical algorithms need no auxiliary data structures.

use cq_ir::{index_requirements as indexes, relational_plan as ra};

/// CP-Defer preserves the exact source and produces the exact empty overlay.
pub(crate) fn deferred_indexes(source: &ra::Module, target: &indexes::Module) -> syn::Result<()> {
    ra::contract::check(source)?;
    indexes::contract::check(target)?;
    if target.relational_plan != *source || !target.indexes.is_empty() {
        return Err(syn::Error::new_spanned(
            target,
            "deferred index selection must preserve the complete RA and produce an empty index overlay",
        ));
    }
    Ok(())
}

/// CP-Realize never silently drops a written index requirement.
pub(crate) fn domain(source: &indexes::Module) -> syn::Result<()> {
    indexes::contract::check(source)?;
    if let Some(requirement) = source.indexes.first() {
        return Err(syn::Error::new_spanned(
            requirement,
            "composed physical realization accepts only an empty index overlay; use a backend that realizes written index requirements",
        ));
    }
    Ok(())
}

/// Check the source-to-invocation relation without invoking an emitter.
pub(crate) fn realization(
    source: &indexes::Module,
    target: &syn::File,
    options: &crate::physical::composed::PhysicalOptions,
) -> syn::Result<()> {
    domain(source)?;
    crate::physical::composed::check_physical(&source.relational_plan, target, options)
}
