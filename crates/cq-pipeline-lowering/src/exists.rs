//! Existential consumption of the existing Pipeline language.

use super::operators::deferred_iterator;
use cq_ir::iterator_pipeline;
use quote::quote;

/// Return whether the Pipeline produces any row, stopping at its first witness.
///
/// Terminal distinctness cannot change nonemptiness, so this endpoint omits
/// precisely that operator. Every preceding source, predicate and projection
/// keeps its ordinary lowering and evaluation order. Evaluating the returned
/// Rust expression consumes enough of the prefix to find a witness or exhaust
/// it; it neither collects the remaining rows nor adds equality, hashing or
/// cloning requirements to the returned payload.
pub fn compile(source: &iterator_pipeline::Pipeline) -> syn::Result<syn::Expr> {
    iterator_pipeline::contract::check(source)?;
    let (last, prefix) = source
        .definitions
        .split_last()
        .expect("a well-formed pipeline has a terminal distinct operator");
    let iterator_pipeline::Operator::Distinct(distinct) = &last.operator else {
        unreachable!("a well-formed pipeline ends in distinct");
    };
    let prefix = deferred_iterator(prefix, &distinct.input_stream)?;
    syn::parse2(quote! {
        ::core::iter::Iterator::next(&mut (#prefix)).is_some()
    })
}
