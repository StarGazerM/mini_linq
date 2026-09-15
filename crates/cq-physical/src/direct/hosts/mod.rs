//! Host construction supplies ordinary Rust scope around shared RA lowering.
pub(super) mod observed;
pub(super) mod plain;
use super::{PhysicalOptions, layout::Layout, wiring::Wiring};
use cq_ir::{relational_plan as ra, symbol_name};
use proc_macro2::TokenStream;
use quote::quote;

fn parameters(source: &ra::Module, layout: &Layout, options: &PhysicalOptions) -> Vec<TokenStream> {
    source
        .program
        .inputs()
        .map(|input| {
            let name = &layout.inputs[&symbol_name(&input.name)];
            let arity = cq_ir::Column::from(input.arity());
            let provider = &options.inputs[&symbol_name(&input.name)];
            quote!(#name: &#provider! { type #arity })
        })
        .collect()
}
fn row(layout: &Layout) -> TokenStream {
    let types = layout
        .output_columns
        .iter()
        .map(|_| quote!(::std::primitive::i64));
    quote!((#(#types,)*))
}
fn answer(source: &ra::Module, layout: &Layout, wiring: &Wiring) -> TokenStream {
    let view = layout.view(&source.plan.output.input);
    let output = &view.name;
    let columns = &layout.output_columns;
    let row = row(layout);
    let provider = wiring.provider(&view.provider);
    quote! {
        let __ml_answer: ::std::collections::BTreeSet<#row> = #provider! { scan (&#output) }
            .map(|__ml_row| (#(__ml_row[#columns],)*)) .collect();
    }
}
