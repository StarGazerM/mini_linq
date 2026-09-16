//! Host construction supplies ordinary Rust scope around shared RA lowering.
pub(super) mod observed;
pub(super) mod plain;
use super::{RelationOptions, layout::Layout, wiring::Wiring};
use cq_ir::{relational_plan as ra, symbol_name};
use proc_macro2::TokenStream;
use quote::quote;

fn parameters(source: &ra::Module, layout: &Layout, options: &RelationOptions) -> Vec<TokenStream> {
    source
        .program
        .inputs()
        .map(|input| {
            let name = &layout.inputs[&symbol_name(&input.name)];
            let types = input
                .columns
                .iter()
                .map(|column| crate::rust_bindings::qualify_type(&column.ty));
            let provider = &options.inputs[&symbol_name(&input.name)];
            quote!(#name: &#provider! { type (#(#types,)*) })
        })
        .collect()
}
fn row(layout: &Layout) -> TokenStream {
    let types = &layout.output_types;
    quote!((#(#types,)*))
}
fn answer(source: &ra::Module, layout: &Layout, wiring: &Wiring) -> TokenStream {
    let view = layout.view(&source.plan.output.input);
    let output = &view.name;
    let row = row(layout);
    let provider = wiring.provider(&view.provider);
    let fields =
        source.plan.output.head.args.iter().map(|expression| {
            super::expressions::output(expression, view, quote!(__ml_row), wiring)
        });
    quote! {
        let __ml_answer: ::std::collections::BTreeSet<#row> = #provider! { scan (&#output) }
            .map(|__ml_row| (#(#fields,)*)) .collect();
    }
}
