//! Independent observations for endpoint correspondence; no emission calls.
use cq_ir::{cq, iterator_pipeline};
use quote::quote;

pub(crate) fn expected_rust_local(
    definition: &iterator_pipeline::Definition,
) -> proc_macro2::TokenStream {
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
            let retained =
                cq::rust::known_uses(binding, &captures.iter().map(cq_ir::symbol_name).collect());
            let copies = captures
                .iter()
                .filter(|v| retained.contains(&cq_ir::symbol_name(v)))
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
                    let mut seen = mini_linq::distinct_state! { operator #stream; new };
                    ::core::iter::Iterator::filter(
                        #input_stream,
                        move |row| mini_linq::distinct_state! { operator #stream; insert(seen, row) },
                    )
                };
            }
        }
    }
}
