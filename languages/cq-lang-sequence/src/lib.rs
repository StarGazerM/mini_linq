//! Sequence refines Structures with positions, symbolic lengths and explicit stores.
//!
//! A program appends `sequence { name = term; ... }` and an optional
//! `graded machine { name at level.path; ... }` to a complete Structures program.
//! Optional `evidence { ... }` records estimates and author-asserted key premises;
//! see [`Evidence`] and [`EvidenceFact`] for their distinct correctness contracts.
//! Terms are prefix forms: `rows`, `map`, `flatten`, `compact`, `tile`, `lookup`,
//! `partition_k[keys](parts)`, `fold`, `group`, `canon`, `store`, `union` and
//! `choose`, or references to earlier definitions. A key partition reads a named
//! ancestor store into fixed key-disjoint parts; scheduling guards may select a cover.
//! Constructor names are also valid references when followed by a term boundary.
//! A store may declare `store capacity N term`; the optional final sequence
//! statement `output count;`, `output reserve;`, `output reserve_previous;` or
//! `output bounded;` selects the final store's output protocol. Bounded output
//! takes its capacity from that store. `reserve_previous` is arena-terminal only:
//! the last validated count is an initial capacity hint (zero on first use),
//! never a bound; full counter and BE-Abort checks and overflow replay precede
//! publication. The optional statement `roots spread;`, after the definitions
//! and before `output`, records the root partition policy of every fold-free
//! dispatch (EXT-SeqRoots, GPU-RootSpread): root tiles are sized at launch to
//! fill the machine's resident blocks, never larger than the declared tile.
//! Codec and provider premises are analysis obligations, not syntax.

use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, LitInt, Token, braced, bracketed, parenthesized};

pub mod contract;
pub mod bounds;
mod evidence;
pub mod gpu;
mod grading;
mod resources;
pub mod task_execution;

pub use contract::check_rewrite;
pub use cq_lang_structures as structures;
pub use evidence::{Evidence, EvidenceFact, Selectivity};
pub use grading::check_grading;
use structures::Capability;

mod kw {
    syn::custom_keyword!(sequence);
    syn::custom_keyword!(graded);
    syn::custom_keyword!(at);
    syn::custom_keyword!(rows);
    syn::custom_keyword!(map);
    syn::custom_keyword!(flatten);
    syn::custom_keyword!(compact);
    syn::custom_keyword!(tile);
    syn::custom_keyword!(partition_k);
    syn::custom_keyword!(items);
    syn::custom_keyword!(lookup);
    syn::custom_keyword!(fold);
    syn::custom_keyword!(group);
    syn::custom_keyword!(canon);
    syn::custom_keyword!(store);
    syn::custom_keyword!(capacity);
    syn::custom_keyword!(choose);
    syn::custom_keyword!(count);
    syn::custom_keyword!(or);
    syn::custom_keyword!(sum);
    syn::custom_keyword!(len);
    syn::custom_keyword!(workers);
    syn::custom_keyword!(output);
    syn::custom_keyword!(roots);
    syn::custom_keyword!(spread);
    syn::custom_keyword!(reserve);
    syn::custom_keyword!(reserve_previous);
    syn::custom_keyword!(bounded);
    syn::custom_keyword!(representation);
    syn::custom_keyword!(identity);
    syn::custom_keyword!(encoded);
}

/// A complete Structures program and its sequence refinement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub structures: cq_lang_structures::Program,
    pub sequence: Sequence,
    pub graded: Option<Graded>,
    pub evidence: Evidence,
    pub representation: Representation,
}

// DESIGN contract7-representation
// Owns recorded `representation { identity, encoded }` facts; absent columns remain unknown.
// Contract 7, STR-Repr/BE-RustRepresentation; source: doc/PLANNING-REDESIGN.md Architecture.
// Source: doc/STORAGE.md §§1,4,9; doc/formal/CONFORMANCE.md BE-RustRepresentation.
// DO NOT infer identity, codecs, widths or preserved operations from Rust type spelling.
// Device Rust needs identity for every read column plus closed captures; encoded comparisons use `where`.
// Width/preserved-op and grouped-offset facts remain storage work in Plan P4-P6, not printer guesses.
/// Target-recorded device column representations; missing columns are unknown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Representation {
    pub identity: BTreeSet<String>,
    pub encoded: BTreeSet<String>,
}

impl Parse for Representation {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::representation>()?;
        let body;
        braced!(body in input);
        let mut result = Self::default();
        while !body.is_empty() {
            let columns = if body.peek(kw::identity) {
                body.parse::<kw::identity>()?;
                &mut result.identity
            } else {
                body.parse::<kw::encoded>()?;
                &mut result.encoded
            };
            let names;
            bracketed!(names in body);
            for name in Punctuated::<Ident, Token![,]>::parse_terminated(&names)? {
                columns.insert(cq_syntax::symbol_name(&name));
            }
            body.parse::<Token![;]>()?;
        }
        Ok(result)
    }
}

impl ToTokens for Representation {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        if self.identity.is_empty() && self.encoded.is_empty() {
            return;
        }
        let identity = self
            .identity
            .iter()
            .map(|name| Ident::new(name, Span::call_site()));
        let encoded = self
            .encoded
            .iter()
            .map(|name| Ident::new(name, Span::call_site()));
        tokens.extend(quote! { representation {
            identity [#(#identity),*]; encoded [#(#encoded),*];
        } });
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sequence {
    pub definitions: Vec<Definition>,
    pub roots: Roots,
    pub output: Option<OutputProtocol>,
}

/// Root partition policy of the program's dispatches (EXT-SeqRoots).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Roots {
    /// The declared distributed tile, or one part covering the source.
    #[default]
    Declared,
    /// Root tiles sized at launch from the resident block count, at most the
    /// declared tile (one block of rows when undeclared). Fold-free dispatches only.
    Spread,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub name: Ident,
    pub term: Term,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Term {
    Rows(Ident),
    Map(Ident, Box<Term>),
    Flatten(Box<Term>),
    Compact(Ident, Box<Term>),
    Tile {
        n: usize,
        items: Option<usize>,
        inner: Box<Term>,
    },
    /// Hash-route a stored valuation stream into a fixed key-disjoint cover.
    PartitionK {
        keys: Vec<Ident>,
        parts: usize,
        inner: Box<Term>,
    },
    Lookup {
        provider: Ident,
        keys: Vec<Ident>,
        inner: Box<Term>,
    },
    Fold(Monoid, Box<Term>),
    /// Host segmented fold over the complete distinct Physical input valuation.
    Group(Ident, Box<Term>),
    Canon(Box<Term>),
    Store {
        capacity: Option<usize>,
        inner: Box<Term>,
    },
    Choose(Vec<(Guard, Term)>),
    Ref(Ident),
    Union(Vec<Term>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Monoid {
    Count,
    Or,
    Sum(Ident),
}

/// Symbolic sizes. Only `Input`, `Lit` and `Add` occur in written guards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Len {
    Input(String),
    Scan(String),
    Lit(usize),
    Add(Box<Len>, Box<Len>),
    /// Number of contiguous parts covering a level of the given length.
    CeilDiv(Box<Len>, usize),
    Unknown(Option<usize>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Guard {
    Lt(Len, usize),
    Ge(Len, usize),
    /// Target-wide available workers; a uniform scheduling parameter, not a size.
    WorkersLt(usize),
    Else,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Graded {
    pub machine: Ident,
    pub grades: Vec<Grade>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grade {
    pub name: Ident,
    pub level: Vec<Ident>,
}

/// A target-supplied machine tree, not an additional language construct.
#[derive(Clone, Debug)]
pub struct Machine {
    /// Native GPU launch policy; absent on non-GPU machines.
    pub gpu: Option<gpu::Machine>,
    pub name: Ident,
    /// Preferred per-lane item count; the planner also considers one item.
    pub items: usize,
    /// Minimum published input rows for automatic routed provider builds.
    /// A positive target parameter, independent of cardinality estimates or workers.
    pub key_partition_threshold: usize,
    /// Minimum rows for automatic routing with fewer than two workers.
    /// Positive; the effective single-worker cutoff is the maximum of both thresholds.
    pub key_partition_cache_threshold: usize,
    pub root: LevelNode,
    /// Target-owned storage layout for one checked dispatch's resource demands.
    /// The language derives the shape; the machine alone assigns byte sizes.
    pub layout: Option<fn(&DispatchShape) -> syn::Result<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelNode {
    pub name: Ident,
    /// Whether this level executes host-compiled Rust rather than device Rust.
    pub host_rust: bool,
    pub collaborative: bool,
    pub dynamic: bool,
    /// Fixed distributing width, or `None` for dynamically sized levels.
    pub width: Option<usize>,
    /// Available shared bytes for a collective at this level, if bounded.
    pub shared_bytes: Option<usize>,
    pub children: Vec<LevelNode>,
}

/// Checked column facts supplied to a target's resource layout model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceColumn {
    pub name: String,
    pub types: Vec<syn::Type>,
    pub coded: bool,
}

/// Ordered lifetime demands, without a target's alignment or byte constants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceAllocation {
    Payload { parents: Vec<ResourceColumn> },
    Fold,
}

/// Resource-relevant structure of one checked dispatch, including every arm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchShape {
    pub publication: String,
    pub level: Vec<String>,
    pub width: Option<usize>,
    pub tile: Option<usize>,
    pub items: usize,
    pub chains: Vec<Vec<ResourceAllocation>>,
}

/// A resource obligation discharged against the target-supplied layout model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceUsage {
    pub shape: DispatchShape,
    pub required_bytes: usize,
    pub capacity_bytes: usize,
}

/// Ordered target catalog: token declarations and the read levels at which
/// this target supports each declared token (PL-Catalog).
///
/// Catalogs contain declarations and levels only, never target implementations.
/// Entries are keyed by token id (and, for a routed token, its inner token);
/// entry order breaks ties. Levels are explicit absolute machine paths;
/// an empty level list permits no read. Provider levels govern consumption,
/// not construction: building from host rows does not license host reads.
/// A program declaration of a token the catalog declares must equal it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    pub declarations: Vec<structures::Declaration>,
    pub providers: Vec<CatalogEntry>,
    pub codecs: Vec<CodecToken>,
}

/// A supported provider instance shape (token and inner tokens) with its
/// default spelling, readable at the listed levels. An entry that lists no
/// inner instance covers its token over any inner instance (a nested trie);
/// an entry that lists one covers only that inner token (a routed pair).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogEntry {
    pub provider: structures::Provider,
    pub levels: Vec<Vec<String>>,
}

impl CatalogEntry {
    /// Whether this entry supports a recorded instance (PL-Catalog), by token id only.
    pub fn covers(&self, instance: &structures::Provider) -> bool {
        fn covers(entry: &structures::Provider, instance: &structures::Provider) -> bool {
            entry.token == instance.token
                && match (&entry.inner, &instance.inner) {
                    (None, _) => true,
                    (Some(entry), Some(instance)) => covers(entry, instance),
                    (Some(_), None) => false,
                }
        }
        covers(&self.provider, instance)
    }
}

impl Catalog {
    pub fn declaration(&self, token: &Ident) -> Option<&structures::Declaration> {
        self.declarations
            .iter()
            .find(|declaration| declaration.token == *token)
    }

    /// Declared facts of a catalog instance shape, applying STR-Routed.
    pub fn facts(&self, provider: &structures::Provider) -> Option<structures::Facts> {
        structures::provider::facts(|token| self.declaration(token), provider)
    }

    /// The first entry supporting this recorded instance.
    pub fn entry(&self, provider: &structures::Provider) -> Option<&CatalogEntry> {
        self.providers.iter().find(|entry| entry.covers(provider))
    }

    /// Add a declaration and an entry of its default instance at `levels`.
    pub fn register(&mut self, declaration: structures::Declaration, levels: Vec<Vec<String>>) {
        self.providers.push(CatalogEntry {
            provider: declaration.instance(),
            levels,
        });
        if self.declaration(&declaration.token).is_none() {
            self.declarations.push(declaration);
        }
    }
}

/// A value-refinement token; `ordered` certifies monotone encoding. `codec`
/// is the exact codec a Structures input column declares; `None` is the
/// identity representation of a column without a declared codec.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodecToken {
    pub codec: Option<syn::Type>,
    pub ordered: bool,
    pub levels: Vec<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputProtocol {
    Count,
    Reserve,
    /// Arena-local previous validated count as a capacity hint, with checked replay.
    ReservePrevious,
    Bounded,
}

/// Whether a tile distributes publication parts or loops within one part.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Tiling {
    Distribute,
    Strip,
}

/// A dispatch derived from the grading, including its failure path to host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dispatch {
    /// Checked target launch/publication steps, printed with the dispatch.
    pub gpu: Option<gpu::Execution>,
    pub outer_store: String,
    pub members: Vec<String>,
    pub inner_stores: Vec<String>,
    pub level: Vec<String>,
    pub failure_channel: Vec<String>,
    /// Effective execution level of the producer; publication stays at `level`.
    pub producer_level: Vec<String>,
    /// Width of the outer tile whose parts are published by this dispatch.
    pub parts: Option<usize>,
    /// Recorded protocol for concatenating published parts.
    pub concatenation: OutputProtocol,
}

/// Runtime premises recorded for discharge by target traits and translations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Obligation {
    /// `columns` carry the exact recorded `codec` (the codec their logical
    /// type's Structures input columns declare; `None`: identity).
    Codec {
        columns: Vec<String>,
        ordered: bool,
        codec: Option<syn::Type>,
    },
    Provider {
        provider: String,
        keys: Vec<String>,
        capability: Capability,
        instance: structures::Provider,
    },
    /// The canon boundary owned by Sequence definition `result` reads this
    /// checked canonical reference; `None` is the returned answer boundary,
    /// whose recorded identity index is `answer_index` (Structures
    /// `head_dedup`). `level` is the boundary's graded execution level,
    /// recorded by grading (`None` before grading).
    Canon {
        result: String,
        canonical: Option<structures::CanonSource>,
        answer_index: Option<structures::Provider>,
        level: Option<Vec<String>>,
    },
    Failure {
        dispatch: usize,
        channel: Vec<String>,
    },
}

/// Checked ownership of a named publication. Views have no publication entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Publication {
    BorrowHost {
        result: String,
        host: String,
    },
    Alias {
        source: String,
    },
    /// `staged` publishes a raw row buffer for subsequent key routing.
    OwnRows {
        staged: bool,
    },
    /// Pure staged rows used only by this checked singleton/routed provider.
    /// A count or routed demand owns one buffer; the worker-singleton arm may
    /// consume the producer once without publishing a raw row buffer.
    DeferredRows {
        consumer: String,
    },
    Parts {
        keys: Vec<String>,
    },
}

/// Checked facts consumed by rewrites and target translations.
#[derive(Clone, Debug)]
pub struct Analysis {
    pub structures: cq_lang_structures::Analysis,
    /// Physical result denoted by each definition, when it is a row result.
    pub results: BTreeMap<String, String>,
    pub lengths: BTreeMap<String, Len>,
    /// Lengths from outermost partition to innermost rows, including tile bounds.
    /// `lengths` retains the total flattened cardinality for existing consumers.
    pub shapes: BTreeMap<String, Vec<Len>>,
    /// Tile interpretations in constructor preorder, grouped by definition.
    pub tilings: BTreeMap<String, Vec<Tiling>>,
    /// Checked host-merged partial-fold modes for Group constructors in preorder,
    /// grouped by owning definition. Ref uses retain their producer's entries.
    pub partial_groups: BTreeMap<String, Vec<bool>>,
    /// BE-PrefixBound: checked cardinality/key facts, indexed by Physical result.
    pub prefix_bounds: BTreeMap<String, bounds::Bound>,
    /// BE-GroupKey: owners whose group valuation streams remain unique after
    /// removal of their upstream canonicalizers.
    pub canon_free_groups: BTreeSet<String>,
    pub views: BTreeSet<String>,
    pub stores: BTreeMap<String, Len>,
    /// Publication ownership, derived once from checked provenance and consumers.
    pub publications: BTreeMap<String, Publication>,
    pub task_execution: task_execution::Execution,
    pub gpu_bindings: gpu::bindings::Bindings,
    /// Concrete GPU carrier storage, ordered key-codec recipes with their
    /// bounds sources, and packed-key equality granularity.
    pub gpu_publications: BTreeMap<String, gpu::publication::PublicationRecord>,
    /// Pure Rows/Ref/Store provenance, before any support-equal reheading.
    pub host_sources: BTreeMap<String, String>,
    /// Borrowable base provenance of Tile inputs, in constructor preorder.
    pub host_tiles: BTreeMap<String, Vec<Option<String>>>,
    /// Native routed-provider representation: result -> provider ordinal -> the
    /// recorded inner instance of its routed token (STR-Routed).
    pub routed_providers: BTreeMap<String, BTreeMap<usize, structures::Provider>>,
    pub capabilities: BTreeMap<(String, Vec<String>), Capability>,
    /// Providers evaluated directly by each named definition, excluding Ref
    /// dependencies and support-equal Map reheadings.
    pub provider_reads: BTreeMap<String, BTreeSet<String>>,
    pub levels: BTreeMap<String, Vec<String>>,
    pub dispatches: Vec<Dispatch>,
    pub resources: BTreeMap<String, ResourceUsage>,
    pub collectives: Vec<(String, Monoid, Vec<String>)>,
    pub output: Option<(String, OutputProtocol)>,
    pub obligations: Vec<Obligation>,
    /// Planning estimates and explicit author-asserted key premises.
    pub evidence: Evidence,
    pub representation: Representation,
}

impl Analysis {
    /// Rust row expressions require identity representations for every read column.
    /// Closed captures are evaluated by the host and do not read encoded rows.
    /// An unannotated computed binding has no authored device result layout.
    pub fn rust_requires_host(&self, operator: &cq_lang_structures::physical::Operator) -> bool {
        matches!(operator, cq_lang_structures::physical::Operator::Bind(bind)
            if bind.annotation.is_none())
            || self
                .structures
                .physical
                .rust_reads(operator)
                .is_some_and(|reads| !reads.is_subset(&self.representation.identity))
    }
}

impl Parse for Program {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        Ok(Self {
            structures: input.parse()?,
            sequence: input.parse()?,
            graded: if input.peek(kw::graded) {
                Some(input.parse()?)
            } else {
                None
            },
            evidence: if input.peek(evidence::kw::evidence) {
                input.parse()?
            } else {
                Evidence::default()
            },
            representation: if input.peek(kw::representation) {
                input.parse()?
            } else {
                Representation::default()
            },
        })
    }
}

impl ToTokens for Program {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self {
            structures,
            sequence,
            graded,
            evidence,
            representation,
        } = self;
        let evidence = (!evidence.facts.is_empty()).then_some(evidence);
        tokens.extend(quote! { #structures #sequence #graded #evidence #representation });
    }
}

impl Parse for Sequence {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::sequence>()?;
        let body;
        braced!(body in input);
        let mut definitions = Vec::new();
        let mut roots = Roots::Declared;
        let mut output = None;
        while !body.is_empty() {
            if body.peek(kw::roots) && !body.peek2(Token![=]) {
                body.parse::<kw::roots>()?;
                body.parse::<kw::spread>()?;
                body.parse::<Token![;]>()?;
                roots = Roots::Spread;
                if !(body.is_empty() || body.peek(kw::output) && !body.peek2(Token![=])) {
                    return Err(body.error("roots must follow every definition, before output"));
                }
            } else if body.peek(kw::output) && !body.peek2(Token![=]) {
                body.parse::<kw::output>()?;
                output = Some(body.parse()?);
                body.parse::<Token![;]>()?;
                if !body.is_empty() {
                    return Err(body.error("output must be the final statement in sequence"));
                }
            } else {
                definitions.push(body.parse()?);
            }
        }
        Ok(Self {
            definitions,
            roots,
            output,
        })
    }
}

impl ToTokens for Sequence {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let definitions = &self.definitions;
        let roots = (self.roots == Roots::Spread).then(|| quote! { roots spread; });
        let output = self
            .output
            .as_ref()
            .map(|protocol| quote! { output #protocol; });
        tokens.extend(quote! { sequence {
            #(#definitions)* #roots #output
        } });
    }
}

impl Parse for Definition {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![=]>()?;
        let term = input.parse()?;
        input.parse::<Token![;]>()?;
        Ok(Self { name, term })
    }
}

impl ToTokens for Definition {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self { name, term } = self;
        tokens.extend(quote! { #name = #term; });
    }
}

impl Parse for Term {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        // Every prefix constructor's final operand is a term, so a bare
        // identifier at a term boundary is always a reference, even when its
        // spelling is a constructor keyword.
        if input.peek(Ident) {
            let rest = input.fork();
            rest.parse::<Ident>()?;
            if rest.is_empty() || rest.peek(Token![;]) || rest.peek(Token![,]) {
                return Ok(Self::Ref(input.parse()?));
            }
        }
        if input.peek(Token![union]) {
            input.parse::<Token![union]>()?;
            let body;
            bracketed!(body in input);
            return Ok(Self::Union(
                Punctuated::<Term, Token![,]>::parse_terminated(&body)?
                    .into_iter()
                    .collect(),
            ));
        }
        if input.peek(kw::rows) {
            input.parse::<kw::rows>()?;
            Ok(Self::Rows(input.parse()?))
        } else if input.peek(kw::map) {
            input.parse::<kw::map>()?;
            Ok(Self::Map(input.parse()?, input.parse()?))
        } else if input.peek(kw::flatten) {
            input.parse::<kw::flatten>()?;
            Ok(Self::Flatten(input.parse()?))
        } else if input.peek(kw::compact) {
            input.parse::<kw::compact>()?;
            Ok(Self::Compact(input.parse()?, input.parse()?))
        } else if input.peek(kw::partition_k) {
            input.parse::<kw::partition_k>()?;
            let keys;
            bracketed!(keys in input);
            let keys = Punctuated::<Ident, Token![,]>::parse_terminated(&keys)?
                .into_iter()
                .collect();
            let parts;
            parenthesized!(parts in input);
            let count = parse_usize(&parts)?;
            if !parts.is_empty() {
                return Err(parts.error("partition_k expects one partition count"));
            }
            Ok(Self::PartitionK {
                keys,
                parts: count,
                inner: input.parse()?,
            })
        } else if input.peek(kw::tile) {
            input.parse::<kw::tile>()?;
            let n = parse_usize(input)?;
            let items = if input.peek(kw::items) {
                input.parse::<kw::items>()?;
                Some(parse_usize(input)?)
            } else {
                None
            };
            Ok(Self::Tile {
                n,
                items,
                inner: input.parse()?,
            })
        } else if input.peek(kw::lookup) {
            input.parse::<kw::lookup>()?;
            let provider = input.parse()?;
            let keys;
            bracketed!(keys in input);
            let keys = Punctuated::<Ident, Token![,]>::parse_terminated(&keys)?
                .into_iter()
                .collect();
            Ok(Self::Lookup {
                provider,
                keys,
                inner: input.parse()?,
            })
        } else if input.peek(kw::fold) {
            input.parse::<kw::fold>()?;
            Ok(Self::Fold(input.parse()?, input.parse()?))
        } else if input.peek(kw::group) {
            input.parse::<kw::group>()?;
            Ok(Self::Group(input.parse()?, input.parse()?))
        } else if input.peek(kw::canon) {
            input.parse::<kw::canon>()?;
            Ok(Self::Canon(input.parse()?))
        } else if input.peek(kw::store) {
            input.parse::<kw::store>()?;
            let capacity = if input.peek(kw::capacity) && input.peek2(LitInt) {
                input.parse::<kw::capacity>()?;
                Some(parse_usize(input)?)
            } else {
                None
            };
            Ok(Self::Store {
                capacity,
                inner: input.parse()?,
            })
        } else if input.peek(kw::choose) {
            input.parse::<kw::choose>()?;
            let body;
            bracketed!(body in input);
            let mut alternatives = Vec::new();
            loop {
                let guard = body.parse()?;
                body.parse::<Token![=>]>()?;
                alternatives.push((guard, body.parse()?));
                if body.is_empty() {
                    break;
                }
                body.parse::<Token![,]>()?;
            }
            Ok(Self::Choose(alternatives))
        } else {
            Ok(Self::Ref(input.parse()?))
        }
    }
}

impl ToTokens for Term {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match self {
            Self::Union(arms) => quote! { union [#(#arms),*] },
            Self::Rows(name) => quote! { rows #name },
            Self::Map(name, inner) => quote! { map #name #inner },
            Self::Flatten(inner) => quote! { flatten #inner },
            Self::Compact(name, inner) => quote! { compact #name #inner },
            Self::PartitionK { keys, parts, inner } => {
                let parts = integer(*parts);
                quote! { partition_k [#(#keys),*](#parts) #inner }
            }
            Self::Tile { n, items, inner } => {
                let n = integer(*n);
                let items = items.map(|n| {
                    let n = integer(n);
                    quote! { items #n }
                });
                quote! { tile #n #items #inner }
            }
            Self::Lookup {
                provider,
                keys,
                inner,
            } => {
                quote! { lookup #provider [#(#keys),*] #inner }
            }
            Self::Fold(monoid, inner) => quote! { fold #monoid #inner },
            Self::Group(name, inner) => quote! { group #name #inner },
            Self::Canon(inner) => quote! { canon #inner },
            Self::Store { capacity, inner } => {
                let capacity = capacity.map(|n| {
                    let n = integer(n);
                    quote! { capacity #n }
                });
                quote! { store #capacity #inner }
            }
            Self::Choose(alternatives) => {
                let alternatives = alternatives.iter().map(|(guard, term)| {
                    quote! { #guard => #term }
                });
                quote! { choose [#(#alternatives),*] }
            }
            Self::Ref(name) => quote! { #name },
        });
    }
}

impl Parse for Monoid {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let lookahead = input.lookahead1();
        if lookahead.peek(kw::count) {
            input.parse::<kw::count>()?;
            Ok(Self::Count)
        } else if lookahead.peek(kw::or) {
            input.parse::<kw::or>()?;
            Ok(Self::Or)
        } else if lookahead.peek(kw::sum) {
            input.parse::<kw::sum>()?;
            Ok(Self::Sum(input.parse()?))
        } else {
            Err(lookahead.error())
        }
    }
}

impl ToTokens for Monoid {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match self {
            Self::Count => quote! { count },
            Self::Or => quote! { or },
            Self::Sum(column) => quote! { sum #column },
        });
    }
}

fn parse_usize(input: ParseStream<'_>) -> syn::Result<usize> {
    input.parse::<LitInt>()?.base10_parse()
}

fn integer(value: usize) -> LitInt {
    LitInt::new(&value.to_string(), Span::call_site())
}

fn parse_len_atom(input: ParseStream<'_>) -> syn::Result<Len> {
    if input.peek(kw::len) {
        input.parse::<kw::len>()?;
        Ok(Len::Input(cq_syntax::symbol_name(&input.parse::<Ident>()?)))
    } else {
        Ok(Len::Lit(parse_usize(input)?))
    }
}

impl Parse for Len {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let mut len = parse_len_atom(input)?;
        while input.peek(Token![+]) {
            input.parse::<Token![+]>()?;
            len = Self::Add(Box::new(len), Box::new(parse_len_atom(input)?));
        }
        Ok(len)
    }
}

impl ToTokens for Len {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            Self::Input(name) => match syn::parse_str::<Ident>(name)
                .or_else(|_| syn::parse_str::<Ident>(&format!("r#{name}")))
            {
                Ok(name) => tokens.extend(quote! { len #name }),
                Err(error) => error.to_compile_error().to_tokens(tokens),
            },
            Self::Lit(n) => integer(*n).to_tokens(tokens),
            Self::Add(left, right) => tokens.extend(quote! { #left + #right }),
            Self::Scan(_) | Self::Unknown(_) | Self::CeilDiv(_, _) => {
                syn::Error::new(
                    Span::call_site(),
                    "analysis-only length cannot appear in a guard",
                )
                .to_compile_error()
                .to_tokens(tokens);
            }
        }
    }
}

impl Parse for Guard {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(Token![else]) {
            input.parse::<Token![else]>()?;
            return Ok(Self::Else);
        }
        if input.peek(kw::workers) {
            input.parse::<kw::workers>()?;
            input.parse::<Token![<]>()?;
            return Ok(Self::WorkersLt(parse_usize(input)?));
        }
        let len = input.parse()?;
        let lookahead = input.lookahead1();
        if lookahead.peek(Token![<]) {
            input.parse::<Token![<]>()?;
            Ok(Self::Lt(len, parse_usize(input)?))
        } else if lookahead.peek(Token![>=]) {
            input.parse::<Token![>=]>()?;
            Ok(Self::Ge(len, parse_usize(input)?))
        } else {
            Err(lookahead.error())
        }
    }
}

impl ToTokens for Guard {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match self {
            Self::Lt(len, threshold) => {
                let threshold = integer(*threshold);
                quote! { #len < #threshold }
            }
            Self::Ge(len, threshold) => {
                let threshold = integer(*threshold);
                quote! { #len >= #threshold }
            }
            Self::WorkersLt(threshold) => {
                let threshold = integer(*threshold);
                quote! { workers < #threshold }
            }
            Self::Else => quote! { else },
        });
    }
}

impl Parse for Graded {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::graded>()?;
        let machine = input.parse()?;
        let body;
        braced!(body in input);
        let mut grades = Vec::new();
        while !body.is_empty() {
            grades.push(body.parse()?);
        }
        Ok(Self { machine, grades })
    }
}

impl ToTokens for Graded {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self { machine, grades } = self;
        tokens.extend(quote! { graded #machine { #(#grades)* } });
    }
}

impl Parse for Grade {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<kw::at>()?;
        let mut level = vec![input.parse()?];
        while input.peek(Token![.]) {
            input.parse::<Token![.]>()?;
            level.push(input.parse()?);
        }
        input.parse::<Token![;]>()?;
        Ok(Self { name, level })
    }
}

impl ToTokens for Grade {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let Self { name, level } = self;
        tokens.extend(quote! { #name at #(#level).*; });
    }
}

impl Parse for OutputProtocol {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let lookahead = input.lookahead1();
        if lookahead.peek(kw::count) {
            input.parse::<kw::count>()?;
            Ok(Self::Count)
        } else if lookahead.peek(kw::reserve) {
            input.parse::<kw::reserve>()?;
            Ok(Self::Reserve)
        } else if lookahead.peek(kw::reserve_previous) {
            input.parse::<kw::reserve_previous>()?;
            Ok(Self::ReservePrevious)
        } else if lookahead.peek(kw::bounded) {
            input.parse::<kw::bounded>()?;
            Ok(Self::Bounded)
        } else {
            Err(lookahead.error())
        }
    }
}

impl ToTokens for OutputProtocol {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        tokens.extend(match self {
            Self::Count => quote! { count },
            Self::Reserve => quote! { reserve },
            Self::ReservePrevious => quote! { reserve_previous },
            Self::Bounded => quote! { bounded },
        });
    }
}

impl cq_stage::Language for Program {
    fn context(&self) -> cq_stage::Context {
        cq_stage::Language::context(&self.structures)
    }

    fn check(&self) -> syn::Result<()> {
        contract::check(self).map(|_| ())
    }

    fn check_rewrite(source: &Self, target: &Self) -> syn::Result<()> {
        check_rewrite(source, target)
    }
}
