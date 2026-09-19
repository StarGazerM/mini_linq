//! Explicit physical selection: preserve IX and record supplied choices.
//!
//! This policy has no defaults, provider-name cases, or runtime construction.
//! It checks assignment structure. Semantic admission still requires the
//! selected external definitions; success here does not certify their laws.

use cq_ir::{index_requirements, physical};

/// Record explicit providers and lowering rules for every required position.
///
/// Selection preserves the complete IX payload, including schema, RA,
/// output metadata, index identities, and use references. Rust paths are opaque
/// names. Missing or extra choices reject rather than invoking a fallback.
pub fn given_bindings(
    index_requirements: index_requirements::Module,
    assignments: physical::Assignments,
) -> syn::Result<physical::Module> {
    let target = physical::Module {
        index_requirements,
        assignments,
    };
    physical::contract::check_structure(&target)?;
    Ok(target)
}
