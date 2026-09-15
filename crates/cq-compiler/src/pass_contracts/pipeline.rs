//! Exact ordinary-Rust realizations of Pipeline iterator, set and stream endpoints.

use cq_ir::{cq, iterator_pipeline};
use quote::quote;

/// Whether `target` is the exact ordinary-Rust implementation denoted by the
/// named operator plan.
///
/// This relation independently constructs the required target shape. It does
/// not call the lowering under contract: it checks the fixed deferred wrapper,
/// one local per definition in source order, every exact adapter initializer,
/// and the recorded final stream.
pub(crate) fn iterator_pipeline_rust_is_exact(
    source: &iterator_pipeline::Pipeline,
    target: &syn::Expr,
) -> bool {
    if !iterator_pipeline::contract::well_formed(source) {
        return false;
    }
    let locals = source
        .definitions
        .iter()
        .map(expected_rust_local)
        .collect::<Vec<_>>();
    let return_stream = &source.return_stream.stream;
    let Ok(expected) = syn::parse2::<syn::Expr>(quote! {
        ::core::iter::Iterator::flatten(
            ::core::iter::once_with(move || {
                #(#locals)*
                #return_stream
            })
        )
    }) else {
        return false;
    };
    target == &expected
}

/// Independently check the eager set endpoint. Only the returned terminal
/// distinct operator is implemented by collection; preceding operators retain
/// their exact adapter expressions and source order.
pub(crate) fn collect_pipeline_rust_is_exact(
    source: &iterator_pipeline::Pipeline,
    target: &syn::Expr,
) -> bool {
    if !iterator_pipeline::contract::well_formed(source) {
        return false;
    }
    let Some((last, prefix)) = source.definitions.split_last() else {
        return false;
    };
    let iterator_pipeline::Operator::Distinct(distinct) = &last.operator else {
        return false;
    };
    let locals = prefix.iter().map(expected_rust_local);
    let input = &distinct.input_stream;
    let result = &source.return_stream.stream;
    let expected: syn::Expr = syn::parse_quote! {{
        #(#locals)*
        let #result: ::std::collections::HashSet<_> = ::core::iter::Iterator::collect(#input);
        #result
    }};
    target == &expected
}

/// Independently check the support-stream endpoint. The local Pipeline contract
/// establishes a sole returned terminal `Distinct`; only its implementation is
/// omitted. The result can repeat values but has exactly the source set support.
pub(crate) fn stream_pipeline_rust_is_exact(
    source: &iterator_pipeline::Pipeline,
    target: &syn::Expr,
) -> bool {
    if !iterator_pipeline::contract::well_formed(source) {
        return false;
    }
    let Some((last, prefix)) = source.definitions.split_last() else {
        return false;
    };
    let iterator_pipeline::Operator::Distinct(distinct) = &last.operator else {
        return false;
    };
    let locals = prefix.iter().map(expected_rust_local);
    let input = &distinct.input_stream;
    let expected: syn::Expr = syn::parse_quote! {
        ::core::iter::Iterator::flatten(
            ::core::iter::once_with(move || {
                #(#locals)*
                #input
            })
        )
    };
    target == &expected
}

fn expected_rust_local(definition: &iterator_pipeline::Definition) -> proc_macro2::TokenStream {
    let stream = &definition.stream;
    match &definition.operator {
        iterator_pipeline::Operator::Unit(operator) => {
            let binding = &operator.binding;
            quote!(let #stream = ::core::iter::once(#binding);)
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
