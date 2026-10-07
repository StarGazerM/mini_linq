//! Executable conformance for the external surface passes.
//!
//! Authored integer answers and independent written-order references are compared
//! with generated Rust through cq-testkit's bottom + Set refinement harness.
//! Structural checks are tested in their owning crates. Effects and evaluation
//! counts in the migrated ownership/laziness regressions are instrumentation,
//! outside the query denotation. The DuckDB extended workload remains in workloads.
//! Core blocks preserve union and ordered fallback through this consumer.
//! Their laws are join distribution over union and `B1 else B2 = B1 union
//! (B2 anti_F B1)`. Common interface bindings escape; locals do not. The complete
//! IR retains union/anti, nested occurrence paths and branch Before ancestry.
//! Rust-free blocks hoist; Rust arms run from one prefix row with ordered else.
//!
