use cq_ir::{cq, iterator_pipeline};
use proc_macro2::TokenStream;
use quote::quote;

#[cfg(any(feature = "iterator", feature = "stream", feature = "exists"))]
pub(crate) fn deferred_iterator(
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

pub(crate) fn lower_operator_to_rust(definition: &iterator_pipeline::Definition) -> TokenStream {
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
