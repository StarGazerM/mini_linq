//! Small external consumers of the public CQ parser and Rust lexical analysis.
//!
//! Run `cargo run --example cq-inspection` to inspect a triangle's result and
//! positive body atoms. Its language contract is checked separately from parsing.
//! Run `cargo run --example free-variables` to inspect atom computations, blocks,
//! macro initializers, negation, and aggregation syntax. This second example
//! intentionally inspects a wider parsed surface than the core accepts for
//! execution; negation and aggregation still require external expansion.
//!
//! Free-variable names are syntactic observations. `dependencies(environment)`
//! retains the whole logical environment for opaque expressions such as macros.
//! Rust resolves host captures, name resolution, and pattern refutability.
//! Tests use public cq-testkit oracles and executable bottom + Set refinement;
//! no check licenses moving Rust steps or replacing Leibniz equality.
//!
//! This workspace uses version plus path dependencies for local development.
//! The external gate replaces the core dependencies with packaged public crates.
//! From the repository root, use `cargo run --manifest-path
//! features/inspection/Cargo.toml --example cq-inspection --locked` or substitute
//! `free-variables` for the lexical analysis example.
