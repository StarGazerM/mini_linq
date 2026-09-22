use super::operators::deferred_iterator;
use cq_ir::iterator_pipeline;
mod check;
pub use check::check;

/// Lower the named physical operators to the actual ordinary-Rust expression.
///
/// Select standard iterator adapters and write one Rust local per operator.
/// Distinct state remains explicit caller-resolved provider macro expressions;
/// their expansion supplies representation code without reconstructing this plan.
#[contracts::contract(
    debug_ensures(
        iterator_pipeline::contract::well_formed(source) -> ret.as_ref()
            .is_ok_and(|target| check(source, target))
    )
)]
pub fn compile(source: &iterator_pipeline::Pipeline) -> syn::Result<syn::Expr> {
    iterator_pipeline::contract::check(source)?;
    deferred_iterator(&source.definitions, &source.return_stream.stream)
}
