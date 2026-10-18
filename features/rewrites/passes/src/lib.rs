//! External, checked transformations for Physical and Structures programs.
//!
//! These functions accept the same opaque argument tokens as their proc macros.
//! They preserve declarations, graph, Rust expressions, keys, and all
//! unrelated decisions. `build_host_reads` changes only host retention modes.
//! Correspondence belongs to the public language checks.
//! Smallest intersections may hoist independent relational definitions earlier;
//! opaque Rust definitions and their positions are unchanged.

//! Physical rewrites preserve union/anti operands, block paths and branch-scoped Before requirements.
//! Canonicality propagation treats union as a possible overlap and anti as retaining the left heading.
//!
//! Core blocks denote `bottom + Set`: `{ B1 } | { B2 }` unions projected
//! answers; `{ B1 } else { B2 }` selects the first nonempty arm. F is the free
//! heading and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Rust keeps a branch ancestry tree: union siblings unordered, fallback ordered;
//! each arm retains prefix Before facts and fallback anti steps. Rust `||` keeps
//! its existing recognition. Only differing literals on one interface column
//! prove union arms disjoint; no data keys or constraints are assumed.
//!
#![forbid(unsafe_code)]

mod physical;
mod selection;
mod structures;

pub use physical::{
    build_host_reads, canon_prefixes, lazy_semijoins, merge_joins, smallest_intersections,
};
pub use structures::{join_filter, retain_lookups, retype, sorted_runs};
