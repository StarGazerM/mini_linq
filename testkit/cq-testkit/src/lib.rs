#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

pub mod cases;
pub mod corpus;
pub mod cq_semantics;
pub mod databases;
#[cfg(feature = "duckdb")]
pub mod duckdb;
pub mod fuzz;
#[cfg(feature = "duckdb")]
pub mod golden;
pub mod harness;
pub mod semantics;
pub mod valuation_semantics;
pub mod values;

// Re-export the language crates so fixture accessor types can be named without
// callers adding dependencies merely to refer to them.
pub use cq_lang_cq as cq;
pub use cq_lang_ix as ix;
pub use cq_lang_physical as physical;
pub use cq_lang_ra as ra;
pub use cq_lang_structures as structures;
pub use cq_lang_vo as vo;

pub use cases::{CASES, Case};
pub use corpus::{SCENARIOS, Scenario};
pub use harness::PassCase;
pub use values::{Binding, Database, Relation, Rows};

/// Fallible I/O and DuckDB helpers preserve their underlying error.
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
