use proc_macro2::TokenStream;
use quote::quote;

pub struct LoweringFixture {
    pub source: TokenStream,
    pub expected: TokenStream,
}

pub struct PhysicalFixture {
    pub source: TokenStream,
    pub program_name: &'static str,
    pub rust_access_plan: TokenStream,
    pub pipeline_to_distinct_iter: TokenStream,
    pub required_rust: &'static [&'static str],
    pub reused_normalized_inputs: &'static [usize],
    pub runtime_assertions: TokenStream,
}

pub struct InvalidSources {
    pub cq: TokenStream,
    pub index_requirements: TokenStream,
    pub rust_access_plan: TokenStream,
}

mod base_hash;
mod invalid;
mod logical;
pub use base_hash::index_requirements_to_staged_rust;
pub use invalid::{invalid_sources, missing_required_index};
pub use logical::cq_to_relational_plan_and_indexes;
