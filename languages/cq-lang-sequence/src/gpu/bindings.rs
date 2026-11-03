//! Concrete provider realizations selected before GPU printing.
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DirectoryPolicy {
    DenseU32 { terminal_entries: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderAbi {
    Range,
    RadixRelation,
    /// A `RadixLayout` built over its resident source; facts in [`RadixRealization`].
    Radix,
    Host,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderLayout {
    /// The built provider instance (declared token, printed spelling) whose
    /// catalog ABI was checked during binding.
    pub provider: cq_lang_structures::Provider,
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
    /// Block width of the radix engine's kernels.
    pub block_threads: u32,
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

/// A complete permutation of a full-key relation materialized as a declared
/// provider before the lookup reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Materialization {
    /// Source key position of each physical column in the new key order.
    pub permutation: Vec<usize>,
    /// The declared materialized provider instance.
    pub provider: cq_lang_structures::Provider,
    pub params: RadixParams,
    /// Complete codec width or conservative full word width, printed verbatim.
    pub active_bits: syn::Expr,
    /// Block width of the permutation kernel.
    pub permutation_threads: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeLookup {
    pub host: String,
    pub source: Vec<String>,
    pub keys: Vec<String>,
    pub payload: Vec<String>,
    /// None borrows the existing view.
    pub materialization: Option<Materialization>,
    /// Full physical key schema and retained snapshot read contract.
    pub radix: RadixRelationRead,
}


/// Native full-key set read, independent of the occurrence radix ABI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadixRelationRead {
    pub provider: cq_lang_structures::Provider,
    pub key_types: Vec<syn::Type>,
    pub word: syn::Type,
    pub word_capture: RadixWordCapture,
    pub offset: syn::Type,
    pub params: RadixParams,
    /// The lookup decodes columns at these physical key positions.
    pub fields: Vec<usize>,
}

/// Kernel word types are inferred from a retained host view, not spelled in
/// the CUDA module: the producer's word expression can name function-local types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RadixWordCapture {
    /// A `Word` kernel generic, prepared with a zero-value type witness inferred
    /// from this input's `RadixLayoutView<W, u64>`. Permutations share its `W`.
    InputView { host: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColumnBounds {
    /// Union of immutable, original resident input column domains.
    Resident { sources: Vec<(String, usize)> },
    /// Declared scalar carrier domain; Rustc supplies its numeric bounds.
    Scalar { ty: syn::Type },
}

/// How an uploaded column's bounds are established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadBounds {
    /// One device reduction and one readback over the uploaded column.
    Measure,
    /// The column carrier's whole range; no device work.
    Scalar,
}

/// Device encoding of an uploaded column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UploadCodec {
    /// Frame of reference over the column's bounds.
    For,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadColumn {
    pub codec: UploadCodec,
    pub bounds: UploadBounds,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UploadAdapter {
    EmptyRows,
    /// One carrier and one encoding record per layout column.
    NumericColumns {
        types: Vec<syn::Type>,
        columns: Vec<UploadColumn>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bindings {
    pub providers: BTreeMap<String, Vec<ProviderLayout>>,
    pub native: BTreeMap<String, NativeLookup>,
    /// Retained typed full-key layouts supplied by host inputs.
    pub relation_inputs: BTreeMap<String, RadixRelationRead>,
    /// Downloaded host scans read through these recorded declared providers.
    pub downloads: BTreeMap<String, cq_lang_structures::Provider>,
    /// Host row subplans use these collection/index adapters instead of the
    /// device-only provider at the recorded Structures entry position.
    pub host_materializations: BTreeMap<usize, cq_lang_structures::Provider>,
    /// Device carrier of every Physical and capture answer column: its
    /// logical type, or the declared codec's u32 code.
    pub carriers: BTreeMap<String, syn::Type>,
    pub bounds: BTreeMap<String, ColumnBounds>,
    pub uploads: BTreeMap<String, UploadAdapter>,
    /// Source access, publication steps, transfers, captures and auxiliary
    /// kernels selected by the target before printing.
    pub realization: super::realization::Realization,
}
