//! Checked Structures → CUDA tile streams. Storage choices are never inferred
//! from query names: keyed reads use their recorded SortedRun, PrefixRun, or
//! exact integral membership provider;
//! each canon requires an explicitly recorded `cq_gpu_runtime::DeviceSet<_>`.
//! The result is a typed, owning `cq_gpu_runtime::DeviceColumns<Row, Codec>`:
//! pass it to another compiled query without a transfer, or call `.to_host()`
//! explicitly. A bound `DeviceColumns` supplies its own immutable columns,
//! dictionary and codec facts; a bound Vec or slice retains the upload path.
//! Neither host codecs nor capacity advice establish canonicality.
//! Fixed full-tile scans decode each 1024-value vector into four register
//! values per lane once, then consume those values across the four stripes.
//! Short recorded batches and resumable scans use scalar device decode without
//! materializing a decompressed global column.
//! Host row tuples must exactly match declared relation types, not merely their
//! encoded widths. Integral predicate literals use the compared column's type.
//! Range reads are scan filters, never claimed as indexed reads. Output bounds
//! follow copied input/run columns and certified constants through resident
//! answer chaining; `probe first` never establishes primary-key uniqueness.
//! A fused stream supports one exact recorded batch size: 1..=1024 for scans,
//! 1..=256 when it contains expansion. Incompatible or larger recorded batches
//! fail with `CQ_GPU_STAGE_SIZE` rather than being silently clamped.
//!
//! Derived columns use the codec recorded by their Structures provider.
//! `DeviceSet<_>` and retained `Vec<_>` record Plain; an explicit
//! `retype(dedup => ::cq_gpu_runtime::DeviceSet<_, ::cq_gpu_runtime::For>)`
//! records packed answers. Retained rows can likewise record
//! `cq_gpu_runtime::DeviceColumns<_, Codec>`. Supported codec markers are
//! `Plain`, `For`, `Delta` and `DictionaryCodec`; unknown aliases are rejected.
//! `DeviceSet<Row, Codec, Policy>` records `Staged` (default), `Compacting`,
//! or `Fused`: respectively materialize then canonicalize, compact exact
//! prefixes at suspension, or compare complete emitted tuples before writing.
//! Fused output and its hash table grow together through saved continuations;
//! capacity advice is its initial reservation, not a uniqueness assumption.
//! Head canon still compares the ordered projected head, including repeats.
//! Unsupported operators and representations fail before CUDA is emitted.
//!
//! Scalar Bind supports explicitly typed copied dereferences and integral,
//! bool or `&str` literals. Arbitrary Rust, arithmetic and generators are
//! rejected rather than evaluated on the host. Copy and projection preserve
//! source bounds; literals have singleton bounds. Recorded head canon runs
//! after the ordered (possibly repeated) head projection.
//! Eager physical Anti uses its recorded run or exact integral membership
//! provider (composite keys require a supported prefix run). The Datalog bridge may certify
//! frozen projected key hosts with `datalog_keys(host:(Type,...),...)` for the
//! narrow `::cq_gpu_runtime::datalog::contains(&host, &(columns,))` total
//! predicate. This is a device membership intrinsic, not arbitrary captured
//! Rust `.contains`; missing schemas and mismatched key types are rejected.
//! Each execution rebuilds these exact indexes from its immutable key input.
//! `datalog_dictionary(shared)` seeds every rule execution from the bridge's
//! shared `Arc<Dictionary>` before binding inputs or encoding literals. This
//! also applies to input-free literal rules, so all rule holes share one ID
//! assignment rather than independently extending dictionary snapshots.
//! `datalog_pool(pool)` selects the bridge's shared `Arc<Pool>` even when a
//! rule has no bound input from which to inherit its device context.
//! `completion(round)` records that the caller owns the completion boundary:
//! holes enqueue work and retain statuses/events for a checked round readback.
//! The default `completion(query)` still completes once before returning.
//! Grow/resume suspension feedback remains necessary before dependent operators.

use proc_macro::TokenStream;

mod compiler;
mod options;
mod predicate;
mod prefix;

/// Terminal pipeline pass. Optional control arguments are `tile(256,4)`,
/// `frontier(8)` (zero disables publishing), `directory(rank)`, and
/// `capacity(InputRelation)` for an explicit input-cardinality allocation bound.
/// Capacity advice does not assert key uniqueness or change relational meaning;
/// exceeding it reports an error on bounded staged paths. Fused canon uses it
/// as an initial reservation and resumes on overflow. Without advice, outputs
/// grow by resuming saved device execution state.
/// Data/evaluation decisions come exclusively from checked Structures.
#[proc_macro]
pub fn print(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_structures::Program, syn::Expr>(input.into(), |source, arguments| {
        compiler::compile(source, syn::parse2(arguments)?)
    })
    .into()
}

/// Explicit Structures rewrite selecting a rank-backed prefix representation.
/// All recorded levels are sorted lexicographically and searched within their prefix.
/// `prefix_runs(Supplier => supplier_key)` explicitly selects a singleton level
/// by base relation or retained result. The selected column's layout index is
/// recorded in PrefixRun's fourth const argument and must match its first level.
#[proc_macro]
pub fn prefix_runs(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_structures::Program>(input.into(), prefix::compile).into()
}

#[cfg(test)]
mod tests;
