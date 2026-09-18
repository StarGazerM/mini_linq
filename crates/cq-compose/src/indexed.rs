//! A fixed convenience recipe, independent of the frontend's macro transport.
use cq_ir::{index_requirements as ix, relational_plan as ra};

/// Whether the default recipe selects the base-input index policy.
/// The source remains RA; the decision is not serialized into a route tag.
pub fn use_input_indexes(source: &ra::Module) -> syn::Result<bool> {
    ra::contract::check(source)?;
    Ok(source.plan.output.head.is_normalized()
        && cq_index_policy::base_demands::check_domain(source).is_ok())
}

/// Select the default index policy for typed callers.
/// Macro callers invoke the selected policy macro instead of this function.
pub fn select_indexes(source: &ra::Module) -> syn::Result<ix::Module> {
    if use_input_indexes(source)? {
        cq_index_policy::base_demands::compile(source)
    } else {
        cq_index_policy::result_demands::compile(source)
    }
}

/// Compile the fixed typed recipe, retaining IX for inspection.
/// Iterator demand and completed-set demand have separate continuations.
pub fn compile_indexed(
    source: &ra::Module,
    iterator_access: &syn::Path,
    set_access: &syn::Path,
) -> syn::Result<(ix::Module, syn::File)> {
    let indexes = select_indexes(source)?;
    let program = if cq_relation_lowering::base_hash::check_domain(&indexes).is_ok() {
        cq_relation_lowering::base_hash::compile(&indexes, iterator_access)?
    } else if cq_relation_lowering::prefix_hash::check_domain(&indexes).is_ok() {
        cq_relation_lowering::prefix_hash::compile(&indexes, iterator_access)?
    } else {
        cq_relation_lowering::materialized_hash::compile(&indexes, set_access)?
    };
    Ok((indexes, program))
}
