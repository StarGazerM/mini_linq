//! Query catalogs and deterministic data for MiniLinq correctness checks.
//!
//! This crate deliberately has no DuckDB dependency. The isolated root `xtask`
//! package owns heavyweight oracle execution. Query leaves use a direct
//! procedural macro that exposes a typed CQ without executing student passes in
//! the catalog build. Executable provider compositions are separate examples.

pub mod adversarial;
pub mod pipelines;
mod sorted_index;
pub use sorted_index::SortedIndex;
pub mod catalog;
pub mod costs;
pub mod dataset;
pub mod execution;
pub mod extended;
pub mod extended_costs;
pub mod generate;
mod loading;
pub mod measurement;
pub mod queries;
pub mod sql;
pub mod tpch;
pub mod typed;

use cq_lang_cq as cq;

/// The stable version written into generated workload manifests.
pub const GENERATOR_VERSION: u32 = 2;

/// One of the workload categories.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suite {
    Classic,
    Retail,
    Extended,
}

impl Suite {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Retail => "retail",
            Self::Extended => "extended",
        }
    }
}

/// Reproducible input sizes. `Large` is an opt-in benchmark scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scale {
    Tiny,
    Medium,
    Large,
}

impl Scale {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tiny => "tiny",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }
}

/// The semantic shape exercised by one generated input dataset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scenario {
    /// A nonempty result plus rows that exercise physical-access edge cases.
    Coverage,
    /// One required input relation is empty.
    EmptyInput,
    /// Every input is nonempty, but the query result is empty.
    NoMatch,
}

impl Scenario {
    pub const ALL: [Self; 3] = [Self::Coverage, Self::EmptyInput, Self::NoMatch];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Coverage => "coverage",
            Self::EmptyInput => "empty-input",
            Self::NoMatch => "no-match",
        }
    }
}

/// Everything needed to reproduce one generated dataset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GenerationConfig {
    pub scenario: Scenario,
    pub scale: Scale,
    pub seed: u64,
}

/// One complete positive-CQ workload and its teaching metadata.
#[derive(Clone, Copy, Debug)]
pub struct QueryCase {
    pub suite: Suite,
    pub name: &'static str,
    /// Rust source module containing the authoritative query token tree.
    pub rust_path: &'static str,
    pub purpose: &'static str,
    /// Honest scope note for derived rather than standards-compliant cases.
    pub scope_note: &'static str,
    /// Independently authored DuckDB query over declaration-ordered raw tables.
    pub sql: &'static str,
    module: fn() -> syn::Result<cq::Program>,
    load: loading::ReadRelations,
}

impl QueryCase {
    /// Construct the exact typed CQ owned by this Rust query module.
    pub fn module(self) -> syn::Result<cq::Program> {
        (self.module)()
    }

    /// Read corpus tuples through loaders generated from the authored callback.
    pub fn load(
        self,
        directory: impl AsRef<std::path::Path>,
    ) -> Result<dataset::Dataset, dataset::DatasetError> {
        let directory = directory.as_ref();
        let schema = self.module().map_err(dataset::DatasetError::InvalidQuery)?;
        dataset::Dataset::from_loaded_schema(
            &schema.inputs().cloned().collect::<Vec<_>>(),
            directory,
            (self.load)(directory)?,
        )
    }
}

/// Return one case by its stable command-line name.
pub fn case(name: &str) -> Option<&'static QueryCase> {
    catalog::all().find(|case| case.name == name)
}

/// Stable catalog seed. Renaming a case deliberately changes its generated data.
pub fn seed_for(case: &QueryCase) -> u64 {
    seed_for_name(case.name)
}

pub fn seed_for_name(name: &str) -> u64 {
    // FNV-1a is fixed here rather than delegated to a randomized standard
    // hasher so checked-in datasets remain reproducible across Rust releases.
    name.as_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}
