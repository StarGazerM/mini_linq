use super::operators::lower_operator_to_rust;
use cq_ir::iterator_pipeline;
use quote::quote;
mod check;
pub use check::check;

/// Consume a pipeline into its exact result set.
///
/// The terminal `Distinct` and the selected exact-set sink establish the same uniqueness
/// guarantee, so this endpoint implements both with one set. All preceding
/// operators retain their usual lowering; the iterator endpoint is unchanged.
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
    let locals = prefix.iter().map(lower_operator_to_rust);
    let input = &distinct.input_stream;
    let result = &last.stream;
    syn::parse2(quote! {{
        #(#locals)*
        let mut #result = mini_linq::set_sink! { operator #result; new };
        for __cq_row in #input {
            mini_linq::set_sink! { operator #result; insert(#result, __cq_row) };
        }
        mini_linq::set_sink! { operator #result; finish(#result) }
    }})
}
