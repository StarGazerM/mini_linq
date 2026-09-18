use super::operators::deferred_iterator;
use cq_ir::iterator_pipeline;
mod check;
pub use check::check;

/// Lower the named physical operators to the actual ordinary-Rust expression.
///
/// This is the final semantic lowering: it selects standard iterator adapters
/// and writes one Rust local per named operator. The returned [`syn::Expr`] is
/// already the target representation, so proc-macro emission is only
/// `ToTokens`; there is no later code generator that reconstructs the plan.
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
