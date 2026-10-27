//! Select `storage_rewrites = [cq_datalog_parallel::shard_storage];`,
//! `round = [cq_datalog_parallel::partition(threshold = 1024, grain = 64)];`
//! and `print = cq_datalog_parallel::print_rounds;` in the Datalog pipeline.
//! Checked Storage/Round refinements precede all CQ hole expansion.
//! Opt into internally parallel keyed IDB layouts with
//! `storage_rewrites = [cq_datalog_parallel::shard_storage(layouts(sharded))];`.
//! The default shards only admission; unit-key and EDB layouts stay native.
//! Select sorted-run keyed IDB layouts with `shard_storage(layouts(runs))`,
//! sorted-run admission with `shard_storage(seen(runs))`, or both with
//! `shard_storage(layouts(runs), seen(runs))`. These options are independent;
//! `layouts(sharded)` and `layouts(runs)` are mutually exclusive. Run layouts
//! require ordered keys and payloads; run admission requires ordered rows.
//! Run layouts accept only the default source layout hasher
//! `BuildHasherDefault<cq_datalog::rustc_hash::FxHasher>`; custom/keyed layout
//! hashers are rejected rather than silently erased. Unit-key and EDB layouts
//! remain native, and omitting `seen(runs)` retains sharded admission.
//! Eligible rule holes replace the entire standard `sequence -> sequential ->
//! translate_pull` tail with the registered Structures `print_push` terminal,
//! feeding the task collector directly. The checker preserves the preceding
//! queue, lowering ordinal, discharged facts and terminal registry exactly.
//! Custom Sequence lowering, grading, rewrites or arguments remain untouched,
//! as do frozen-key holes and already-authored Structures push pipelines.
//! Negated atoms read completed borrowed key sets through CQ anti-joins. Key
//! sets are built once per stratum and shared by tasks, never rebuilt per call.
//!
//! # Validation commands
//!
//! Run from `features/datalog-parallel`, one command at a time:
//!
//! ```text
//! cargo test --workspace --locked
//! cargo test --workspace --all-features --locked
//! cargo test --locked -p cq-datalog-parallel --features counters --test counters -- --nocapture
//! cargo run --release --locked -p datalog-parallel-oracle -- golden
//! cargo run --release --locked -p datalog-parallel-oracle --no-default-features --features mini-par,counters -- counters
//! ```
//!
//! # DOOP engine features
//!
//! The `datalog-parallel-doop` package retains the `doop-parallel` CLI. Default
//! Cargo features enable all six engines: `mini`, `mini-keyed`, `ascent`,
//! `mini-par`, `mini-par-keyed`, and `ascent-par`. `--check` requires all six
//! comparison features and compares full tuple sets against sequential Ascent.
//! From `features/datalog-parallel`, select an isolated engine or the complete
//! default comparison:
//!
//! ```text
//! cargo run --release --locked -p datalog-parallel-doop --no-default-features --features mini-par -- /tmp/doop-data/cassandra --engine mini-par --threads 15
//! cargo run --release --locked -p datalog-parallel-doop -- /tmp/doop-data/cassandra --check --threads 15
//! ```
//!
//! Parallel implementations live in `doop/engines/{mini-par,mini-par-keyed,ascent-par}`
//! as `doop-mini-par`, `doop-mini-par-keyed`, and `doop-ascent-par`. Optional
//! sequential features depend directly on `doop-mini`, `doop-mini-keyed`, and
//! `doop-ascent` from the base Datalog workspace, never on its CLI package.
//! Both CLIs share facts, interning, loading, engine-source macros, and the
//! common harness through `doop-shared` in `features/datalog/doop/shared`.
//! `phase-timing` forwards instrumentation only to enabled engines, without
//! pulling in otherwise disabled implementations. Build timing binaries
//! without instrumentation and run each engine in a separate process.
//!
//! Sharding and run admission preserve the Datalog default Fx hasher.
//! CQ holes discharge result canonicalization to admission and negation key
//! sets rather than allocating separate answer-dedup sets. Fx is unkeyed: for
//! adversarial inputs select `cq_datalog_hasher::hasher(keyed)` before
//! `shard_storage` and `cq_datalog_hasher::hasher_round(keyed)` before `partition`
//! when not selecting `layouts(runs)`. Hash iteration order is not an output guarantee.
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
