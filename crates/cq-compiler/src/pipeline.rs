//! IteratorPipeline -> ordinary Rust iterator expressions.

use cq_ir::{cq, iterator_pipeline};
use proc_macro2::TokenStream;
use quote::quote;

use crate::pass_contracts;

/// Lower the named physical operators to the actual ordinary-Rust expression.
///
/// This is the final semantic lowering: it selects standard iterator adapters
/// and writes one Rust local per named operator. The returned [`syn::Expr`] is
/// already the target representation, so proc-macro emission is only
/// `ToTokens`; there is no later code generator that reconstructs the plan.
#[contracts::contract(
    debug_ensures(
        iterator_pipeline::contract::well_formed(source) -> ret.as_ref()
            .is_ok_and(|target| pass_contracts::iterator_pipeline_rust_is_exact(source, target))
    )
)]
pub fn compile_iterator_pipeline(source: &iterator_pipeline::Pipeline) -> syn::Result<syn::Expr> {
    iterator_pipeline::contract::check(source)?;
    deferred_iterator(&source.definitions, &source.return_stream.stream)
}

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
            .is_ok_and(|target| pass_contracts::stream_pipeline_rust_is_exact(source, target))
    )
)]
pub fn compile_stream_pipeline(source: &iterator_pipeline::Pipeline) -> syn::Result<syn::Expr> {
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

/// Consume a pipeline into its exact result set.
///
/// The terminal `Distinct` and the `HashSet` sink establish the same uniqueness
/// guarantee, so this endpoint implements both with one set. All preceding
/// operators retain their usual lowering; the iterator endpoint is unchanged.
#[contracts::contract(
    debug_ensures(
        iterator_pipeline::contract::well_formed(source) -> ret.as_ref()
            .is_ok_and(|target| pass_contracts::collect_pipeline_rust_is_exact(source, target))
    )
)]
pub fn compile_collect_pipeline(source: &iterator_pipeline::Pipeline) -> syn::Result<syn::Expr> {
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
        let #result: ::std::collections::HashSet<_> = ::core::iter::Iterator::collect(#input);
        #result
    }})
}

fn deferred_iterator(
    definitions: &[iterator_pipeline::Definition],
    return_stream: &syn::Ident,
) -> syn::Result<syn::Expr> {
    let locals = definitions.iter().map(lower_operator_to_rust);
    syn::parse2(quote! {
        ::core::iter::Iterator::flatten(
            ::core::iter::once_with(move || {
                #(#locals)*
                #return_stream
            })
        )
    })
}

fn lower_operator_to_rust(definition: &iterator_pipeline::Definition) -> TokenStream {
    let stream = &definition.stream;
    match &definition.operator {
        iterator_pipeline::Operator::Unit(operator) => {
            let binding = &operator.binding;
            quote! {
                let #stream = ::core::iter::once(#binding);
            }
        }
        iterator_pipeline::Operator::Scan(operator) => {
            let pattern = &operator.item_pattern;
            let source = &operator.source;
            let binding = &operator.binding;
            quote! {
                let #stream = ::core::iter::Iterator::map(
                    ::core::iter::IntoIterator::into_iter(#source),
                    move |#pattern| #binding,
                );
            }
        }
        iterator_pipeline::Operator::Join(operator) => {
            let input_stream = &operator.input_stream;
            let input_pattern = &operator.input_pattern;
            let item_pattern = &operator.item_pattern;
            let source = &operator.source;
            let binding = &operator.binding;
            let captures = cq::rust::bindings(input_pattern);
            let copies = captures
                .iter()
                .map(|v| quote!(let #v = ::core::clone::Clone::clone(&#v);));
            quote! {
                let #stream = ::core::iter::Iterator::flat_map(
                    #input_stream,
                    move |#input_pattern| {
                        ::core::iter::Iterator::map(
                            ::core::iter::IntoIterator::into_iter(#source),
                            move |#item_pattern| { #(#copies)* #binding },
                        )
                    },
                );
            }
        }
        iterator_pipeline::Operator::Filter(operator) => {
            let input_stream = &operator.input_stream;
            let input_pattern = &operator.input_pattern;
            let variables = cq::rust::bindings(input_pattern);
            let condition = &operator.condition;
            quote! {
                let #stream = ::core::iter::Iterator::filter_map(
                    #input_stream,
                    move |#[allow(unused_variables)] #input_pattern| {
                        if #condition { ::core::option::Option::Some((#(#variables,)*)) }
                        else { ::core::option::Option::None }
                    },
                );
            }
        }
        iterator_pipeline::Operator::Project(operator) => {
            let input_stream = &operator.input_stream;
            let input_pattern = &operator.input_pattern;
            let output = &operator.output;
            quote! {
                let #stream = ::core::iter::Iterator::map(
                    #input_stream,
                    move |#[allow(unused_variables)] #input_pattern| #output,
                );
            }
        }
        iterator_pipeline::Operator::Distinct(operator) => {
            let input_stream = &operator.input_stream;
            quote! {
                let #stream = {
                    let mut seen = ::std::collections::HashSet::new();
                    ::core::iter::Iterator::filter(
                        #input_stream,
                        move |row| seen.insert(::core::clone::Clone::clone(row)),
                    )
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_stream_omits_only_the_returned_terminal_distinct() {
        let source: iterator_pipeline::Pipeline = syn::parse_quote! {
            iter0 = scan (x,) in (rows) yield (x,);
            iter1 = filter iter0 as (x,) if (keep(x));
            iter2 = project iter1 as (x,) yield (x % 2);
            iter3 = distinct iter2;
            return iter3;
        };
        let actual = compile_stream_pipeline(&source).unwrap();
        assert!(pass_contracts::stream_pipeline_rust_is_exact(
            &source, &actual
        ));
        assert!(!pass_contracts::iterator_pipeline_rust_is_exact(
            &source, &actual
        ));
        let text = quote!(#actual).to_string();
        assert!(!text.contains("HashSet"));
        assert!(!text.contains("iter3"));
        assert!(text.contains("keep (x)"));
        assert!(text.contains("x % 2"));
        assert!(!pass_contracts::stream_pipeline_rust_is_exact(
            &source,
            &compile_iterator_pipeline(&source).unwrap()
        ));
        let mut wrong_source = source.clone();
        let iterator_pipeline::Operator::Scan(scan) = &mut wrong_source.definitions[0].operator
        else {
            unreachable!();
        };
        scan.source = syn::parse_quote!(other_rows);
        assert!(!pass_contracts::stream_pipeline_rust_is_exact(
            &wrong_source,
            &actual
        ));

        let mut wrong_return = source.clone();
        wrong_return.return_stream.stream = syn::parse_quote!(iter2);
        assert!(compile_stream_pipeline(&wrong_return).is_err());
        assert!(!pass_contracts::stream_pipeline_rust_is_exact(
            &wrong_return,
            &actual
        ));
        let mut missing_distinct = source;
        missing_distinct.definitions.pop();
        missing_distinct.return_stream.stream = syn::parse_quote!(iter2);
        assert!(compile_stream_pipeline(&missing_distinct).is_err());
    }

    #[test]
    fn set_sink_contract_checks_the_terminal_stream_and_collection_type() {
        let source: iterator_pipeline::Pipeline = syn::parse_quote! {
            iter0 = scan (x,) in (rows) yield (x,);
            iter1 = filter iter0 as (x,) if (keep(x));
            iter2 = project iter1 as (x,) yield (x % 2);
            iter3 = distinct iter2;
            return iter3;
        };
        let actual = compile_collect_pipeline(&source).unwrap();
        let expected: syn::Expr = syn::parse_quote! {{
            let iter0 = ::core::iter::Iterator::map(
                ::core::iter::IntoIterator::into_iter(rows),
                move |(x,)| (x,),
            );
            let iter1 = ::core::iter::Iterator::filter_map(
                iter0,
                move |#[allow(unused_variables)] (x,)| {
                    if keep(x) { ::core::option::Option::Some((x,)) }
                    else { ::core::option::Option::None }
                },
            );
            let iter2 = ::core::iter::Iterator::map(
                iter1,
                move |#[allow(unused_variables)] (x,)| x % 2,
            );
            let iter3: ::std::collections::HashSet<_> = ::core::iter::Iterator::collect(iter2);
            iter3
        }};
        assert_eq!(actual, expected);
        assert!(pass_contracts::collect_pipeline_rust_is_exact(
            &source, &actual
        ));
        assert!(!pass_contracts::collect_pipeline_rust_is_exact(
            &source,
            &compile_iterator_pipeline(&source).unwrap()
        ));

        for changed in [
            syn::parse_quote!(let iter3: ::std::vec::Vec<_> = ::core::iter::Iterator::collect(iter2);),
            syn::parse_quote!(let iter3: ::std::collections::HashSet<_> = ::core::iter::Iterator::collect(iter0);),
        ] {
            let mut target = expected.clone();
            let syn::Expr::Block(block) = &mut target else {
                unreachable!();
            };
            block.block.stmts[3] = changed;
            assert!(!pass_contracts::collect_pipeline_rust_is_exact(
                &source, &target
            ));
        }
    }
}
