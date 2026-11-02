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
    /// A `RadixLayout` built over its resident source; facts in [`RadixRealization`].
    Radix,
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
    /// Present exactly for [`ProviderAbi::Radix`].
    pub radix: Option<RadixRealization>,
}

/// `RadixRunParams` of one radix instance, printed verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadixParams {
    pub ratio: usize,
    pub radix_bits: u32,
    pub merge_tile_rows: u32,
}

/// How one Sequence read of a radix instance is executed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RadixRead {
    /// Every key field bound: one group per run, items read through its cursor.
    Group,
    /// A leading key prefix: one encoded inclusive key interval per run.
    Prefix,
    /// A PH-Smallest side: the distinct next key field under the bound
    /// prefix, counted exactly over the union of all runs.
    Candidates { column: String },
}

/// Formal rule GPU-RadixProvider: the per-instance facts of a radix index.
/// `RadixLayout<K, V, W, O, C>` has `K` = `key_types` (one per `keys`
/// column, most significant first), `V` = `lanes` with `C` one `Buffer`
/// column per lane (one per `payload` column, in order), `W`/`O` = the
/// authored word/offset carriers and `params` from the instance's advice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadixRealization {
    pub key_types: Vec<syn::Type>,
    pub lanes: Vec<syn::Type>,
    pub word: syn::Type,
    pub offset: syn::Type,
    pub params: RadixParams,
    /// Every read of this instance as (sorted lookup keys, form).
    pub reads: Vec<(Vec<String>, RadixRead)>,
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
    /// Source access, publication steps, transfers, captures and auxiliary
    /// kernels selected by the target before printing.
    pub realization: super::realization::Realization,
}
