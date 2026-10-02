//! # own-dialect
//!
//! A standalone external consumer with a terminal Structures → Rust push pass:
//!
//! ```rust
//! let R = [(1_i32,), (1,), (2,)];
//! let answers: Vec<_> = mini_linq::compile! {
//!     pipeline { print = own_push::push; }
//!     struct Q;
//!     relation R(x: i32);
//!     answer(x) :- R(x);
//! }.collect();
//! assert_eq!(answers.len(), 2);
//! ```
//!
//! The printer receives a complete checked Structures program through
//! `cq_stage::pass`. It preserves the Physical graph and uses its recorded
//! build, host read, retained row, canonical boundary and computed-head types.
//! Authored Structures can also start directly at
//! `own_push::push! { pipeline {}; physical { ... } structures { ... } }`.
//! The pass accepts no arguments and ends the queue.
//!
//! Nested loops push answers into a Vec endpoint. At retained constructions a
//! temporary traversal buffer feeds the recorded collection's `FromIterator`
//! and the recorded indexes' `Build` implementations. These buffers do not
//! introduce canonical boundaries or replace a recorded representation. The
//! endpoint is eager; the returned iterator traverses the completed answer.
//!
//! The denotation is Kleisli composition over bottom + Set. A panic aborts the
//! whole evaluation. Rust steps remain opaque, pinned in their chain, and observe
//! shared references; relational steps may move only earlier across them. Hosts
//! are immutable, columns stay borrowed, and equality requires Leibniz identity.
//! Effects and evaluation counts are outside this denotation. Owned computed
//! columns can flow through the tail; retained scans and payload fan-out impose
//! the emitted Rust's Copy requirements. `exists` consumes the full input before
//! retaining zero or one unit row.
//!
//! The explicit allow-lists in `support.rs` cover all 25 public testkit
//! cases and all 30 published Structures plans. Each is compiled with both pull
//! and push on slice, HashMap and BTreeMap hosts, then executed on raw authored
//! inputs (including duplicates) and eight seeded binary-domain databases.
//! Outputs must contain no duplicates and equal authored answers or independent
//! CQ oracle answers. A changed corpus requires reviewing the allow-lists.
//!
//! Unsupported written shapes are also listed with reasons in support.rs:
//!
//! - `merge`: ordered push co-iteration is not implemented.
//! - `probe smallest`: adjacent candidate-intersection traversal is not implemented.
//!
//! The compile-fail tests check valid source contracts and compile each shape
//! successfully with the pull printer before requiring the push refusal.
//! They also reject printer arguments and remaining passes. Execution tests
//! cover custom recorded types, borrowed columns without Clone or Hash,
//! computed answer images, and written-order partial-function refinement.
//!
//! From this directory, run `cargo fmt --check`,
//! `cargo clippy --all-targets -- -D warnings`, and `cargo test`.
//! From the repository root, `python3 tools/check_external.py` validates the
//! copied feature against Cargo-packaged public core crates.

//! An external consumer that selects `own_push::push` as its Structures printer.
//!
//! The denotation is `T X = bottom + Set X`. Rust chain nodes are opaque partial
//! functions, retaining order and their source-before relational constraints.
//! I/O, mutation and evaluation counts are outside this meaning.

pub use own_push::push;
