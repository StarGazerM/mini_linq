//! External, checked transformations for Physical and Structures programs.
//!
//! These functions accept the same opaque argument tokens as their proc macros.
//! They preserve declarations, graph, Rust expressions, keys, modes, and all
//! unrelated decisions. Correspondence belongs to the public language checks.
//! Smallest intersections may hoist independent relational definitions earlier;
//! opaque Rust definitions and their positions are unchanged.

#![forbid(unsafe_code)]

mod physical;
mod selection;
mod structures;

pub use physical::{canon_prefixes, lazy_semijoins, merge_joins, smallest_intersections};
pub use structures::retype;
