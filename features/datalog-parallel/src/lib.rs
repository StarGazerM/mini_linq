//! Select `storage_rewrites = [cq_datalog_parallel::shard_storage];`,
//! `round = [cq_datalog_parallel::partition(threshold = 1024, grain = 64)];`
//! and `print = cq_datalog_parallel::print_rounds;` in the Datalog pipeline.
//! Checked Storage/Round refinements precede all CQ hole expansion.
use proc_macro::TokenStream;

#[proc_macro]
pub fn shard_storage(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_datalog_round::storage::Program>(
        input.into(),
        cq_datalog_partition::compile_storage_with,
    )
    .into()
}

#[proc_macro]
pub fn partition(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_round::Program, cq_datalog_partition::ShardedProgram>(
        input.into(),
        cq_datalog_partition::compile_sharded_with,
    )
    .into()
}

#[proc_macro]
pub fn print_rounds(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_datalog_partition::ShardedProgram, syn::Expr>(
        input.into(),
        |source, arguments| {
            if !arguments.is_empty() {
                return Err(syn::Error::new_spanned(
                    arguments,
                    "print_rounds takes no arguments",
                ));
            }
            cq_datalog_rayon_print::compile(source)
        },
    )
    .into()
}
