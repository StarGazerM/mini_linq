//! Query catalogs and deterministic data for MiniLinq correctness checks.
//!
//! This crate deliberately has no DuckDB dependency. The isolated root `xtask`
//! package owns heavyweight oracle execution. Query leaves use a direct
//! procedural macro that exposes a typed CQ without executing student passes in
//! the catalog build. Executable provider compositions are separate examples.

pub mod catalog;
pub mod dataset;
pub mod generate;
pub mod queries;
pub mod sql;

use cq_lang_cq as cq;

/// The stable version written into generated workload manifests.
pub const GENERATOR_VERSION: u32 = 2;

/// One of the two teaching corpora.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Suite {
    Classic,
    Retail,
}

impl Suite {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Retail => "retail",
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
    module: fn() -> syn::Result<cq::Program>,
}

impl QueryCase {
    pub(crate) const fn new(
        suite: Suite,
        name: &'static str,
        rust_path: &'static str,
        purpose: &'static str,
        scope_note: &'static str,
        module: fn() -> syn::Result<cq::Program>,
    ) -> Self {
        Self {
            suite,
            name,
            rust_path,
            purpose,
            scope_note,
            module,
        }
    }

    /// Construct the exact typed CQ owned by this Rust query module.
    pub fn module(self) -> syn::Result<cq::Program> {
        (self.module)()
    }
}

/// Return one case by its stable command-line name.
pub fn case(name: &str) -> Option<&'static QueryCase> {
    catalog::all().find(|case| case.name == name)
}

/// Stable catalog seed. Renaming a case deliberately changes its generated data.
pub fn seed_for(case: &QueryCase) -> u64 {
    // FNV-1a is fixed here rather than delegated to a randomized standard
    // hasher so checked-in datasets remain reproducible across Rust releases.
    case.name
        .as_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}
