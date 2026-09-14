//! Compile-time configuration, separate from semantic pass inputs.
//!
//! Callers select dependencies explicitly. Instrumentation is absent unless a
//! counter expression is supplied; an uninstrumented expansion emits no counter
//! binding, inspection, or update.

/// Dependencies and optional instrumentation for a storage operation.
#[derive(Clone, Debug)]
pub struct ProviderOptions {
    pub storage: syn::Path,
    pub work: Option<syn::Expr>,
}

/// Dependencies and implementation choices for native access leaves.
#[derive(Clone, Debug)]
pub struct AccessOptions {
    pub storage: syn::Path,
    pub iterator: syn::Path,
    pub keys: Option<syn::Path>,
    pub work: Option<syn::Expr>,
}
