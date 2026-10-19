//! Insert-only Datalog implemented above the ordinary MiniLinq compiler.
//!
//! The language records checked strata, semi-naive delta variants and owned
//! index layouts. The bridge executes ordinary CQ holes against borrowed hosts
//! and admits each candidate through one fast-hashed canonical insertion.
//! It does not change core planning or inject pipeline arguments.
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
//! An optional `pipeline { ... }` before `struct` selects the ordinary CQ
//! pipeline; its arguments are forwarded without additions. [`stratified!`]
//! resumes text containing checked `strata` decisions, while [`seminaive!`]
//! resumes text also containing explicit delta variants and index layouts.
//! Full and delta layout decisions are distinct: for example,
//! `delta_layout Reach [1,0] key 1;` names the base IDB `Reach`, orders its
//! columns, and selects a one-column key for its current delta. These entries
//! follow all `layout` entries and precede variants in a `seminaive` block.
//! Full layouts receive per-admitted-row insertions. Each demanded delta
//! layout is batch-built once at seed publication and once at every subsequent
//! publication, replacing the previous run even when the new delta is empty.
//! The next round's holes share that immutable grouped host rather than
//! building private indexes. Keyed delta reads and later zero-key probes use
//! these layouts; only a first-position unkeyed delta remains a `Vec` scan.
//! Neither layout selection nor publication reorders written atoms or guards.
//!
//! # Opt-in parallel candidate evaluation
//!
//! Write `parallel;` before `struct` in [`datalog!`], [`stratified!`], or
//! [`seminaive!`] and add a direct `rayon = "1"` dependency to the consuming
//! crate. Sequential programs need no Rayon dependency or parallel trait bounds.
//! Parallel execution uses the current Rayon pool, so callers can select a local
//! pool with `rayon::ThreadPoolBuilder` and `pool.install(|| { ... })`.
//!
//! Only a positive atom at body position zero with a zero-column key can be
//! partitioned. Its ordered source is either the current delta vector or the
//! zero-key bucket of a full layout. Contiguous source chunks evaluate the same
//! rewritten rule hole as sequential execution; only that first occurrence is
//! replaced, including when the same relation occurs again in a self-join.
//! Later atoms, keyed probes, guards, and generators keep their written order.
//! The sequential one-chunk binding retains the original whole host, preserving
//! its existing `Rows` realization without allocating or rehashing a wrapper.
//! Empty, tiny, and ineligible sources use sequential evaluation.
//!
//! Workers produce candidate rows, not admissions. Chunk results are consumed
//! in source order, and canonical insertion, layout updates, delta publication,
//! and observer calls remain on the evaluating thread (the thread running the
//! macro, which may itself be a Rayon pool thread). Output order is unspecified:
//! ordinary EDB canonicalization uses randomized hashing. Given identical
//! published source order (as in deterministic `counters` builds), parallel
//! execution preserves the exact sequential output iterator sequence. An
//! observer can retain ordinary mutable, non-`Send` state.
//! Parallel mode requires shared inputs and Rust values captured by worker
//! expressions to be `Sync`, and candidate rows to be `Send`; sequential mode
//! adds neither requirement. Rust expressions must still be pure: their worker
//! execution order is not an ordering guarantee for side effects. Worker panics
//! propagate to the caller instead of yielding a partial result.
//!
//! The opt-in `phase-timing` Cargo feature is for profiling, not normal builds.
//! It prints per-round `evaluation_ns`, `admission_ns`, and `publication_ns` to
//! stderr. To separate those phases it materializes candidates in sequential
//! mode too; use an uninstrumented build for sequential-regression/scaling
//! measurements. Without the feature, sequential holes stream into admission
//! and generated evaluation code contains no timing calls.
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
//! bottom, not a failed match. Unscheduled source round trips retain `if let`;
//! stratification and parsing scheduled text lower it to the ordinary `for`,
//! so stratified/semi-naive printing exposes the desugaring.
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

pub use cq_datalog_fork as fork;
pub use cq_datalog_schedule as schedule;
pub use cq_lang_datalog as language;
pub use datalog_bridge::{datalog, seminaive, stratified};
