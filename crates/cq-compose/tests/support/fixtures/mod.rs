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
    pub iterator_pipeline: TokenStream,
    pub required_rust: &'static [&'static str],
    pub reused_normalized_inputs: &'static [usize],
    pub runtime_assertions: TokenStream,
}

pub struct InvalidSources {
    pub cq: TokenStream,
    pub index_requirements: TokenStream,
    pub rust_access_plan: TokenStream,
}

mod input_indexed;
mod invalid;
mod logical;
pub use input_indexed::index_requirements_to_staged_rust;
pub use invalid::{invalid_sources, missing_required_index};
pub use logical::cq_to_relational_plan_and_indexes;
