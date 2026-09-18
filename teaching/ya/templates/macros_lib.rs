//! The selected macro frontend; Rust aliases choose the stage continuations.
mod access_input;
mod entry;
use access_input::AccessInput;
use entry::stage;
use proc_macro::TokenStream;

#[proc_macro]
pub fn cq_to_ra_hypergraph(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_index_policy)),
        cq_query_lowering::hypergraph::compile,
    )
}

#[proc_macro]
pub fn result_index_demands(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::choose_relation_lowering)),
        cq_index_policy::result_demands::compile,
    )
}

#[proc_macro]
pub fn materialized_hash_program(input: TokenStream) -> TokenStream {
    stage(
        input,
        None,
        |input: &AccessInput<cq_ir::index_requirements::Module>| {
            cq_relation_lowering::materialized_hash::compile(&input.source, &input.access)
        },
    )
}

/// The same selected computation inside an anonymous Rust closure.
#[proc_macro]
pub fn materialized_hash_callable(input: TokenStream) -> TokenStream {
    stage(
        input,
        None,
        |input: &AccessInput<cq_ir::index_requirements::Module>| {
            cq_relation_lowering::materialized_hash::compile_callable(&input.source, &input.access)
        },
    )
}

#[proc_macro]
pub fn access_to_set(input: TokenStream) -> TokenStream {
    stage(
        input,
        Some(syn::parse_quote!(mini_linq::pipeline_to_set)),
        cq_access_lowering::to_pipeline::compile,
    )
}

#[proc_macro]
pub fn pipeline_to_set(input: TokenStream) -> TokenStream {
    stage(input, None, cq_pipeline_lowering::collect_set::compile)
}
