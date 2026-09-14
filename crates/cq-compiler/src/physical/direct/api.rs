//! Query methods and instrumentation, independent of operator realization.
use cq_ir::{relational_plan as ra, symbol_name};
use proc_macro2::TokenStream;
use quote::quote;

use super::{PhysicalOptions, layout::Layout, wiring::Wiring};

pub(super) fn emit(
    source: &ra::Module,
    layout: &Layout,
    statements: &[(TokenStream, Option<TokenStream>)],
    options: &PhysicalOptions,
    wiring: &Wiring,
) -> syn::Result<syn::File> {
    let storage = &options.storage;
    let output = &layout.view(&source.plan.output.input).name;
    let output_columns = &layout.output_columns;
    let parameters: Vec<_> = source
        .program
        .inputs()
        .map(|input| {
            let name = &layout.inputs[&symbol_name(&input.name)];
            let arity = cq_ir::Column::from(input.arity());
            let provider = &options.inputs[&symbol_name(&input.name)];
            quote!(#name: &#provider! { storage #storage; type #arity })
        })
        .collect();
    let output_types = output_columns.iter().map(|_| quote!(::std::primitive::i64));
    let row = quote!((#(#output_types,)*));
    let visibility = &source.program.visibility;
    let program = &source.program.name;
    let stats = quote::format_ident!("{}Stats", symbol_name(program));
    let provider = wiring.provider(&layout.view(&source.plan.output.input).provider);
    let plain_bindings = wiring.emit(options, false);
    let observed_bindings = wiring.emit(options, true);
    let plain_statements = statements.iter().map(|(stmt, _)| stmt);
    let observed_statements = statements.iter().map(|(stmt, stats)| quote!(#stmt #stats));
    let answer = quote! {
        let __ml_answer: ::std::collections::BTreeSet<#row> = #provider! { scan (&#output) }
            .map(|__ml_row| (#(__ml_row[#output_columns],)*)) .collect();
    };
    syn::parse2(quote! {
        #visibility struct #program;
        #[derive(Debug, Default)]
        #[allow(dead_code)]
        #visibility struct #stats {
            pub steps: ::std::vec::Vec<(&'static ::core::primitive::str, ::core::primitive::usize, #storage::Work)>,
            pub output: #storage::Work,
        }
        impl #program {
            pub fn run(#(#parameters,)* __ml_block_rows: ::std::num::NonZeroUsize) -> ::std::vec::Vec<#row> {
                #plain_bindings
                #(#plain_statements)*
                #answer
                __ml_answer.into_iter().collect()
            }
            pub fn run_with_stats(#(#parameters,)* __ml_block_rows: ::std::num::NonZeroUsize)
                -> (::std::vec::Vec<#row>, #stats)
            {
                let __ml_work = &#storage::Counters::default();
                let mut __ml_stats = #stats::default();
                #observed_bindings
                #(#observed_statements)*
                #answer
                __ml_stats.output = __ml_work.take();
                (__ml_answer.into_iter().collect(), __ml_stats)
            }
        }
    })
}
