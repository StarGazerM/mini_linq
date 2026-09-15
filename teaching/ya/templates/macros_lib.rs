//! The selected macro frontend; Rust aliases choose the stage continuations.
mod access_input;
mod entry;
use access_input::AccessInput;
use entry::stage;
use proc_macro::TokenStream;

#[proc_macro]
pub fn graph_query(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::relational_plan)),
        cq_plan::graph::compile,
    )
}

#[proc_macro]
pub fn result_indexes(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::index_requirements)),
        cq_index::result::compile,
    )
}

#[proc_macro]
pub fn materialized(input: TokenStream) -> TokenStream {
    stage(
        input,
        None,
        |input: &AccessInput<cq_ir::index_requirements::Module>| {
            cq_physical::materialized::compile_materialized(&input.source, &input.access)
        },
    )
}

/// The same selected computation inside an anonymous Rust closure.
#[proc_macro]
pub fn materialized_query(input: TokenStream) -> TokenStream {
    stage(
        input,
        None,
        |input: &AccessInput<cq_ir::index_requirements::Module>| {
            cq_physical::materialized::compile_materialized_callable(&input.source, &input.access)
        },
    )
}

#[proc_macro]
pub fn collect_access(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::collect_pipeline)),
        cq_pull::compile,
    )
}

#[proc_macro]
pub fn collect_pipeline(input: TokenStream) -> TokenStream {
    stage(input, None, cq_pipeline::collect::compile)
}
