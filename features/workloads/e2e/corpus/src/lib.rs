//! Mandatory public-pipeline corpus matrix: 20 queries and a checked advice/host matrix.
//!
//! Triangle, oriented-chain, claw and retail_h03 have valid products of 4,608;
//! 16 other queries use a cover verified against every valid pair of ten axes.
//! Every configuration either executes all applicable CSV/DuckDB scenarios or
//! receives an individually attributed compiler refusal. Unsupported printer
//! shapes and ordered host capabilities remain explicit negative tests.
//! The census forbids duplicate/unclassified rows and matches both generated
//! execution/rejection totals; compile-fail tests pin diagnostic obligations.
//! Accepted configurations are dealt round-robin into 128 `shard_NNN` test
//! crates of about 110 queries each, so each rustc front end stays small
//! (about 1 GB) and Cargo compiles shards in parallel; run them together with
//! `cargo test --test 'shard_*'`.
