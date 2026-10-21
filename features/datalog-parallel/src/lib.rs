//! Select `storage_rewrites = [cq_datalog_parallel::shard_storage];`,
//! `round = [cq_datalog_parallel::partition(threshold = 1024, grain = 64)];`
//! and `print = cq_datalog_parallel::print_rounds;` in the Datalog pipeline.
//! Checked Storage/Round refinements precede all CQ hole expansion.
//!
//! # DOOP engine features
//!
//! The `datalog-parallel-doop` package retains the `doop-parallel` CLI. Default
//! Cargo features enable all six engines: `mini`, `mini-fx`, `ascent`,
//! `mini-par`, `mini-par-fx`, and `ascent-par`. `--check` requires all six
//! comparison features and compares full tuple sets against sequential Ascent.
//! From `features/datalog-parallel`, select an isolated engine or the complete
//! default comparison:
//!
//! ```text
//! cargo run --release --locked -p datalog-parallel-doop --no-default-features --features mini-par -- /tmp/doop-data/cassandra --engine mini-par --threads 15
//! cargo run --release --locked -p datalog-parallel-doop -- /tmp/doop-data/cassandra --check --threads 15
//! ```
//!
//! Parallel implementations live in `doop/engines/{mini-par,mini-par-fx,ascent-par}`
//! as `doop-mini-par`, `doop-mini-par-fx`, and `doop-ascent-par`. Optional
//! sequential features depend directly on `doop-mini`, `doop-mini-fx`, and
//! `doop-ascent` from the base Datalog workspace, never on its CLI package.
//! Both CLIs share facts, interning, loading, engine-source macros, and the
//! common harness through `doop-shared` in `features/datalog/doop/shared`.
//! `phase-timing` forwards instrumentation only to enabled engines, without
//! pulling in otherwise disabled implementations. Build timing binaries
//! without instrumentation and run each engine in a separate process.
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
