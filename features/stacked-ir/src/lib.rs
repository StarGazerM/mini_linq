//! # Stacked Trace IR
//!
//! This standalone Cargo workspace consumes the public core crates. Trace is an
//! external, nonterminal language with this complete program syntax:
//!
//! ```text
//! physical { ... }
//! trace {}
//! ```
//!
//! `cq_lang_trace::Program` implements `syn::parse::Parse`, `quote::ToTokens`,
//! and `cq_stage::Language`. Its public fields are `physical` and `trace`; its
//! local contract checks the complete Physical base. The empty annotation adds
//! no meaning. A Trace rewrite delegates correspondence to Physical's public
//! `check_rewrite`, preserving RA erasure and admissible recorded decisions.
//!
//! `physical_to_trace::{compile, check}` adds the annotation, and
//! `trace_to_physical::{compile, check}` erases it. Each independent pass checker
//! checks both contracts and requires an identical Physical AST, including every
//! declaration, operator, mode, key, canonical boundary and Rust payload. Checkers
//! never re-run compilation. The passes clone identifiers with their spans and
//! preserve Rust's chain, order and binders. Meaning remains the core's Kleisli
//! denotation over bottom + Set, written-order refinement, and Leibniz equality.
//!
//! ## Composition
//!
//! Use the composite `stacked_ir::round_trip` Physical rewrite in a named facade
//! pipeline:
//!
//! ```rust
//! use std::collections::BTreeSet;
//! let rows = [(1,), (1,), (2,)];
//! let answers: BTreeSet<_> = mini_linq::compile! {
//!     pipeline { physical = [stacked_ir::round_trip]; }
//!     struct Q;
//!     relation R(x: i32) = (rows);
//!     answer(x) :- R(x);
//! }.map(|(x,)| *x).collect();
//! assert_eq!(answers, BTreeSet::from([1, 2]));
//! ```
//!
//! The composite is the primary interface because it fits one Physical rewrite
//! entry and returns the language expected by the next named phase. It invokes
//! both checked library passes and `cq_stage::rewrite`, preserving the caller's
//! remaining queue. Representation selection and the printer stay in that queue;
//! the bridge chooses no continuation path. Repeating or combining this rewrite
//! with other public passes uses the same protocol.
//!
//! `stacked_ir::enter` and `stacked_ir::leave` expose the individual stage passes.
//! Start at Physical with an explicit queue when working at that boundary:
//!
//! ```rust
//! use std::collections::BTreeSet;
//! let rows = [(1,), (1,), (2,)];
//! let answers: BTreeSet<_> = stacked_ir::enter! {
//!     pipeline {
//!         stacked_ir::leave,
//!         mini_linq::passes::select,
//!         mini_linq::passes::print_pull
//!     };
//!     physical {
//!         relation R(x: i32) = (rows);
//!         r = pipe rename R@0 {x -> x};
//!         out = pipe canon project r keep {x};
//!         return out as answer(x);
//!     }
//! }.map(|(x,)| *x).collect();
//! assert_eq!(answers, BTreeSet::from([1, 2]));
//! ```
//!
//! An authored Trace program can start directly at `leave`, followed by selection
//! and a printer. Both stage macros use `cq_stage::pass`, so source/target
//! contracts and the nonterminal queue requirement are checked. All three macros
//! are also exported by the `trace-bridge` package and accept no arguments:
//!
//! ```compile_fail
//! let _ = stacked_ir::enter! {
//!     arguments { unexpected };
//!     pipeline { stacked_ir::leave, mini_linq::passes::select, mini_linq::passes::print_pull };
//!     physical { out = pipe unit; return out as answer(); }
//! };
//! ```
//!
//! ```compile_fail
//! let _ = stacked_ir::leave! {
//!     arguments { unexpected };
//!     pipeline { mini_linq::passes::select, mini_linq::passes::print_pull };
//!     physical { out = pipe unit; return out as answer(); }
//!     trace {}
//! };
//! ```
//!
//! ```compile_fail
//! let _ = stacked_ir::round_trip! {
//!     arguments { unexpected };
//!     pipeline { mini_linq::passes::select, mini_linq::passes::print_pull };
//!     physical { out = pipe unit; return out as answer(); }
//! };
//! ```
//!
//! ## Conformance
//!
//! The feature-owned `tests/reference.rs` and `tests/stacked.rs` reproduce every
//! current facade `physical_query` scenario: hosted reads, built indexes, a custom
//! index, merge, reductions, ordered ranges, computed columns, canonical generic
//! prefixes, generic multi-key reads and smallest intersections. The baseline
//! uses each authored scenario's original chain; the stacked target inserts
//! Trace into that same chain. `test-passes/` implements the required planners and
//! rewrites through public core APIs, recording their decisions in target IR.
//!
//! `physical_query_cases.txt` pins the ten query modules. `check_expansions.py`
//! checks both scenario/test inventories and source agreement apart from the
//! Trace insertion and formatting, executes both suites against the same authored
//! answers, and compares all ten expanded `run` functions as raw bytes. It reads
//! only this workspace and captures expansions and SHA256 sums under
//! `target/expansions/{reference,stacked}/`. It requires `cargo-expand`.
//!
//! The public `cq-testkit` supplies all 30 Physical corpus fixtures, independent
//! integer Set oracles, the pass mutation harness, and executable written-order
//! refinement. Tests check exact embedding/erasure, changed decisions and opaque
//! Rust payloads, malformed programs, queue forwarding, both explicit stage
//! entry points, repeated composites, and returning/empty/bottom inputs. The
//! corpus's selected/printed Rust ASTs and token texts are also unchanged.
//! Fixture evidence is finite; it does not establish arbitrary Rust effect or
//! evaluation-count equivalence.
//!
//! Run these commands from this directory; all six packages are default members:
//!
//! ```sh
//! cargo fmt --check
//! cargo clippy --all-targets -- -D warnings
//! cargo test
//! python3 check_expansions.py
//! ```
//!
//! Core dependencies declare both a version and a development path in
//! `workspace.dependencies`. The repository gate `python3 tools/check_external.py`
//! substitutes packaged public crates and builds/tests a copy outside the checkout.
//! Its package inventory must include `cq-testkit`, including the embedded corpus.

//! Core blocks preserve union and ordered fallback through this consumer.
//! Their laws are join distribution over union and `B1 else B2 = B1 union
//! (B2 anti_F B1)`. Common interface bindings escape; locals do not. The complete
//! IR retains union/anti, nested occurrence paths and branch Before ancestry.
//! Rust-free blocks hoist; Rust arms run from one prefix row with ordered else.
//!
pub use cq_lang_trace as trace;
pub use physical_to_trace;
pub use trace_bridge::{enter, leave, round_trip};
pub use trace_to_physical;
