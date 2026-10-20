//! Insert-only Datalog implemented above the ordinary MiniLinq compiler.
//!
//! Each stage has its own checked language: Source, Strata, Incremental,
//! Storage and Round. The terminal printer executes ordinary CQ holes against
//! borrowed hosts and admits each candidate through canonical set insertion.
//! Datalog owns its pipeline configuration without changing core CQ planning.
//!
//! Consumers declare direct `mini-linq` and `cq-storage` dependencies, just as
//! ordinary generated CQ programs do.
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
//! [`DatalogConfiguration`] exposes the same resolution to Rust callers.
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
//! Constants in positive atoms use the ordinary CQ selection rules, including
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
//! Key-hole constant comparisons run after scanning the completed relation, so
//! an owned constant such as `String::from("static")` is not moved through
//! a fanout iterator closure. Only borrowed variable columns enter the key set.
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
//! Rule holes retain their projected heads: returning all body variables still
//! triggers the frozen core's canonical-return deduplication for borrowed hosts,
//! so widening those rows does not remove the per-call canonical set.

pub use cq_datalog_configuration::DatalogConfiguration;
pub use cq_datalog_macros::datalog;
pub use cq_lang_datalog_incremental as incremental;
pub use cq_lang_datalog_round as round;
pub use cq_lang_datalog_source as source;
pub use cq_lang_datalog_storage as storage;
pub use cq_lang_datalog_strata as strata;

/// Direct checked entries, each consuming its source IR and a continuation.
pub mod passes {
    pub use cq_datalog_macros::{
        fork, incremental, print_rounds, storage, stratify, surface_if_let,
    };
}
