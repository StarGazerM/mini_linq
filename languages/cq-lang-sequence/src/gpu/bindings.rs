//! Concrete provider realizations selected before GPU printing.
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectoryPolicy {
    DenseU32 { terminal_entries: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderAbi {
    Range,
    Runs,
    Host,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderLayout {
    /// Actual provider type whose catalog ABI was checked during binding.
    pub ty: syn::Type,
    pub abi: ProviderAbi,
    pub keys: Vec<String>,
    pub payload: Vec<String>,
    pub directory: Option<DirectoryPolicy>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeLookup {
    pub host: String,
    pub source: Vec<String>,
    pub keys: Vec<String>,
    pub payload: Vec<String>,
    /// None borrows the existing view; Some materializes this complete permutation.
    pub permutation: Option<Vec<usize>>,
    pub materialized_provider: Option<syn::Type>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColumnBounds {
    /// Union of immutable, original resident input column domains.
    Resident { sources: Vec<(String, usize)> },
    /// Declared scalar carrier domain; Rustc supplies its numeric bounds.
    Scalar { ty: syn::Type },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UploadAdapter {
    EmptyRows,
    NumericColumns { types: Vec<syn::Type> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bindings {
    pub providers: BTreeMap<String, Vec<ProviderLayout>>,
    pub native: BTreeMap<String, NativeLookup>,
    /// Downloaded host scans use these recorded replacement providers.
    pub downloads: BTreeMap<String, syn::Type>,
    pub bounds: BTreeMap<String, ColumnBounds>,
    pub uploads: BTreeMap<String, UploadAdapter>,
}
