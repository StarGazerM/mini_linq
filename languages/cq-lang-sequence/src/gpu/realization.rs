//! GPU realization records selected by the target refinement before printing.
//! Each records one decision a printer would otherwise make: source access,
//! publication steps, terminal grouping, host transfers, auxiliary kernel
//! geometry, provider descriptor bounds and closed captures.
use std::collections::{BTreeMap, BTreeSet};

use cq_lang_structures::syntax::algebra::Theta;

use super::Grid;

/// Where a device source's rows come from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceOrigin {
    /// Immutable resident input columns (`host`), renamed into `columns`.
    Resident { host: String },
    /// Resident packed device runs; lookups use the recorded native providers.
    Runs {
        host: String,
        providers: Vec<String>,
    },
    /// Host-evaluated rows uploaded through the recorded adapter during build.
    Uploaded { evaluation: HostEvaluation },
}

/// A contiguous half-open interval on one resident column: the intersection
/// of its bounds, each stated by one constraint of a relational selection
/// (or constant rename) on the source's chain. Printing folds the bounds in
/// order: `start = max(i64::MIN, lower...)`, `end = min(i64::MAX, upper...)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interval {
    /// Origin (resident input) column.
    pub column: String,
    pub bounds: Vec<IntervalBound>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Lower,
    Upper,
}

/// One bound of an [`Interval`]; an equality states an inclusive lower and an
/// inclusive upper bound on the same value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntervalBound {
    /// The Physical selection or constant rename stating this bound.
    pub definition: String,
    pub side: Side,
    pub inclusive: bool,
    /// The encoded host value compared against the column.
    pub value: syn::Expr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceAccess {
    /// Read every source row.
    Scan,
    /// Materialize the interval's row positions once, then read through them.
    Positions(Interval),
    /// Build the recorded Range provider over the source, restricted to the
    /// optional interval; the provider's payload is the source read.
    Provider { selection: Option<Interval> },
}

/// Construction placement: before the timed build (immutable resident
/// inputs) or inside it (host evaluation and upload).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Setup,
    Build,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceRealization {
    pub origin: SourceOrigin,
    /// Physical column to origin column.
    pub columns: BTreeMap<String, String>,
    pub access: SourceAccess,
    pub phase: Phase,
}

/// Device publications and resident inputs crossing to the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransferSource {
    /// A completed Sequence publication, downloaded once per invocation.
    Publication(String),
    /// A resident input read by host Rust through its recorded download adapter.
    Resident(String),
}

/// Transfer granularity: complete logical rows of the source's layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Granularity {
    CompleteRows,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transfer {
    pub source: TransferSource,
    pub granularity: Granularity,
}

/// Host evaluation of a Physical result over completed device publications
/// (bound by result) and downloaded resident inputs, before its upload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostEvaluation {
    /// Completed publications whose results bind host dependencies.
    pub bound: Vec<String>,
    /// Transfers first performed by this evaluation, in order.
    pub transfers: Vec<Transfer>,
}

/// One bounds domain used by a provider descriptor column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundSource {
    /// Recorded resident input column metadata (`host`, layout index).
    Resident { host: String, index: usize },
    /// Bounds metadata of an uploaded host publication (layout index).
    Upload { publication: String, index: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BaseBounds {
    /// Union of the listed domains.
    Union(Vec<BoundSource>),
    /// Declared Scalar carrier range.
    Scalar(syn::Type),
    None,
}

/// Bounds printed into a provider descriptor column: the base domain, then
/// each upload merged pairwise in order. A measured key replaces the bounds
/// by one device reduction over the published column (one readback).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DescriptorBounds {
    pub base: BaseBounds,
    pub merges: Vec<BoundSource>,
    pub measure: bool,
}

/// Range providers built over one completed device publication.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderBuild {
    /// Indices of device layouts built by this publication; host layouts have
    /// their own host construction and are never rebuilt from device rows.
    pub layouts: Vec<usize>,
    /// Every key and payload column of the publication's provider layouts.
    pub columns: BTreeMap<String, DescriptorBounds>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnionArm {
    /// Copy a flat publication into the staging carrier.
    Copy { publication: String },
    /// Decode a packed publication's recorded fields into the staging carrier.
    Decode { publication: String, grid: Grid },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostUnion {
    pub arms: Vec<UnionArm>,
    /// Pack the staging carrier into the publication's recorded key codec.
    pub pack: Option<Grid>,
    pub provider: Option<ProviderBuild>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupInput {
    /// Reuse the packed canonical publication; argument extraction decodes
    /// its recorded fields.
    Canonical { publication: String },
    /// Fuse a completed flat publication's rows and the chain's row maps.
    Stored { publication: String },
    /// Fuse a device source's rows and the chain's row maps.
    Source { source: String },
    /// Evaluate the Physical input at the host and upload it when it is not
    /// already a device source; the device fold reads the uploaded rows.
    Host {
        input: String,
        upload: Option<HostEvaluation>,
    },
}

/// Launch geometry of one generated or runtime kernel: one-dimensional
/// blocks of `block_threads` lanes with `shared_bytes` of dynamic shared memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockGeometry {
    pub block_threads: u32,
    pub shared_bytes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupCompletion {
    /// Keyed: an argument-extraction kernel (`arguments` grid, `geometry`
    /// blocks), then the recorded accumulator.
    Segmented {
        arguments: Grid,
        geometry: BlockGeometry,
    },
    /// Keyless: fused argument evaluation into per-block typed partials
    /// (`partials` grid, `geometry` blocks holding one i64 value and one u32
    /// presence word per lane), then one stream-ordered combine block per
    /// aggregate with `combine` geometry.
    GridPartials {
        partials: Grid,
        geometry: BlockGeometry,
        combine: BlockGeometry,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalGroup {
    /// Physical Group result folded at the terminal.
    pub group: String,
    pub input: GroupInput,
    pub completion: GroupCompletion,
}

/// How a Sequence definition is realized, in definition order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Transparent store alias of the terminal group's grouped publication;
    /// the terminal fold realizes it.
    GroupAlias,
    Group(TerminalGroup),
    /// The recorded dispatch publishing Physical `result`; nonterminal
    /// publications build their providers.
    Dispatch {
        result: String,
        provider: Option<ProviderBuild>,
    },
    HostUnion(HostUnion),
    /// A host publication uploaded for device consumers.
    Upload {
        evaluation: HostEvaluation,
        provider: Option<ProviderBuild>,
    },
    /// No device realization.
    Host,
}

/// A host-origin terminal publication packed on the device by one kernel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostOutput {
    pub source: String,
    pub pack: Grid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Capture {
    /// Query-closed value evaluated once at host, encoded through the
    /// compared column's recorded codec (identity when absent).
    Value {
        expression: syn::Expr,
        logical: syn::Type,
        logical_type_id: Option<cq_syntax::LogicalTypeId>,
        logical_column: Option<String>,
        codec: Option<syn::Type>,
        codec_id: Option<cq_lang_structures::CodecId>,
    },
    /// Verbatim device Rust: row reads are callable arguments; closed
    /// subexpressions are host-evaluated when the callable is enabled.
    Callable {
        reads: Vec<(syn::Ident, syn::Type)>,
        output: syn::Type,
        closed: Vec<syn::Expr>,
        body: syn::Expr,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Captures {
    /// Host bindings and answer gates, evaluated in checked source order.
    pub query: cq_lang_structures::syntax::Captures,
    /// Kernel parameter order.
    pub captures: Vec<Capture>,
    /// Captures read by each Physical definition.
    pub uses: BTreeMap<String, BTreeSet<usize>>,
    /// Device expression of each row function or relational selection.
    pub device: BTreeMap<String, syn::Expr>,
    /// Selection conditions with closed operands replaced by host values.
    pub host: BTreeMap<String, Theta>,
}

/// Launch geometry of each runtime auxiliary kernel family, printed into the
/// runtime's `Auxiliary` argument; the runtime rejects geometry its kernels
/// cannot run, never adjusting it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Auxiliary {
    /// Interval selection flags (positions access).
    pub selection: BlockGeometry,
    /// Deferred-range offset scan check.
    pub offsets: BlockGeometry,
    /// Range-provider build element kernels.
    pub range: BlockGeometry,
    /// Canonical (distinct) build element kernels.
    pub canon: BlockGeometry,
    /// Keyed group element kernels (order, key gather, boundaries).
    pub group: BlockGeometry,
    /// Keyed group reduction: one block per group.
    pub group_reduce: BlockGeometry,
    /// Bounds reduction of measured columns (uploads, provider descriptors).
    pub bounds: BlockGeometry,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Realization {
    /// The target cannot realize this graded program; printing rejects it
    /// with this reason. Grading alone (for example of planner candidates)
    /// does not require a realization.
    pub rejection: Option<String>,
    pub sources: BTreeMap<String, SourceRealization>,
    /// Lookup operands read as complete relations rather than keyed layouts.
    pub whole_lookups: BTreeSet<String>,
    pub steps: Vec<(String, Step)>,
    pub output: Option<HostOutput>,
    pub captures: Captures,
    /// PH-Smallest probes realized by exact candidate counts, by Physical probe.
    pub smallest: BTreeMap<String, SmallestRealization>,
    /// Present exactly when the realization is not rejected.
    pub auxiliary: Option<Auxiliary>,
}

/// One PH-Smallest side: a radix provider (rows-provider name) read under
/// the bound `prefix`, enumerating the realization's candidate column.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmallestSide {
    pub provider: String,
    pub prefix: Vec<String>,
}

/// The exact count compared between sides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CandidateCount {
    /// Distinct candidate keys over the union of all runs (never a sum of
    /// per-run or occurrence counts).
    ExactUnion,
}

/// Which side enumerates when several have the minimum count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ties {
    /// The first minimum side in recorded order.
    FirstInOrder,
}

/// How the parent row's choice reaches each candidate child row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChoiceCarrier {
    /// The parent counts every side and chooses once; each child position is
    /// a u32 tag: bit 31 marks a direct slot (the chosen side has one run, so
    /// its candidates are the contiguous key slots of the prefix interval),
    /// the next `choice_bits` bits hold the chosen side and the low
    /// `31 - choice_bits` bits the key slot or the distinct-candidate
    /// ordinal. Every side's build is checked at runtime to hold fewer than
    /// `2^(31 - choice_bits)` occurrences.
    PositionTag { choice_bits: u32 },
}

impl ChoiceCarrier {
    /// The carrier of `sides` alternatives: the fewest bits naming each.
    pub fn for_sides(sides: usize) -> Self {
        Self::PositionTag {
            choice_bits: usize::BITS - sides.saturating_sub(1).leading_zeros(),
        }
    }
}

/// Formal rule GPU-Smallest: per bound row, count every side's distinct
/// candidates exactly, enumerate the first minimum side's candidates (none
/// when it is empty) and keep a candidate iff every other side contains it.
/// The chain's later reads run as authored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmallestRealization {
    pub column: String,
    /// Every Physical side once, in PH-Smallest order.
    pub sides: Vec<SmallestSide>,
    pub count: CandidateCount,
    pub ties: Ties,
    pub carrier: ChoiceCarrier,
}
