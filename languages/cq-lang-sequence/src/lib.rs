//! Sequence refines Structures with positions, symbolic lengths and explicit stores.
//!
//! A program appends `sequence { name = term; ... }` and an optional
//! `graded machine { name at level.path; ... }` to a complete Structures program.
//! Optional `evidence { ... }` records estimates and author-asserted key premises;
//! see [`Evidence`] and [`EvidenceFact`] for their distinct correctness contracts.
//! Terms are prefix forms: `rows`, `map`, `flatten`, `compact`, `tile`, `lookup`,
//! `partition_k[keys](parts)`, `fold`, `group`, `canon`, `store`, `union` and
//! `choose`, or references to earlier definitions. A key partition reads a named
//! ancestor store into fixed key-disjoint parts; runtime worker count is unrelated.
//! Constructor names are also valid references when followed by a term boundary.
//! A store may declare `store capacity N term`; the optional final sequence
//! statement `output count;`, `output reserve;` or `output bounded;` selects the
//! final store's output protocol. Bounded output takes its capacity from that
//! store. Codec and provider premises are analysis obligations, not syntax.

use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Ident, LitInt, Token, braced, bracketed, parenthesized};

pub mod contract;
mod evidence;
mod grading;
mod resources;

pub use contract::check_rewrite;
pub use cq_lang_structures as structures;
pub use evidence::{Evidence, EvidenceFact, Selectivity};
pub use grading::check_grading;

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
    syn::custom_keyword!(output);
    syn::custom_keyword!(reserve);
    syn::custom_keyword!(bounded);
}

/// A complete Structures program and its sequence refinement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub structures: cq_lang_structures::Program,
    pub sequence: Sequence,
    pub graded: Option<Graded>,
    pub evidence: Evidence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sequence {
    pub definitions: Vec<Definition>,
    pub output: Option<OutputProtocol>,
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
    pub name: Ident,
    /// Preferred per-lane item count; the planner also considers one item.
    pub items: usize,
    /// Minimum published input rows for automatic routed provider builds.
    /// A positive target parameter, independent of cardinality estimates or workers.
    pub key_partition_threshold: usize,
    pub root: LevelNode,
    /// Target-owned storage layout for one checked dispatch's resource demands.
    /// The language derives the shape; the machine alone assigns byte sizes.
    pub layout: Option<fn(&DispatchShape) -> syn::Result<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LevelNode {
    pub name: Ident,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Capability {
    Exact,
    Range,
    Membership,
    Ordered,
}

/// Ordered target tokens used to discharge planner obligations.
///
/// Catalogs contain syntax and capabilities only, never target implementations.
/// Candidate order breaks ties. Levels are explicit absolute machine paths;
/// an empty level list permits no read. Provider levels govern consumption,
/// not construction: building from host rows does not license host reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    pub providers: Vec<ProviderToken>,
    pub codecs: Vec<CodecToken>,
    pub canons: Vec<CanonToken>,
}

/// An authored provider type whose reads are supported at the listed levels.
///
/// In token patterns, `_` matches any type or const argument and `'_` any lifetime.
/// Matching never replaces the author's provider type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderToken {
    pub ty: syn::Type,
    pub capabilities: Vec<Capability>,
    pub levels: Vec<Vec<String>>,
    /// Accepted scalar row elements; `None` leaves the element type unrestricted.
    pub elements: Option<Vec<syn::Type>>,
}

/// A value-refinement token; `ordered` certifies monotone encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodecToken {
    pub ty: syn::Type,
    pub ordered: bool,
    pub levels: Vec<Vec<String>>,
}

/// A canonicalization token supported at the listed execution levels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonToken {
    pub ty: syn::Type,
    pub levels: Vec<Vec<String>>,
    /// Packed equality-key capability, checked against codec bounds at setup.
    /// `None` denotes a provider without a fixed packed-width limit.
    pub max_packed_bits: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutputProtocol {
    Count,
    Reserve,
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
    Codec {
        columns: Vec<String>,
        ordered: bool,
    },
    Provider {
        provider: String,
        keys: Vec<String>,
        capability: Capability,
        ty: syn::Type,
    },
    Canon {
        result: String,
        ty: Option<syn::Type>,
    },
    Failure {
        dispatch: usize,
        channel: Vec<String>,
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
    pub views: BTreeSet<String>,
    pub stores: BTreeMap<String, Len>,
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
        } = self;
        let evidence = (!evidence.facts.is_empty()).then_some(evidence);
        tokens.extend(quote! { #structures #sequence #graded #evidence });
    }
}

impl Parse for Sequence {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::sequence>()?;
        let body;
        braced!(body in input);
        let mut definitions = Vec::new();
        let mut output = None;
        while !body.is_empty() {
            if body.peek(kw::output) && !body.peek2(Token![=]) {
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
            output,
        })
    }
}

impl ToTokens for Sequence {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        let definitions = &self.definitions;
        let output = self
            .output
            .as_ref()
            .map(|protocol| quote! { output #protocol; });
        tokens.extend(quote! { sequence { #(#definitions)* #output } });
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
