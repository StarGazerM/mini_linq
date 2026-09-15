use super::operators::deferred_iterator;
use cq_ir::iterator_pipeline;
mod check;
pub use check::check;

/// Realize the result's support as a stream which may contain duplicates.
///
/// The checked linear Pipeline has a returned terminal `Distinct` with no
/// downstream consumer. This endpoint omits precisely that operator; every
/// preceding operator retains its normal implementation and source order.
/// The stream's support equals the Pipeline's set denotation. Consumers must
/// be insensitive to multiplicity, or separately establish output uniqueness.
/// Resource collectors and conservative summary accumulation satisfy the
/// first obligation; unique provider handles satisfy the second.
#[contracts::contract(
    debug_ensures(
        iterator_pipeline::contract::well_formed(source) -> ret.as_ref()
            .is_ok_and(|target| check(source, target))
    )
)]
pub fn compile(source: &iterator_pipeline::Pipeline) -> syn::Result<syn::Expr> {
    iterator_pipeline::contract::check(source)?;
    let (last, prefix) = source
        .definitions
        .split_last()
        .expect("a well-formed pipeline has a terminal distinct operator");
    let iterator_pipeline::Operator::Distinct(distinct) = &last.operator else {
        unreachable!("a well-formed pipeline ends in distinct");
    };
    deferred_iterator(prefix, &distinct.input_stream)
}
