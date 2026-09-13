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

/// Dependencies and implementation choices for deferred operator lowering.
#[derive(Clone, Debug)]
pub struct RecipeOptions {
    pub storage: syn::Path,
    pub access: syn::Path,
    pub keys: Option<syn::Path>,
    pub work: Option<syn::Expr>,
}
