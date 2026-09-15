//! Alternative physical realizations of relational programs.
//! Each module owns its admitted domain, representation and lowering.

pub(crate) mod access;
pub(crate) mod direct;
mod input_indexed;
pub(crate) mod lazy;
pub(crate) mod materialized;
pub(crate) mod options;
pub(crate) mod providers;
mod rust;

use cq_ir::index_requirements;

/// Lower a linear indexed plan to owning storage and a lazy Access region.
///
/// The returned file must contain the complete query API and concrete Rust
/// storage-building statements. Its core join is one
/// `::mini_linq::pull!` call whose body and head occurrences are paired with
/// their exact Rust patterns, sources, predicates, and output expression.
/// Every written index requirement, including an unused extra one, is built as
/// a concrete Rust collection. During its one source walk, this pass returns a
/// span-aware error if a canonical access requirement is missing.
#[contracts::contract(
    debug_ensures(
        index_requirements::contract::well_formed(source)
            && crate::pass_contracts::physical::indexed_input_is_supported(source) -> ret.is_ok()
    ),
    debug_ensures(!crate::pass_contracts::physical::indexed_input_is_supported(source) -> ret.is_err()),
)]
pub fn compile_index_requirements(source: &index_requirements::Module) -> syn::Result<syn::File> {
    index_requirements::contract::check(source)?;
    if crate::relational_to_indexes::uses_result_index_route(&source.relational_plan) {
        crate::relational_to_indexes::check_coverage(source)?;
        let target = lazy::compile(source)?;
        crate::pass_contracts::physical::lazy(source, &target)?;
        Ok(target)
    } else {
        let target = input_indexed::compile(source)?;
        crate::pass_contracts::physical::input_indexed(source, &target)?;
        Ok(target)
    }
}
