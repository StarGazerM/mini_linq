//! # cq-planners
//!
//! An external CQ→RA′ planner on the public `cq-stage` protocol. This standalone
//! workspace contains a planner library, its protocol macro, the public
//! `cq-planners` facade, and a test-only Rust refinement driver. Core dependencies
//! use a version and a local path; the external gate replaces those paths with
//! packaged dependencies.
//!
//! ```rust
//! let r = [(1, 2), (1, 2), (2, 3)];
//! let s = [(2, 4), (3, 5)];
//! let answer: std::collections::BTreeSet<_> = mini_linq::compile! {
//!     pipeline { plan = cq_planners::order(prefix greedy, fine); }
//!     struct Q;
//!     relation r(x: i32, y: i32);
//!     relation s(y: i32, z: i32);
//!     answer(x, z) :- r(x, y), s(y, z);
//! }
//! .map(|(x, z)| (*x, *z))
//! .collect();
//! assert_eq!(answer, [(1, 4), (2, 5)].into_iter().collect());
//! ```
//!
//! The pass parses exactly these arguments:
//!
//! ```text
//! written, coarse
//! written, fine
//! hypergraph
//! prefix connected, coarse
//! prefix connected, fine
//! prefix greedy, coarse
//! prefix greedy, fine
//! elimination, coarse
//! elimination, fine
//! ```
//!
//! Empty arguments select `written, coarse`. Unknown names, missing granularity,
//! and trailing tokens are errors. The same macro can start an explicit queue:
//!
//! ```rust
//! let rows = [(1,), (2,), (2,)];
//! let answer: std::collections::BTreeSet<_> = cq_planners::order! {
//!     arguments { elimination, fine };
//!     pipeline {
//!         mini_linq::passes::index_demands,
//!         mini_linq::passes::lower,
//!         mini_linq::passes::select,
//!         mini_linq::passes::print_pull
//!     };
//!     struct Q; relation rows(x: i32); answer(x) :- rows(x);
//! }
//! .map(|(x,)| *x)
//! .collect();
//! assert_eq!(answer, [1, 2].into_iter().collect());
//! ```
//!
//! Every invocation normalizes CQ, calls `cq-order`, calls `cq-reduce`, then
//! elaborates the recorded valuation with `cq-elaborate`. Public correspondence
//! checks validate each boundary. The emitted RA′ contains the chosen operator
//! graph and opaque Rust chain. Later passes consume that written graph.
//!
//! | Planner | Recorded valuation |
//! | --- | --- |
//! | Written | Written clause order; coarse uses full atoms, fine splits each atom in place |
//! | Hypergraph | Written/coarse plus the full upward/downward Yannakakis reducer when acyclic |
//! | Prefix connected | Ready connected atoms before disconnected atoms, then written position |
//! | Prefix greedy | Connected atoms, then smallest resulting heading width, then written position |
//! | Elimination | Coarse ranks atoms by elimination variables; fine enumerates the reversed elimination order so connecting variables precede leaf products |
//!
//! All planners use elaboration's relational selection pushdown. Hypergraph
//! places tree reductions before the first atom; cyclic queries carry no tree
//! reductions. Rust guards are never pushed onto base relations. These are
//! deterministic heuristics, without cardinality estimates or complexity claims.
//!
//! For library consumers, `Planner::ALL` lists the nine choices.
//! `compile_valuation(&cq, planner)` returns complete VO text and
//! `compile(&cq, planner)` returns RA′. `check_valuation(&cq, &vo, planner)` checks
//! coverage, granularity, written-order obligations and complete tree reductions.
//! It accepts admissible free rankings without rerunning the selector.
//! `check(&cq, &ra)` checks refinement independently of planner selection, using
//! the core's tableau and Rust-chain proof against a directly constructed exact
//! written normal-form schedule.
//!
//! The meaning is the Kleisli composition for `bottom + Set`: when the written
//! reference returns, a target must return the same set without panic. A panicking
//! reference admits either target outcome. Rust steps stay opaque and retain their
//! relative order, free-variable binders and written relational predecessors.
//! Equality assumes Leibniz identity. Effects and evaluation counts lie outside
//! this denotation.
//!
//! Tests use only the public `cq-testkit` package. They cover all 30 published
//! scenarios through all nine planners, authored answers and independent CQ/VO/RA′
//! oracles, 54 pinned VO/RA′ texts, rejected mutations and admitted free rankings,
//! named and explicit macro pipelines, and 200 deterministic partial Rust seeds
//! (`0, 2, ..., 398`) executed under every planner against exact written-order
//! Rust. The seed corpus covers division, remainder, overflow and unwrap in guards,
//! bindings, atom arguments and heads, including empty and duplicate inputs.
//! The elimination regression separately asserts that the leaf-first intermediate
//! product of 576 rows becomes 24, with equal answer sets, and keeps the compiled
//! 24-visit cost probe.
//!
//! `triangle`, `chain` (the public `oriented_chain` scenario), and `fine_triangle`
//! are pinned for every planner in `tests/fixtures/`. The two triangle CQ inputs
//! are identical; each planner's split determines its VO. Fixtures contain complete
//! canonical token text and are compared byte for byte. An intentional fixture
//! change can be captured with `cargo run --example pin_plans` and reviewed;
//! ordinary tests never regenerate fixtures.
//!
//! Acceptance from this directory:
//!
//! ```text
//! cargo fmt --check
//! cargo clippy --all-targets -- -D warnings
//! cargo test
//! ```
//!
//! From the repository root, run `python3 tools/check_external.py`. The gate must
//! package `cq-testkit` along with the core, including its embedded corpus. The
//! feature uses only public testkit APIs.

//! External CQ→RA′ planning passes.
//!
//! [`order!`] consumes CQ with the cq-stage queue protocol and emits complete
//! RA′ text. [`compile_valuation`] exposes the chosen VO text for inspection.
//! Every choice is recorded there; subsequent passes receive only written IR.
#![forbid(unsafe_code)]

pub use cq_planners_macros::order;
pub use cq_planners_plan::*;
