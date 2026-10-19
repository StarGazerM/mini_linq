//! Authored MiniLinq/SQL workloads and deterministic conformance data.
//!
//! This standalone workspace consumes public core APIs and sibling planner,
//! rewrite, dialect and surface-pass packages. The primary corpus has 20 queries;
//! the extended corpus adds 29 authored pairs, including 17 extension queries.
//! Five typed pairs run on both CSV and Parquet. The complete golden inventory
//! is 151 files; xtask uses cq-testkit for independent SQL evaluation, strict
//! byte comparisons and seeded abstract fuzz queries.
//!
//! Corpus tests compose named public pipelines: all nine planner passes,
//! Physical merge/canonical-prefix/smallest/lazy rewrites, recorded Structures
//! types, supported pull/push printers, and slice/HashMap/BTreeMap hosts.
//! One joining triangle kernel uses all 2,400 supported configurations; every
//! catalog case also runs visible pairwise rows. Positional advice is slice-only.
//! UI fixtures pin unsupported printer/host choices and malformed retained layouts.
//!
//! Queries keep Kleisli bottom + Set semantics, written-order refinement,
//! opaque pinned Rust steps and Leibniz equality. Successful checks at one
//! boundary never replace compilation and oracle agreement of emitted Rust.
//!
//! Synthetic and TPC-H-derived timings use release executables compiled without
//! storage instrumentation. Counters come from separate instrumented executions.
//! TPC workers require exact DuckDB agreement for every warm/timed run and both
//! independent counted runs; no time threshold is a correctness gate. The TPC
//! inventory retains its 12 kernels and 48 timing rows across both scales.
//! retail_h07_block adds two pipelines at each scale, for 52 timing rows total.
//! Q6/Q19 retain full-query identities; other kernels remain explicitly derived.
//! Fresh acceptance evidence and final tables belong in the task ledger/stdout.

//!
//! Run acceptance from this standalone workspace. Corpus and benchmark contracts
//! are documented here; commands that measure performance print fresh evidence.
//!
//! ```sh
//! cargo fmt --check
//! cargo clippy --all-targets -- -D warnings
//! cargo test
//! cargo test -p cq-workloads --features counters --test growth -- --nocapture
//! cargo test -p cq-workload-e2e --features parquet
//! cargo run -p cq-xtask --features oracle -- golden
//! cargo run -p cq-xtask --features oracle -- fuzz --cases 200 --seed 0
//! cargo run -p cq-xtask --features oracle -- fuzz --cases 2000 --seed 0
//! cargo run -p cq-xtask --features oracle -- bench
//! cargo run -p cq-xtask --features oracle -- bench --tpch --sf 0.1
//! cargo run -p cq-xtask --features oracle -- bench --tpch --sf 1
//! cargo run -p cq-xtask --features oracle -- bench --tpch --sf 1 --runs 5 --config le
//! cargo run -p cq-xtask --features oracle -- bench --tpch --sf 1 --runs 5 --config le_runs
//! ```
//!
//! From the repository root: `python3 tools/check_external.py`.
//!
//! The repository root provides `cargo xtask` for the same tool.

//! TPC coverage adds retail_h07_block with H07's unchanged SQL, default/hypergraph candidates and country-block semijoins before Lineitem; the existing catalog remains unchanged.
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
#![recursion_limit = "512"]

pub mod adversarial;
pub mod pipelines;
pub mod schema;
mod sorted_index;
pub use sorted_index::SortedIndex;
pub mod catalog;
pub mod costs;
pub mod dataset;
pub mod execution;
pub mod extended;
pub mod extended_costs;
pub mod generate;
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
    pub inputs: &'static [schema::Relation],
    pub arity: usize,
}

impl QueryCase {
    /// Read the authoritative literal query for generators and inspection tools.
    /// Executable cases use `inputs` metadata instead of parsing their source.
    pub fn module(self) -> syn::Result<cq::Program> {
        cq_workload_authored::program(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(self.rust_path),
        )
    }

    /// Read and validate integer fixtures using the catalog's checked schema.
    pub fn load(
        self,
        directory: impl AsRef<std::path::Path>,
    ) -> Result<dataset::Dataset, dataset::DatasetError> {
        let directory = directory.as_ref();
        dataset::Dataset::read_schema(&schema::declarations(self.inputs), directory)
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
