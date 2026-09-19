//! Authored Rust modules containing the workload query programs.
//!
//! Each leaf directly invokes the `mini_linq::workload_query!` procedural macro
//! once. Its token tree becomes a typed CQ factory for the catalog. It does not
//! select physical providers or emit an executable query API. Workload-wide
//! physical assignments and a host input/output contract remain to be supplied.

pub mod classic;
pub mod retail;
