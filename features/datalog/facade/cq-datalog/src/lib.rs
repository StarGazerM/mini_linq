//! Insert-only Datalog implemented above the ordinary MiniLinq compiler.
//!
//! Each stage has its own checked language: Source, Strata, Incremental,
//! Storage and Round. The terminal printer executes ordinary CQ holes against
//! borrowed hosts and admits each candidate through canonical set insertion.
//! Datalog owns its pipeline configuration without changing core CQ planning.
//!
//! Consumers declare direct `mini-linq` and `cq-storage` dependencies, just as
//! ordinary generated CQ programs do. Fx is supplied through this facade;
//! no direct `rustc-hash` dependency is needed for the default pipeline.
//!
//! ```
//! let edges = [(0_i32, 1_i32), (1, 2), (2, 3)];
//! let seeds = [(0_i32,)];
//! let result = cq_datalog::datalog! {
//!     struct Reachability;
//!     relation Edge(x: i32, y: i32) = (edges);
//!     relation Seed(x: i32) = (seeds);
//!     relation Reach(x: i32);
//!     output Reach;
//!     Reach(*x) :- Seed(x);
//!     Reach(*y) :- Reach(x), Edge(x,y);
//! };
//! assert_eq!(result.into_iter().collect::<std::collections::BTreeSet<_>>(),
//!     [(0,), (1,), (2,), (3,)].into_iter().collect());
//! ```
//!
//! An optional named `pipeline { ... }` selects Datalog phases:
//! `surface = [..]; stratify = pass; incremental = pass; storage = pass;`
//! `storage_rewrites = [..]; fork = pass; round = [..]; print = pass;`.
//! Omitted phases use the base defaults, including `surface_if_let`.
//! `surface = [];` explicitly disables surface expansion.
//! Select the embedded CQ compiler with `cq = pipeline { plan = pass; ... };`
//! inside that configuration. Its resolved queue is recorded on each hole.
//! Embedded holes default to `sequence`, `sequential`, and `translate_pull`
//! after Structures representation and staging. Custom Sequence terminals,
//! including `print_hole`, explicitly select `sequence` and `grade` in their
//! CQ configuration; terminal-only custom choices retain Structures input.
//! [`DatalogConfiguration`] exposes the same resolution to Rust callers.
//!
//! Each rule hole records `consumer canonical`: its result belongs to exactly
//! one checked admission owner for its head relation. Frozen-key holes have
//! their own canonical key-set consumer. Storage certificates establish
//! `host_set H discharged` for certified positive hosts: canonical EDBs,
//! admitted IDB all/delta rows, and their maintained layouts. Certificates may
//! cover a subset of hosts; an uncertified host receives no discharged fact.
//! Provider types are not whitelisted. Checked scan substitutions preserve the
//! certificate for the renamed input.
//! Round checks owners and host provenance directly from the supplied records;
//! it never regenerates Fork's output to justify these facts. Opaque providers
//! must still obey their recorded collection, membership and maintenance laws.
//!
//! Raw CQ entry remains fact-free; Fork records facts in the selected lower
//! argument packet. `Hole.lower` records that invocation's ordinal, and Round
//! checks its parsed context exactly equals `Hole.facts`. Fork's explicit
//! `lower = N` argument selects a custom lowering alias by ordinal; otherwise
//! the queue must have one path ending in `lower`.
//! Checked scan renaming updates both facts and the packet, and facts naming
//! non-read inputs are rejected before erasure.
//! Once recorded in Physical, they travel in an exact explicit
//! `context { consumer = canonical, host_set H discharged, ... };` envelope.
//! Reentering a factful CQ IR requires that envelope: missing or extra facts
//! are rejected, and matching pass arguments do not replace it. Per-hole
//! rewrites and both pull and push terminals preserve this checked boundary.
//! Sharded execution also preserves owners and facts; candidate buffers do not
//! become new set boundaries, and shard admission remains responsible for
//! full-row uniqueness before publication.
//! Parallel execution may record `ShardCertificate.self_dedup = Some(per_task)`
//! to deduplicate candidates within each task before global admission.
//! Partitioned scans repeat the policy in `Execution.self_dedup`, checked only
//! with `rayon_sharded`; a rule without a partitionable scan is one task.
//! Frozen-key holes are excluded. The task-local collector uses `HashTable<usize>`
//! indices into an owned `Vec`, shares the admission hasher, preserves first
//! occurrence order, and returns that vector without row cloning or CQ
//! `__cq_seen` state. The collector alone does not eliminate retained pull
//! result materialization; checked push selection streams into it directly.
//! These checked realization records erase without changing the CQ or context
//! facts. The optimization is never the canonical owner: cross-task duplicates
//! and old rows still require global admission. Sequential execution has no
//! such annotation or local dedup state.
//!
//! Opt into bounded frozen positive-plus-negation materialization with
//! `arguments { hoist_invariants; };` before the named pipeline or program.
//! The checked transformation runs between Strata and Incremental, before
//! semi-naive expansion. Direct `passes::incremental!` entries accept the same
//! argument. The `hoist-invariants` Cargo feature enables this policy by default;
//! ordinary CPU builds retain the unmodified scheduling policy.
//!
//! Select streaming push holes with
//! `cq = pipeline { print = ::cq_datalog::passes::print_push; };`.
//! Rule answers are passed directly to canonical admission, and negation keys
//! directly to their key-set provider, without a printer-owned result buffer.
//! The default registry recognizes both pull and push terminals; custom paths
//! declare their interface with `terminals = [my::printer => push];`.
//! A checked Round rewrite may change a hole's interface together with its
//! pipeline and lowering ordinal, but the selected terminal must match the
//! unchanged registry. It cannot add registry entries or alter the hole's
//! context facts, canonical owner, or CQ meaning.
//! Parallel partition selects push for rule holes whose terminal is exactly
//! registered `::mini_linq::passes::translate_pull` with empty arguments, provided
//! `::cq_datalog::passes::print_push` is also registered. It preserves custom
//! terminals, authored terminal arguments, and all frozen-key holes. This
//! applies with or without task-local deduplication, including no-scan rules.
//! The partition checker restores only that allowed terminal/interface change
//! before requiring exact erased correspondence; it never changes the registry.
//! Parallel and phase-timed execution retain their explicit candidate storage.
//!
//! Datalog defaults to unkeyed Fx hashing for layout maps, IDB membership,
//! EDB canonicalization, and negation key sets. Checked context facts eliminate
//! dedup representations in the generated rule holes, so the default queue
//! installs no CQ dedup retype rewrite.
//! This assumes trusted inputs: Fx is not collision-attack resistant.
//! For adversarial input, select keyed hashing at both Datalog boundaries:
//! ```ignore
//! pipeline {
//!     storage_rewrites = [cq_datalog_hasher::hasher(keyed)];
//!     round = [cq_datalog_hasher::hasher_round(keyed)];
//! }
//! ```
//! Explicit `hasher(fx)` and `hasher_round(fx)` remain available. Do not add
//! `retype(dedup => ...)` to the discharged hole pipeline: it has no dedup
//! representation to retype. MiniLinq core defaults are unchanged.
//!
//! Resume authored IR at its next pass, not at a separate front-end macro:
//! `cq_datalog::passes::incremental! { pipeline {
//! cq_datalog::passes::storage, cq_datalog::passes::fork,
//! cq_datalog::passes::print_rounds }; <strata program> }`.
//! For authored Incremental text, enter [`passes::storage!`] instead.
//! Every direct entry accepts `arguments { ... }; pipeline { ... }; <IR>`.
//! Storage records full and delta layouts separately.
//! Full layouts receive per-admitted-row insertions. Each demanded delta
//! layout is batch-built once at seed publication and once at every subsequent
//! publication, replacing the previous run even when the new delta is empty.
//! The next round's holes share that immutable grouped host rather than
//! building private indexes. Keyed delta reads and later zero-key probes use
//! these layouts; only a first-position unkeyed delta remains a `Vec` scan.
//! Neither layout selection nor publication reorders written atoms or guards.
//!
//! The opt-in `phase-timing` Cargo feature is for profiling, not normal builds.
//! It prints per-round `evaluation_ns`, `admission_ns`, and `publication_ns` to
//! stderr. The named default fork records `timing = phases;` in Round.
//! To separate those phases it materializes candidates; use an uninstrumented
//! build for regression measurements. Direct pass entries select timing with
//! explicit fork arguments, independently of the named entry's defaults.
//!
//! `output A, B;` returns a tuple of owned `Vec::IntoIter`s in the written
//! output order, even when dependency order differs. One output still returns
//! its owned iterator directly. Outputs must be distinct declared EDBs or IDBs.
//! EDB outputs are deduplicated and cloned into owned rows without consuming
//! their host bindings. An EDB-only program does not require any rules.
//! Literals in positive atoms use CQ's filtered-relation occurrences, including
//! string literals against `String` columns. All heads, including recursive
//! heads, accept Rust expressions and body `let` bindings (for example
//! `Path(*x, *z, *length + 1)` with a preceding `if *length < 10` guard).
//! A `String` head constant is written `"java.lang.String".to_owned()`, while
//! an `&'static str` column can return a literal directly.
//! Bodies also accept `for pattern in expression`, using an irrefutable Rust
//! pattern and a pure finite `IntoIterator`, for example
//! `Reach(next) :- Reach(x), if *x < 4, for next in [*x + 1, *x + 1];`.
//! Generated bindings are available to later clauses and heads, as with `let`;
//! duplicates collapse at Boolean insertion. Generator clauses preserve the
//! written opaque Rust clause order. A panic while constructing or advancing
//! an iterator produces no denoted result.
//! Refutable patterns use `if let pattern = expression`, for example
//! `Value(x) :- Optional(o), if let Some(x) = *o;`. A failed match drops the
//! valuation; a successful match makes every pattern binding available to
//! later clauses and recursive or non-recursive heads. Patterns without
//! bindings, such as `if let Kind::A = *kind`, act as filters.
//! This is Datalog-only sugar for `for (bindings, ...) in match expression {
//! pattern => Some((bindings, ...)), _ => None }` (using qualified `Option`
//! variants); no bindings use `Some(())` and `for ()`. It inherits generators'
//! written-order and placement constraints. A panic in the expression is
//! bottom, not a failed match. The default surface pass lowers it before the
//! Source contract; Source and every subsequent checked IR contain only the
//! ordinary `for` form.
//! Termination is the caller's responsibility: copying positive-atom columns
//! preserves a finite domain, but computed recursive values can grow without
//! bound even when every individual generator is finite. Evaluation runs until
//! no new tuples are admitted; a divergent run
//! does not produce a denoted result. Rust expressions must be total and pure
//! for the least-fixpoint interpretation to apply.
//! Rust keywords used as logical names must be renamed (`type_`, `super_`,
//! `ret`) or use a legal raw identifier such as `r#type`.
//!
//! `!Modifier("abstract", method)` filters the completed input by its constant
//! columns and projects only the bound variable columns. Identical projections
//! and filters share one borrowed key set per stratum, outside recursive rounds.
//! Wildcards are omitted; an all-constant/wildcard atom uses an existence key.
//! Closed Rust expressions such as `symbol("abstract")` are also supported;
//! parenthesize a captured constant name to distinguish it from a logical variable.
//! Key-hole literals remain in their atom and become occurrence constants.
//! Nonliteral constant expressions compare after scanning the completed relation,
//! so an owned value such as `String::from("static")` is not moved through a
//! fanout iterator closure. Only borrowed variable columns enter the key set.
//! Negative constants cannot reference rule-local bindings or opaque macros.
//!
//! # Oracle and performance commands
//!
//! Run from this feature workspace, one command at a time:
//!
//! ```text
//! cargo test --workspace --locked
//! cargo run --release --locked -p datalog-oracle -- golden
//! cargo run --release --locked -p datalog-oracle -- golden --write
//! cargo run --release --locked -p datalog-oracle --features counters -- counters
//! cargo run --release --locked -p datalog-oracle -- bench
//! cargo test --workspace --all-features --locked
//! cargo run --release --locked --features counters --example layout_mismatch
//! ```
//!
//! CSV goldens are generated by sequential Ascent, not by this compiler.
//! The default golden command compares complete canonical CSV bytes; only the
//! explicit `--write` flag replaces checked-in files. Correctness tests compare
//! full answer sets against Ascent and a feature-local naive evaluator.
//! Golden encoding and byte comparison use `cq-testkit`'s public helpers.
//! The frozen testkit gates them behind `duckdb`, so oracle builds also compile
//! bundled DuckDB even though the Datalog oracle itself uses sequential Ascent.
//!
//! Counters are opt-in; `bench` rejects counters-enabled builds. Counters describe
//! the ordinary CQ runtime: Datalog-owned fast-hashed full-layout insertions,
//! delta-run batch builds, and canonical-set writes are not counted as core
//! index builds. Compatible borrowed keys probe both grouped host kinds
//! natively; counter regressions require zero per-hole fallback builds.
//! `layout_mismatch` measures native host probing versus a fallback for the
//! other column, checking identical complete answers in both cases.
//! Benchmarks use sequential Ascent and five-run medians; inputs are loaded
//! before timing, evaluation includes index construction, and canonical answer
//! sorting occurs after timing. Build first and do not compile concurrently
//! with a timing run.
//! Rule holes retain their projected heads; widening them is unnecessary.
//! Checked `consumer canonical` facts assign the returned set boundary to
//! admission, eliminating the per-hole `__cq_seen` set and unnecessary
//! `return canon`. Discharged host facts also let CQ infer uniqueness from
//! canonical inputs. Internal canonicalization needed by intermediate operators
//! remains; this is not an instruction to erase every deduplication operation.
//!
//! # DOOP engine features
//!
//! The `datalog-doop` package retains the `doop` CLI. Its default Cargo features
//! enable all three engines: `mini`, `mini-keyed`, and `ascent`. For a single-engine
//! build, disable defaults and explicitly select the matching engine feature:
//!
//! ```text
//! cargo run --release --locked -p datalog-doop --no-default-features --features mini -- /tmp/doop-data/cassandra --engine mini
//! cargo run --release --locked -p datalog-doop -- /tmp/doop-data/cassandra --check
//! ```
//!
//! `--check` requires all three comparison features (`mini,mini-keyed,ascent`);
//! selecting only one engine cannot provide the full comparison.
//! `doop/shared` (`doop-shared`) owns facts, interning, loading, the common
//! harness, and engine-source macros without instantiating a heavy engine.
//! Separate `doop/engines/{mini,mini-keyed,ascent}` crates (`doop-mini`,
//! `doop-mini-keyed`, and `doop-ascent`) instantiate the engines. CLI features
//! select those optional dependencies; `phase-timing` forwards instrumentation
//! only to enabled engines rather than enabling additional engines.
//! Fetch published facts with `python3 doop/fetch.py cassandra` from
//! `features/datalog`; generated facts remain outside the repository.

pub use cq_datalog_configuration::DatalogConfiguration;
pub use cq_datalog_macros::datalog;
pub use cq_lang_datalog_incremental as incremental;
pub use cq_lang_datalog_round as round;
pub use cq_lang_datalog_source as source;
pub use cq_lang_datalog_storage as storage;
pub use cq_lang_datalog_strata as strata;
pub use cq_rewrites;
pub use rustc_hash;

/// Codegen partitioning helpers used by the terminal printer.
///
/// Rustc ignores modules nested in functions when assigning codegen units.
/// These module-level definitions give captured printer bodies distinct homes.
#[doc(hidden)]
pub mod __codegen {
    macro_rules! partitions {
        ($($name:ident),* $(,)?) => {$(
            pub mod $name {
                #[inline(never)]
                pub fn run<T, R>(storage: T, evaluate: impl FnOnce(T) -> R) -> R {
                    evaluate(storage)
                }
            }
        )*};
    }
    partitions!(
        partition_0,
        partition_1,
        partition_2,
        partition_3,
        partition_4,
        partition_5,
        partition_6,
        partition_7,
        partition_8,
        partition_9,
        partition_10,
        partition_11,
        partition_12,
        partition_13,
        partition_14,
        partition_15,
    );
}

/// Direct checked entries, each consuming its source IR and a continuation.
pub mod passes {
    pub use cq_datalog_macros::{
        fork, incremental, print_hole, print_rounds, storage, stratify, surface_if_let,
    };
    pub use own_push::push_into as print_push;
}
