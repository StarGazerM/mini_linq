//! Structures records providers, retained rows or positions, and read adapters.
//! Host reads record either a native read the input declares (`native`,
//! STR-NativeHost) or a lazily built fallback provider (`host`). Explicit
//! positional decisions name a base, total column lineage, marker and summary;
//! generated Rust checks the asserted host and provider capabilities.
//! Every provider is a declared token (STR-Provider, `provider.rs`): entries
//! record `@token as Type`, the `providers` table declares each token's facts.
//! Provider Rust spellings carry no meaning here: trait capabilities,
//! including equality and ordered access, remain generated Rust obligations.
//! Structural checks still enforce key coverage, lineage and read ownership.
//! Host key levels retain their recorded order; their flattened, sorted key set
//! must exactly match the named read. A scan has one empty level. Printers may
//! impose stricter representation-specific level requirements.
//! Providers may declare `supports [retain, expand, keys]` on a keyed build or
//! host fallback. Expansion implies retention, but neither implies ordered or
//! contiguous access. Checks use the selected layout and the token's declared
//! facts, never the provider's spelling; generated Rust must discharge every declared capability and
//! ordinary read trait obligation. Missing metadata grants no optimization.
//! Smallest intersections require two candidate levels on every retained side,
//! alongside any structures needed by other reads of the same result.
//! Candidate reads select that exact two-level directory independently of an
//! ordinary complete payload provider on the same prefix.
//! Rust expressions are opaque and total by premise, placed by free variables.
//! Const supplies use one point read and never justify a build.
//! ```text
//! Structures ::= "structures" "{" Providers Entry* "}"
//! Entry ::= Ident ("=" Provider ("[" Ident,* "]")+ [Support] [Items]
//!                | "host" ReaderRef "=" Provider ("[" Ident,* "]")+ [Support]
//!                | "native" ReaderRef "=" HostRead "[" Ident,* "]" [Support]
//!                | ("rows" | "head" | "group") "=" Provider
//!                | "canon" "=" ("rows" | "[" Ident,* "]" | "host")
//!                | "selection" "=" Provider Origin "summary" Summary
//!                | "keyset" "=" Provider "[" Ident,* "]" Ownership Origin
//!                | "read" "=" "positional" Origin "column" Ident
//!                    "marker" RustType "predicate" ReadPredicate
//!                | "repr" "=" ("tuples" | "positions" "{"
//!                    (Ident "lineage" "{" (Ident "->" Ident),* "}"),*
//!                    "}" "owned" "[" Ident,* "]")
//!                    ["witness" "{" Ident "[" Ident,* "]" "at" Ident "as" RustType "}"]
//!                | "share" "=" "lookup" Ident "[" Ident,* "]" "first" Ident "force" Ident
//!                | "stage" "=" "batch" Integer ["expand"]) ";"
//! Support ::= "supports" "[" ("retain" | "expand" | "keys"),* "]"
//! Items ::= "items" ("rows" | "columns")
//! Origin ::= "base" Ident "lineage" "{" (Ident "->" Ident),* "}"
//! ReaderRef ::= "scan" | "candidates" | "membership" | "result" "(" Ident ")"
//! ```
//!
//! Relational equality is Leibniz identity, decided by admissible column `==`.
//! Functional extensionality of shared-reference Rust inputs permits canonical
//! projection whenever free variables remain; see the premises in
//! `cq-syntax`.

//! Structures records canonicalization strategies and anti membership representations.
//! A derived union used only by a shared-key index streams its chained scan directly into the filtered build path, without a retained row buffer.
//!
//! Core blocks denote sets: `{ B1 } | { B2 }` unions projected answers;
//! `{ B1 } else { B2 }` selects the first nonempty arm. F is the free heading
//! and I the common new interface; other binders remain branch-local.
//! Join distributes over union; fallback is `B1 union (B2 anti_F B1)`.
//! Only explicit relational predicates introduce comparisons and constraints.
//!
pub use cq_lang_physical as physical;

pub mod blocks;
pub mod encoded;
pub mod provider;
pub use provider::{Capability, CodecId, Declaration, Facts, LogicalTypeId, Provider, Providers};
pub mod stats;

use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet};

use syn::{Ident, Token, token};

pub use cq_syntax::{CommaList, symbol_name};
pub use cq_stage::OutputConsumer;

pub mod kw {
    syn::custom_keyword!(structures);
    syn::custom_keyword!(host);
    syn::custom_keyword!(native);
    syn::custom_keyword!(scan);
    syn::custom_keyword!(candidates);
    syn::custom_keyword!(result);
    syn::custom_keyword!(group);
    syn::custom_keyword!(canon);
    syn::custom_keyword!(head);
    syn::custom_keyword!(rows);
    syn::custom_keyword!(payload);
    syn::custom_keyword!(constants);
    syn::custom_keyword!(selection);
    syn::custom_keyword!(keyset);
    syn::custom_keyword!(read);
    syn::custom_keyword!(positional);
    syn::custom_keyword!(base);
    syn::custom_keyword!(lineage);
    syn::custom_keyword!(summary);
    syn::custom_keyword!(none);
    syn::custom_keyword!(per_block);
    syn::custom_keyword!(column);
    syn::custom_keyword!(marker);
    syn::custom_keyword!(predicate);
    syn::custom_keyword!(physical);
    syn::custom_keyword!(membership);
    syn::custom_keyword!(exact);
    syn::custom_keyword!(row);
    syn::custom_keyword!(owned);
    syn::custom_keyword!(borrowed);
    syn::custom_keyword!(join_filter);
    syn::custom_keyword!(build);
    syn::custom_keyword!(covering);
    syn::custom_keyword!(policy);
    syn::custom_keyword!(always);
    syn::custom_keyword!(abandon);
    syn::custom_keyword!(key_count);
    syn::custom_keyword!(of);
    syn::custom_keyword!(repr);
    syn::custom_keyword!(positions);
    syn::custom_keyword!(tuples);
    syn::custom_keyword!(stage);
    syn::custom_keyword!(batch);
    syn::custom_keyword!(share);
    syn::custom_keyword!(lookup);
    syn::custom_keyword!(first);
    syn::custom_keyword!(force);
    syn::custom_keyword!(witness);
    syn::custom_keyword!(at);
    syn::custom_keyword!(expand);
    syn::custom_keyword!(supports);
    syn::custom_keyword!(retain);
    syn::custom_keyword!(keys);
    syn::custom_keyword!(items);
    syn::custom_keyword!(columns);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub struct Structures {
    pub structures_token: kw::structures,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    pub providers: Providers,
    #[syn(in = brace)]
    pub publications: stats::Publications,
    #[syn(in = brace)]
    #[to_tokens(|tokens, entries| quote::TokenStreamExt::append_all(tokens, entries))]
    pub entries: Vec<Entry>,
}

impl syn::parse::Parse for Structures {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        Self::parse_declared(input, &[])
    }
}

impl Structures {
    fn parse_declared(
        input: syn::parse::ParseStream<'_>,
        declarations: &[cq_syntax::RelationDecl],
    ) -> syn::Result<Self> {
        let structures_token = input.parse()?;
        let content;
        let brace = syn::braced!(content in input);
        let providers = content.parse()?;
        let publications = content.parse()?;
        let entries = parse_entries(&content)?;
        let mut structures = Self {
            structures_token,
            brace,
            providers,
            publications,
            entries,
        };
        structures.intern_input_declarations(declarations)?;
        Ok(structures)
    }
}

fn parse_entries(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    while !input.is_empty() {
        entries.push(input.parse()?);
    }
    Ok(entries)
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Entry {
    pub name: Ident,
    pub structure: Structure,
    pub semi_token: Token![;],
}

/// The canonical reference of a `canon` result: one complete ordinary index of
/// that result, recorded by index selection (SRDatalog `canonical_index`).
/// `rows` names the result's stored rows record (its identity index: a
/// complete-row provider whose build yields distinct rows, SRDatalog's sorted
/// array); `[a, ..]` names an existing one-level Built entry on exactly these
/// columns that the result's reads already use; `host` names the result's
/// load-verified host carrier. No store, provider kind or type is introduced:
/// the referenced index is an ordinary member of the result's storage.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum CanonIndex {
    #[parse(peek = kw::rows)]
    Rows(kw::rows),
    #[parse(peek = kw::host)]
    Host(kw::host),
    #[parse(peek = token::Bracket)]
    Built(Level),
}

/// Checked canonical reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CanonSource {
    /// The stored rows record, of this provider.
    Rows(Provider),
    /// The one-level Built entry on these (sorted) complete-row columns.
    Index(Vec<String>),
    /// The load-verified host carrier.
    Host,
}

impl CanonSource {
    /// The provider of a canonical rows record.
    pub fn rows_provider(&self) -> Option<&Provider> {
        match self {
            Self::Rows(provider) => Some(provider),
            Self::Index(_) | Self::Host => None,
        }
    }
}

/// Host roles occupy a different namespace from Physical result identifiers.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReaderRef<I = Ident> {
    Scan,
    Candidates,
    Membership,
    Result(I),
}

impl<I> ReaderRef<I> {
    pub fn result(&self) -> Option<&I> {
        match self {
            Self::Result(name) => Some(name),
            _ => None,
        }
    }
}

impl ReaderRef {
    pub fn key(&self) -> ReaderRef<String> {
        match self {
            Self::Scan => ReaderRef::Scan,
            Self::Candidates => ReaderRef::Candidates,
            Self::Membership => ReaderRef::Membership,
            Self::Result(name) => ReaderRef::Result(symbol_name(name)),
        }
    }
}

/// Formal rule STR-Reader: Parse disjoint host roles and explicitly named results.
impl syn::parse::Parse for ReaderRef {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(kw::scan) {
            input.parse::<kw::scan>()?;
            Ok(Self::Scan)
        } else if input.peek(kw::candidates) {
            input.parse::<kw::candidates>()?;
            Ok(Self::Candidates)
        } else if input.peek(kw::membership) {
            input.parse::<kw::membership>()?;
            Ok(Self::Membership)
        } else {
            input.parse::<kw::result>()?;
            let body;
            syn::parenthesized!(body in input);
            let name = body.parse()?;
            if !body.is_empty() {
                return Err(body.error("expected one Physical result identifier"));
            }
            Ok(Self::Result(name))
        }
    }
}

impl ToTokens for ReaderRef {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        tokens.extend(match self {
            Self::Scan => quote::quote!(scan),
            Self::Candidates => quote::quote!(candidates),
            Self::Membership => quote::quote!(membership),
            Self::Result(name) => quote::quote!(result(#name)),
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Structure {
    #[parse(peek = kw::share)]
    Share {
        share_token: kw::share,
        eq_token: Token![=],
        lookup_token: kw::lookup,
        build: Ident,
        keys: Level,
        first_token: kw::first,
        first: Ident,
        force_token: kw::force,
        force: Ident,
    },
    #[parse(peek = kw::repr)]
    Repr {
        repr_token: kw::repr,
        eq_token: Token![=],
        repr: Repr,
        #[parse(repr::parse_witness)]
        witness: Option<Witness>,
    },
    #[parse(peek = kw::stage)]
    Stage {
        stage_token: kw::stage,
        eq_token: Token![=],
        batch_token: kw::batch,
        size: syn::LitInt,
        expand: Option<kw::expand>,
    },
    #[parse(peek = kw::join_filter)]
    JoinFilter {
        join_filter_token: kw::join_filter,
        eq_token: Token![=],
        filter: JoinFilter,
    },
    #[parse(peek = kw::selection)]
    Selection {
        selection_token: kw::selection,
        eq_token: Token![=],
        provider: Provider,
        origin: Origin,
        summary_token: kw::summary,
        summary: Summary,
    },
    #[parse(peek = kw::keyset)]
    KeySet {
        keyset_token: kw::keyset,
        eq_token: Token![=],
        provider: Provider,
        key: Level,
        ownership: KeyOwnership,
        origin: Origin,
    },
    #[parse(peek = kw::read)]
    Read {
        read_token: kw::read,
        eq_token: Token![=],
        positional_token: kw::positional,
        origin: Origin,
        column_token: kw::column,
        column: Ident,
        marker_token: kw::marker,
        marker: syn::Type,
        predicate_token: kw::predicate,
        predicate: ReadPredicate,
    },
    #[parse(peek = kw::native)]
    Native {
        native_token: kw::native,
        reader: ReaderRef,
        eq_token: Token![=],
        read: NativeRead,
    },
    #[parse(peek = kw::host)]
    Host {
        host_token: kw::host,
        reader: ReaderRef,
        eq_token: Token![=],
        keyed: Box<Keyed>,
    },
    #[parse(peek = kw::canon)]
    Dedup {
        canon_token: kw::canon,
        eq_token: Token![=],
        index: CanonIndex,
    },
    #[parse(peek = kw::head)]
    HeadDedup {
        head_token: kw::head,
        eq_token: Token![=],
        provider: Provider,
    },
    #[parse(peek = kw::rows)]
    Rows {
        rows_token: kw::rows,
        eq_token: Token![=],
        provider: Provider,
    },
    #[parse(peek = kw::group)]
    Group {
        group_token: kw::group,
        eq_token: Token![=],
        provider: Provider,
    },
    #[parse(peek = Token![=])]
    Built {
        eq_token: Token![=],
        keyed: Box<Keyed>,
    },
}

/// `@token as Type [a, b]` or `[a][b]`: a provider built over the result,
/// keyed on these columns, one bracket per level.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Keyed {
    pub provider: Provider,
    #[parse(parse_levels)]
    #[to_tokens(|tokens, levels| quote::TokenStreamExt::append_all(tokens, levels))]
    pub levels: Vec<Level>,
    #[parse(parse_support)]
    pub supports: Option<ProviderSupport>,
    #[parse(parse_items)]
    pub items: Option<Items>,
    pub membership: Option<kw::membership>,
    #[parse(parse_filtered)]
    pub filtered: Option<Filtered>,
}

/// Provider-authored read laws, independently of its Rust type spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProviderCapability {
    Retain,
    Expand,
    Keys,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum ProviderCapabilityToken {
    #[parse(peek = kw::retain)]
    Retain(kw::retain),
    #[parse(peek = kw::expand)]
    Expand(kw::expand),
    #[parse(peek = kw::keys)]
    Keys(kw::keys),
}

impl ProviderCapabilityToken {
    pub fn capability(&self) -> ProviderCapability {
        match self {
            Self::Retain(_) => ProviderCapability::Retain,
            Self::Expand(_) => ProviderCapability::Expand,
            Self::Keys(_) => ProviderCapability::Keys,
        }
    }
}

/// Explicit contracts checked by generated Rust trait obligations.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct ProviderSupport {
    pub supports_token: kw::supports,
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    #[parse(CommaList::parse_terminated)]
    pub capabilities: CommaList<ProviderCapabilityToken>,
}

impl ProviderSupport {
    pub fn supports(&self, capability: ProviderCapability) -> bool {
        self.capabilities.iter().any(|token| {
            let declared = token.capability();
            declared == capability
                || capability == ProviderCapability::Retain
                    && declared == ProviderCapability::Expand
        })
    }
}

fn parse_support(input: syn::parse::ParseStream<'_>) -> syn::Result<Option<ProviderSupport>> {
    if input.peek(kw::supports) {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
}

/// The payload layout of a one-level payload build (STR-ItemLayout), chosen
/// by index selection from the recorded read needs. `rows`
/// stores complete payload rows contiguously; `columns` stores one vector per
/// payload field, so a reader that needs some fields reads only those. The
/// provider token declares the layout argument and both spellings
/// ([`provider::ItemSlot`]); `Analysis::built` carries the provider spelling
/// with this fact printed there, and the instance spelling never states it.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Items {
    pub items_token: kw::items,
    pub layout: ItemLayout,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum ItemLayout {
    #[parse(peek = kw::rows)]
    Rows(kw::rows),
    #[parse(peek = kw::columns)]
    Columns(kw::columns),
}

fn parse_items(input: syn::parse::ParseStream<'_>) -> syn::Result<Option<Items>> {
    if input.peek(kw::items) {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
}

/// A filtered index retains this projected row schema. Key columns are
/// extracted from it; the remaining columns form each stored item. Constant
/// columns are reconstructed from the selection's program text.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Filtered {
    pub payload_token: kw::payload,
    pub payload: Level,
    pub constants_token: kw::constants,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    #[parse(CommaList::parse_terminated)]
    pub constants: CommaList<ConstantBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct ConstantBinding {
    pub column: Ident,
    pub eq_token: Token![=],
    pub value: syntax::constraint::Term,
}

fn parse_filtered(input: syn::parse::ParseStream<'_>) -> syn::Result<Option<Filtered>> {
    if input.peek(kw::payload) {
        input.parse().map(Some)
    } else {
        Ok(None)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Level {
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<Ident>,
}

fn parse_levels(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<Level>> {
    let mut levels = vec![input.parse()?];
    while input.peek(token::Bracket) {
        levels.push(input.parse()?);
    }
    Ok(levels)
}

impl Keyed {
    pub fn supports(&self, capability: ProviderCapability) -> bool {
        self.supports
            .as_ref()
            .is_some_and(|support| support.supports(capability))
    }

    fn check_support(&self) -> syn::Result<()> {
        let Some(support) = &self.supports else {
            return Ok(());
        };
        if self.levels.len() > 1 && self.levels.iter().any(|level| level.columns.is_empty()) {
            return error(
                support,
                "provider support requires nonempty levels in a multi-level layout",
            );
        }
        let mut seen = BTreeSet::new();
        for token in &support.capabilities {
            if !seen.insert(token.capability()) {
                return error(token, "duplicate provider capability");
            }
        }
        if self.membership.is_some() && self.supports(ProviderCapability::Retain) {
            return error(
                support,
                "membership-only storage cannot retain or expand payload cursors",
            );
        }
        Ok(())
    }

    /// Every column across all levels, level by level.
    pub fn columns(&self) -> impl Iterator<Item = &Ident> {
        self.levels.iter().flat_map(|l| l.columns.iter())
    }

    /// Sorted column names of each level.
    pub fn level_names(&self) -> Vec<Vec<String>> {
        self.levels.iter().map(|l| names(&l.columns)).collect()
    }
}

fn names(columns: &CommaList<Ident>) -> Vec<String> {
    let mut names: Vec<String> = columns.iter().map(symbol_name).collect();
    names.sort();
    names
}

impl Structure {
    /// The recorded provider of a provider-bearing entry and its instance support.
    pub fn provider(&self) -> Option<(&Provider, Option<&ProviderSupport>)> {
        match self {
            Structure::Built { keyed, .. } | Structure::Host { keyed, .. } => {
                Some((&keyed.provider, keyed.supports.as_ref()))
            }
            Structure::Selection { provider, .. }
            | Structure::KeySet { provider, .. }
            | Structure::HeadDedup { provider, .. }
            | Structure::Rows { provider, .. }
            | Structure::Group { provider, .. } => Some((provider, None)),
            Structure::JoinFilter { filter, .. } => Some((&filter.provider, None)),
            Structure::Share { .. }
            | Structure::Repr { .. }
            | Structure::Stage { .. }
            | Structure::Read { .. }
            | Structure::Native { .. }
            | Structure::Dedup { .. } => None,
        }
    }

    /// Mutable access to the recorded provider of a provider-bearing entry.
    pub fn provider_mut(&mut self) -> Option<&mut Provider> {
        match self {
            Structure::Built { keyed, .. } | Structure::Host { keyed, .. } => {
                Some(&mut keyed.provider)
            }
            Structure::Selection { provider, .. }
            | Structure::KeySet { provider, .. }
            | Structure::HeadDedup { provider, .. }
            | Structure::Rows { provider, .. }
            | Structure::Group { provider, .. } => Some(provider),
            Structure::JoinFilter { filter, .. } => Some(&mut filter.provider),
            Structure::Share { .. }
            | Structure::Repr { .. }
            | Structure::Stage { .. }
            | Structure::Read { .. }
            | Structure::Native { .. }
            | Structure::Dedup { .. } => None,
        }
    }
}

impl Structures {
    /// Intern opaque logical and codec spellings once at the input declaration boundary.
    /// Explicit identities survive printing and distinguish otherwise identical
    /// spellings. Unnumbered declarations of one spelling share one identity.
    /// Returns a parse error when a fresh identity would exceed `u32::MAX`.
    pub fn intern_codecs(&mut self) -> syn::Result<()> {
        self.intern_input_declarations(&[])
    }

    fn intern_input_declarations(
        &mut self,
        declarations: &[cq_syntax::RelationDecl],
    ) -> syn::Result<()> {
        fn collect<'a>(
            provider: &'a mut Provider,
            columns: &mut Vec<&'a mut provider::InputColumn>,
        ) {
            if let Some(input) = &mut provider.input {
                columns.extend(&mut input.columns);
            }
            if let Some(inner) = &mut provider.inner {
                collect(inner, columns);
            }
        }
        let mut columns = Vec::new();
        for entry in &mut self.entries {
            if let Some(provider) = entry.structure.provider_mut() {
                collect(provider, &mut columns);
            }
        }
        provider::intern_declared_logical_types(
            columns.iter_mut().map(|column| &mut **column),
            declarations,
        )?;
        provider::intern_codecs(columns)
    }

    /// Declare `declared` tokens that entries reference and drop every
    /// unreferenced declaration, keeping table order (STR-Provider).
    /// Interns input codecs and reports exhausted identity space to the caller.
    pub fn declare(&mut self, declared: &[Declaration]) -> syn::Result<()> {
        for declaration in declared {
            self.providers.declare(declaration);
        }
        let referenced: BTreeSet<String> = self
            .entries
            .iter()
            .filter_map(|entry| entry.structure.provider())
            .flat_map(|(provider, _)| provider.tokens().map(ToString::to_string))
            .collect();
        self.providers
            .declarations
            .retain(|declaration| referenced.contains(&declaration.token.to_string()));
        self.intern_codecs()
    }
}

/// Storage annotations over a separately supplied Physical program.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub struct Program {
    pub structures: Structures,
    /// Immutable original value demand bound by the selection stage. This is a
    /// consumer obligation, never permission to remove the returned set boundary.
    pub consumer: Option<OutputConsumer>,
    /// Optional encoded facts travel with the program and are always checked.
    /// The empty contract emits no tokens and changes no default selection.
    pub encoded: encoded::Contract,
}

impl Program {
    pub fn parse(input: syn::parse::ParseStream<'_>, physical: &physical::Program) -> syn::Result<Self> {
        let structures = Structures::parse_declared(input, &physical.program.declarations)?;
        let consumer = OutputConsumer::parse_optional(input)?;
        let encoded = encoded::Contract::parse_optional(input)?;
        Ok(Self {
            structures,
            consumer,
            encoded,
        })
    }
}

impl Program {
    fn check_consumer(physical: &physical::Program, program: &Self, facts: &Analysis) -> syn::Result<()> {
        let Some(consumer) = &program.consumer else {
            return Ok(());
        };
        consumer.check()?;
        let body = &physical.program;
        let head: Vec<_> = body.head.variables().map(symbol_name).collect();
        if head.len() != body.head.args.len() || consumer.head != head {
            return error(program, "STR-Consumer: original head identities must match the emitted head exactly");
        }
        let returned = &facts.physical.rows[&symbol_name(&body.returned)];
        if consumer.head.iter().any(|name| {
            !returned.bindings.contains_key(name) && !body.captures.contains(name)
        }) {
            return error(program, "STR-Consumer: head fields must name returned values or original captures");
        }
        Ok(())
    }
}

pub mod contract {
    /// Formal rule STR-Entries: Check Physical and the concrete entry obligations before returning analysis.
    pub fn check(physical: &super::physical::Program, program: &super::Program) -> syn::Result<super::Analysis> {
        let facts = super::analyze(physical, program)?;
        super::Program::check_consumer(physical, program, &facts)?;
        super::encoded::check(physical, program, &facts)?;
        Ok(facts)
    }
}

use physical::{ModeKind, Operator, syntax::SupplyKind};

/// A built structure: the sorted columns of each level, and its provider.
pub type Built = (Vec<Vec<String>>, Provider);

/// Checked access to a Physical provider, independent of its read capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadAccess {
    /// Enumerate the retained whole relation rather than a keyed index.
    Whole,
    /// Read the recorded keyed provider.
    Keyed,
}

/// What the contract establishes for later stages.
#[derive(Clone, Debug)]
pub struct Analysis {
    /// Explicitly selected immutable statistic publications (STR-BuildStats).
    pub publications: BTreeMap<String, stats::Publication>,
    /// Checked by-need lookup decisions, keyed by the first semijoin result.
    pub shares: BTreeMap<String, Share>,
    /// Non-column cursor carriers attached to result representations.
    pub witnesses: BTreeMap<String, Witness>,
    /// Probe boundaries with length-known range expansion.
    pub expands: BTreeSet<String>,
    /// Explicit result representations; omitted results keep heading tuples.
    pub repr: BTreeMap<String, Repr>,
    /// Checked batch sizes; omitted boundaries keep scalar evaluation.
    pub stages: BTreeMap<String, usize>,
    /// Independently derived single-base occurrence lineage.
    pub lineage: BTreeMap<String, Lineage>,
    /// Checked base occurrence and complete logical-to-host column mappings.
    pub selections: BTreeMap<String, Selection>,
    /// Explicit key-only builds; they confer neither payload nor ordered reads.
    pub keysets: BTreeMap<(String, Vec<String>), KeySet>,
    /// Positional access decisions attached to existing Physical readers.
    pub reads: BTreeMap<String, Read>,
    /// Checked whole-relation versus keyed access, by provider and sorted key.
    pub read_accesses: BTreeMap<String, BTreeMap<Vec<String>, ReadAccess>>,
    /// Distinct runtime filters per scan occurrence, in first-witness order.
    /// Every raw decision's exact covering witness is checked before coalescing.
    pub join_filters: BTreeMap<String, Vec<JoinFilter>>,
    pub physical: physical::Analysis,
    /// The program's checked token declarations (STR-Provider).
    pub providers: Providers,
    /// Declared column order of every host relation.
    pub host_layouts: BTreeMap<String, Vec<String>>,
    /// Built structures per result: (sorted columns of each level, provider
    /// whose spelling carries the recorded item layout, STR-ItemLayout).
    pub built: BTreeMap<String, Vec<Built>>,
    /// Exact fallback per host rename, typed reader role/identity, and key.
    pub fallbacks: BTreeMap<(String, ReaderRef<String>, Vec<String>), Provider>,
    /// Declared native host reads per host rename, reader and sorted key
    /// (STR-NativeHost); such reads build no fallback.
    pub natives: BTreeMap<(String, ReaderRef<String>, Vec<String>), Native>,
    /// Concrete accumulator tables for keyed Group results.
    pub groups: BTreeMap<String, Provider>,
    built_support: BTreeMap<(String, Vec<Vec<String>>), ProviderSupport>,
    host_support: BTreeMap<(String, ReaderRef<String>, Vec<String>), ProviderSupport>,
    /// The canonical reference of every `canon` result (index selection).
    pub canonical: BTreeMap<String, CanonSource>,
    /// Provider of the answer relation's identity index for a computed head.
    pub head_dedup: BTreeMap<String, Provider>,
    pub retained: BTreeMap<String, Provider>,
    /// Explicit projections and constant bindings of index-only selections.
    pub filtered: BTreeMap<String, Filtered>,
    /// Full-key probes certified to observe existence only, not witness payloads.
    pub membership: BTreeSet<(String, Vec<String>)>,
    /// Emitted row layouts, derived from the recorded filtered projections.
    pub layouts: BTreeMap<String, Vec<String>>,
    /// Duplicate-free results and their keys from the shared Physical analysis.
    /// Recorded canon boundaries contribute their full heading when needed.
    pub unique: BTreeMap<String, Vec<String>>,
}

impl Analysis {
    /// Declared facts of a recorded instance (STR-Provider, STR-Routed).
    pub fn facts(&self, provider: &Provider) -> Facts {
        self.providers
            .facts(provider)
            .expect("STR-Provider: every recorded token is declared")
    }

    /// A pipe host rename is read in place: natively where recorded, else
    /// through its recorded fallback.
    pub fn host_read(&self, result: &str) -> bool {
        self.physical.hosts.contains_key(result) && self.physical.modes[result] == ModeKind::Pipe
    }

    /// The structure answering a read of `result` on `columns`, and how
    /// many of its levels the read applies.
    pub fn built_on(&self, result: &str, columns: &[String]) -> Option<(&Provider, usize)> {
        self.selected_built(result, columns)
            .map(|((_, provider), depth)| (provider, depth))
    }

    fn selected_built(&self, result: &str, columns: &[String]) -> Option<(&Built, usize)> {
        let mut wanted = columns.to_vec();
        wanted.sort();
        self.built
            .get(result)?
            .iter()
            .filter_map(|built| prefix_depth(&built.0, &wanted).map(|depth| (built, depth)))
            // Complete keys precede candidate prefixes; ties preserve entry order.
            .min_by_key(|((levels, _), depth)| (levels.len() != *depth, levels.len()))
    }

    /// Reads of `result` that enumerate its rows: every read except nonempty
    /// stream-keyed reads, which a reusable structure answers. Const and range
    /// supplies may scan and filter stored rows; empty keys scan all rows.
    pub fn scan_reads(&self, result: &str) -> usize {
        let keyed = self.physical.keyed_reads.get(result).map_or(0, |reads| {
            reads
                .iter()
                .filter(|(_, columns, supply)| !columns.is_empty() && *supply == SupplyKind::Stream)
                .count()
        });
        self.physical
            .readers
            .get(result)
            .map_or(0, |reads| reads - keyed)
    }

    /// Formal rule STR-RowsByValue: a stored `let` result's rows record is
    /// held for a replaying empty-key stream read or more than one consumer
    /// (scan reads plus Built entries). Otherwise its build is bound by value
    /// and moved into that single consumer: one scan reader, or
    /// the one Built entry. Filtered builds replay their direct source, so an
    /// unscanned filtered result never holds rows. Selections and keysets keep
    /// their own sources.
    pub fn rows_held(&self, result: &str) -> bool {
        let scans = self.scan_reads(result);
        self.physical.modes.get(result) != Some(&ModeKind::Let)
            || self.selections.contains_key(result)
            || self.physical.keyed_reads.get(result).is_some_and(|reads| {
                reads
                    .iter()
                    .any(|(_, columns, supply)| columns.is_empty() && *supply == SupplyKind::Stream)
            })
            || self.keysets.keys().any(|(name, _)| name == result)
            || scans + self.built.get(result).map_or(0, Vec::len) > 1
                && !(scans == 0 && self.filtered.contains_key(result))
    }

    /// STR-RowsByValue: a `let` result whose rows record is not held and has
    /// no Built entry is bound by value and moved into its single scan reader.
    pub fn rows_by_value(&self, result: &str) -> bool {
        self.physical.modes.get(result) == Some(&ModeKind::Let)
            && self.retained.contains_key(result)
            && !self.rows_held(result)
            && self.built.get(result).is_none_or(Vec::is_empty)
    }

    fn selected_candidate(&self, result: &str, prefix: &[String], column: &str) -> Option<&Built> {
        let mut prefix = prefix.to_vec();
        prefix.sort();
        self.built.get(result)?.iter().find(|(levels, _)| {
            levels.len() == 2
                && levels[0] == prefix
                && levels[1].len() == 1
                && levels[1][0] == column
        })
    }

    /// Candidate enumeration selects its exact two-level directory, not the
    /// complete payload layout that ordinary `built_on` prefers on the prefix.
    pub fn candidate_on(&self, result: &str, prefix: &[String], column: &str) -> Option<&Provider> {
        self.selected_candidate(result, prefix, column)
            .map(|(_, provider)| provider)
    }

    /// Declared support of the directory selected by a Smallest candidate read.
    pub fn candidate_support(
        &self,
        result: &str,
        prefix: &[String],
        column: &str,
        capability: ProviderCapability,
    ) -> bool {
        self.selected_candidate(result, prefix, column)
            .and_then(|(levels, _)| self.built_support.get(&(result.to_owned(), levels.clone())))
            .is_some_and(|support| support.supports(capability))
    }

    /// Declared support of the exact provider selected for this read.
    pub fn provider_support(
        &self,
        result: &str,
        columns: &[String],
        reader: Option<&str>,
        capability: ProviderCapability,
    ) -> bool {
        let support = if let Some(reader) = reader {
            let mut columns = columns.to_vec();
            columns.sort();
            self.host_support.get(&(
                result.to_owned(),
                ReaderRef::Result(reader.to_owned()),
                columns,
            ))
        } else {
            self.selected_built(result, columns)
                .and_then(|((levels, _), _)| {
                    self.built_support.get(&(result.to_owned(), levels.clone()))
                })
        };
        support.is_some_and(|support| support.supports(capability))
    }
}

/// The number of leading levels whose union is exactly `wanted`, if any.
/// Formal rule STR-E-Built: A built read key is the union of a prefix of its distinct heading levels.
pub fn prefix_depth(levels: &[Vec<String>], wanted: &[String]) -> Option<usize> {
    let mut union: Vec<String> = Vec::new();
    for (depth, level) in levels.iter().enumerate() {
        union.extend(level.iter().cloned());
        union.sort();
        if union == wanted {
            return Some(depth + 1);
        }
    }
    None
}

fn error<T>(spanned: &impl quote::ToTokens, message: impl std::fmt::Display) -> syn::Result<T> {
    Err(syn::Error::new_spanned(
        spanned,
        format!("{}: {message}", spanned.to_token_stream()),
    ))
}

/// Formal rule WF-Structures: Check entries and read coverage over checked Physical.
/// Formal rule STR-Key: Answer probe and semi reads by host dispatch or a built level prefix.
/// Formal rule STR-Merge: Answer each merge side by ordered storage or a same-key merge pipe.
/// Formal rule CK-Return: Physical checks the returned set boundary, including canonical consumers.
fn analyze(physical: &crate::physical::Program, program: &Program) -> syn::Result<Analysis> {
    let module = &program.structures;
    provider::check(module)?;
    let checked_physical = physical::contract::check(physical)?;
    let unique = physical::uniqueness(physical, &checked_physical.headings, &checked_physical.disjoint, |definition, _| {
        definition.canon.is_some()
    });
    let mut analysis = Analysis {
        physical: checked_physical,
        providers: module.providers.clone(),
        publications: BTreeMap::new(),
        host_layouts: physical
            .inputs()
            .map(|r| {
                (
                    symbol_name(&r.name),
                    r.columns.iter().map(|c| symbol_name(&c.name)).collect(),
                )
            })
            .collect(),
        shares: BTreeMap::new(),
        witnesses: BTreeMap::new(),
        expands: BTreeSet::new(),
        repr: BTreeMap::new(),
        stages: BTreeMap::new(),
        lineage: BTreeMap::new(),
        selections: BTreeMap::new(),
        keysets: BTreeMap::new(),
        reads: BTreeMap::new(),
        read_accesses: BTreeMap::new(),
        join_filters: BTreeMap::new(),
        built: BTreeMap::new(),
        fallbacks: BTreeMap::new(),
        natives: BTreeMap::new(),
        groups: BTreeMap::new(),
        built_support: BTreeMap::new(),
        host_support: BTreeMap::new(),
        canonical: BTreeMap::new(),
        head_dedup: BTreeMap::new(),
        retained: BTreeMap::new(),
        filtered: BTreeMap::new(),
        membership: BTreeSet::new(),
        layouts: BTreeMap::new(),
        unique,
    };
    analysis.lineage = lineages(physical, &analysis.physical);
    let candidates = physical::candidate_prefixes(physical, &analysis.physical);
    let mut seen = BTreeSet::new();
    let mut references: BTreeMap<String, (&Entry, &CanonIndex)> = BTreeMap::new();
    for entry in &module.entries {
        let name = symbol_name(&entry.name);
        let Some(heading) = analysis.physical.headings.get(&name).cloned() else {
            return error(&entry.name, "structure must name a Physical result");
        };
        if positional::entry(program, entry, &mut analysis)? {
            continue;
        }
        if native::entry(physical, program, entry, &mut analysis)? {
            continue;
        }
        let keyed = match &entry.structure {
            Structure::Repr { .. } | Structure::Stage { .. } | Structure::Share { .. } => continue,
            Structure::JoinFilter { .. }
            | Structure::Selection { .. }
            | Structure::KeySet { .. }
            | Structure::Read { .. }
            | Structure::Native { .. } => {
                unreachable!()
            }
            Structure::Built { keyed, .. } => keyed,
            Structure::Host { reader, keyed, .. } => {
                // Formal rule STR-E-Fallback: Validate tagged fallback roles and their Physical read keys.
                if matches!(reader, ReaderRef::Scan) && keyed.supports.is_some() {
                    return error(
                        keyed,
                        "provider support belongs to keyed reads, not scan adapters",
                    );
                }
                if keyed.filtered.is_some() {
                    return error(keyed, "a host fallback cannot retain a filtered projection");
                }
                if !(analysis.host_read(&name)
                    || matches!(reader, ReaderRef::Scan)
                        && analysis.physical.hosts.contains_key(&name))
                {
                    return error(
                        &entry.name,
                        "fallback requires a reusable host rename or initial host scan",
                    );
                }
                if matches!(reader, ReaderRef::Scan)
                    && let Some(input) = &keyed.provider.input
                {
                    let Some(host) = analysis.physical.hosts.get(&name) else {
                        return error(keyed, "GPU-NativeAbi: input schema has no host origin");
                    };
                    let source = physical
                        .inputs()
                        .find(|relation| symbol_name(&relation.name) == *host)
                        .expect("checked host input");
                    if input.columns.len() != source.columns.len()
                        || input
                            .columns
                            .iter()
                            .zip(&source.columns)
                            .any(|(recorded, column)| {
                                recorded.logical_type_id != column.logical_type_id
                            })
                    {
                        return error(
                            keyed,
                            "GPU-NativeAbi: recorded input schema differs from the relation",
                        );
                    }
                }
                keyed.check_support()?;
                if let Some(items) = &keyed.items {
                    return error(items, "an item layout belongs to a built payload provider");
                }
                let columns = keyed.level_names();
                if columns.is_empty() || (columns.len() > 1 && columns.iter().any(Vec::is_empty)) {
                    return error(keyed, "host fallback requires nonempty key levels");
                }
                let mut read_columns = columns.concat();
                read_columns.sort();
                let mut unique = BTreeSet::new();
                for column in keyed.columns() {
                    if !heading.contains(&symbol_name(column)) {
                        return error(column, format!("`{name}` has no column `{column}`"));
                    }
                    if !unique.insert(symbol_name(column)) {
                        return error(column, "host fallback repeats a key column");
                    }
                }
                let reader_name = reader.key();
                let valid = match &reader_name {
                    ReaderRef::Scan => columns.len() == 1 && read_columns.is_empty(),
                    ReaderRef::Candidates | ReaderRef::Membership => {
                        candidates.iter().flat_map(|c| &c.sides).any(|(n, p)| {
                            *n == name
                                && read_columns
                                    == if matches!(reader, ReaderRef::Candidates) {
                                        p.clone()
                                    } else {
                                        heading.clone()
                                    }
                        })
                    }
                    ReaderRef::Result(result) => analysis
                        .physical
                        .keyed_reads
                        .get(&name)
                        .into_iter()
                        .flatten()
                        .any(|(r, c, _)| r == result && *c == read_columns),
                };
                if !valid {
                    return error(reader, "fallback does not match a host read");
                }
                if keyed.membership.is_some() {
                    if columns.len() != 1
                        || read_columns != heading
                        || !matches!(reader, ReaderRef::Result(_))
                        || analysis
                            .physical
                            .keyed_reads
                            .get(&name)
                            .into_iter()
                            .flatten()
                            .any(|(_, key, supply)| {
                                key != &read_columns || *supply != SupplyKind::Stream
                            })
                        || analysis
                            .physical
                            .merge_reads
                            .get(&name)
                            .is_some_and(|reads| !reads.is_empty())
                    {
                        return error(
                            keyed,
                            "host membership requires one complete row key and no ordered reads",
                        );
                    }
                    analysis
                        .membership
                        .insert((name.clone(), read_columns.clone()));
                }
                if let Some(support) = &keyed.supports {
                    analysis.host_support.insert(
                        (name.clone(), reader_name.clone(), read_columns.clone()),
                        support.clone(),
                    );
                }
                // STR-ReaderChoice: the served reader's recorded Physical
                // choice prints the provider's declared choice argument.
                let provider = match analysis.facts(&keyed.provider).choice {
                    Some(slot) => {
                        let lazy = physical.program.definitions.iter().any(|d| {
                            matches!(&reader_name, ReaderRef::Result(r) if symbol_name(&d.name) == *r)
                                && matches!(&d.operator, Operator::Semi(_, k) | Operator::Anti(_, k)
                                    if matches!(k.choice, Some(physical::Choice::Lazy(_))))
                        });
                        slot.print(&keyed.provider, lazy)
                    }
                    None => keyed.provider.clone(),
                };
                if analysis
                    .fallbacks
                    .insert((name.clone(), reader_name, read_columns), provider)
                    .is_some()
                {
                    return error(entry, "duplicate host fallback");
                }
                continue;
            }
            Structure::Group { provider, .. } => {
                check_group(physical, program, entry)?;
                if analysis.groups.insert(name, provider.clone()).is_some() {
                    return error(entry, "duplicate group accumulator type");
                }
                continue;
            }
            Structure::Dedup { index, .. } => {
                if !analysis.physical.canon.contains(&name) {
                    return error(entry, "a canonical reference requires a canon boundary");
                }
                if references.insert(name, (entry, index)).is_some() {
                    return error(entry, "duplicate canonical reference");
                }
                continue;
            }
            Structure::HeadDedup { provider, .. } => {
                // Formal rule STR-E-Canon: Type exactly the recorded canonical and return dedup boundaries.
                // PH-HeadDedup: an answer identity index types exactly the
                // recorded Physical return-boundary obligation (PH-HeadDedup).
                if name != symbol_name(&physical.program.returned)
                    || !physical.program.head_dedup()
                {
                    return error(
                        entry,
                        "head dedup types the recorded Physical `return canon` or `return dedup` obligation",
                    );
                }
                if analysis.head_dedup.insert(name, provider.clone()).is_some() {
                    return error(entry, "duplicate answer identity index");
                }
                continue;
            }
            Structure::Rows { provider, .. } => {
                if analysis.physical.modes[&name] == ModeKind::Pipe {
                    return error(
                        entry,
                        "retained rows require an enumerated let or exists result",
                    );
                }
                if analysis.retained.insert(name, provider.clone()).is_some() {
                    return error(entry, "duplicate storage decision");
                }
                continue;
            }
        };
        {
            if analysis.host_layouts.contains_key(&name) {
                return error(
                    &entry.name,
                    "a host relation is represented by its host value; build over a let result",
                );
            }
            let Some(heading) = analysis.physical.headings.get(&name) else {
                return error(&entry.name, "structure must name a Physical result");
            };
            keyed.check_support()?;
            if keyed.columns().next().is_none() {
                return error(
                    keyed,
                    "a built index requires at least one key column; use rows for a whole-relation read",
                );
            }
            let mut all = BTreeSet::new();
            for column in keyed.columns() {
                if !heading.contains(&symbol_name(column)) {
                    return error(column, format!("`{name}` has no column `{column}`"));
                }
                if !all.insert(symbol_name(column)) {
                    return error(column, format!("`{column}` appears in two levels"));
                }
            }
            if analysis.physical.modes[&name] == ModeKind::Pipe {
                return error(
                    &entry.name,
                    format!("`{name}` is a pipe; a structure is built only over a `let`"),
                );
            }
            let columns = keyed.level_names();
            let serves_stream = analysis
                .physical
                .keyed_reads
                .get(&name)
                .into_iter()
                .flatten()
                .any(|(_, read_columns, supply)| {
                    *supply == SupplyKind::Stream && prefix_depth(&columns, read_columns).is_some()
                });
            if !serves_stream {
                return error(
                    &entry.name,
                    "a built structure must serve a stream demand; const-only reads use direct access or scan and filter",
                );
            }
            if !seen.insert((name.clone(), columns.clone())) {
                return error(&entry.name, "duplicate structure on these columns");
            }
            if let Some(support) = &keyed.supports {
                analysis
                    .built_support
                    .insert((name.clone(), columns.clone()), support.clone());
            }
            // STR-ItemLayout: the recorded layout prints the declared
            // item-layout argument; the instance spelling ends before it.
            // STR-ReaderChoice: built storage never serves a lazy reader
            // (WF-Physical: lazy reads name pipe host renames), so a declared
            // choice argument prints `eager`.
            let facts = analysis.facts(&keyed.provider);
            let provider = match (&keyed.items, facts.items, facts.choice) {
                (Some(items), Some(slot), _) => slot.print(&keyed.provider, &items.layout),
                (_, _, Some(slot)) => slot.print(&keyed.provider, false),
                _ => keyed.provider.clone(),
            };
            analysis
                .built
                .entry(name.clone())
                .or_default()
                .push((columns, provider));
            if keyed.membership.is_some() {
                if keyed.levels.len() != 1 {
                    return error(keyed, "membership storage requires one complete key");
                }
                // Formal rule STR-FullKey: raw set rows can be represented by
                // existence only when every logical column is bound by the key.
                if keyed.filtered.is_none()
                    && (keyed.level_names().concat() != analysis.physical.headings[&name]
                        || analysis
                            .physical
                            .keyed_reads
                            .get(&name)
                            .into_iter()
                            .flatten()
                            .any(|(_, columns, supply)| {
                                columns != &keyed.level_names().concat()
                                    || *supply != SupplyKind::Stream
                            })
                        || analysis
                            .physical
                            .merge_reads
                            .get(&name)
                            .is_some_and(|reads| !reads.is_empty()))
                {
                    return error(
                        keyed,
                        "raw membership storage requires the complete row key and no ordered reads",
                    );
                }
                analysis
                    .membership
                    .insert((name.clone(), keyed.level_names().concat()));
            }
            if let Some(filtered) = &keyed.filtered {
                if analysis
                    .filtered
                    .get(&name)
                    .is_some_and(|previous| previous != filtered)
                {
                    return error(
                        entry,
                        "filtered indices must record an identical projection",
                    );
                }
                analysis.filtered.insert(name, filtered.clone());
            }
        }
    }
    positional::check(physical, program, &analysis)?;
    analysis.shares = share::check(physical, program, &analysis)?;
    analysis.witnesses = repr::check_witnesses(physical, program, &analysis)?;
    analysis.repr = repr::check(physical, program, &analysis)?;
    analysis.stages = stage::check(physical, program, &analysis)?;
    analysis.expands = program
        .structures
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.structure,
                Structure::Stage {
                    expand: Some(_),
                    ..
                }
            )
        })
        .map(|entry| symbol_name(&entry.name))
        .collect();
    filtered::check(physical, program, &analysis)?;
    analysis.layouts = filtered::layouts(physical, program, &analysis);
    native::check(&analysis)?;
    let mut shared: BTreeMap<(String, Vec<String>, bool), &Provider> = BTreeMap::new();
    for ((name, reader, columns), provider) in &analysis.fallbacks {
        if matches!(reader, ReaderRef::Scan)
            || physical.program.definitions.iter().any(|d| {
                matches!(reader, ReaderRef::Result(result) if symbol_name(&d.name) == *result)
                    && matches!(d.operator, Operator::Point(..) | Operator::Range(..))
            })
        {
            continue;
        }
        let ordered = physical.program.definitions.iter().any(|d| {
            matches!(reader, ReaderRef::Result(result) if symbol_name(&d.name) == *result)
                && matches!(d.operator, Operator::Merge(..))
        });
        if let Some(other) = shared.insert((name.clone(), columns.clone(), ordered), provider) {
            if other != provider {
                return error(
                    provider,
                    "shared host fallback providers on one key must agree",
                );
            }
        }
    }
    for candidate in candidates {
        let v = candidate.column;
        for (name, prefix) in candidate.sides {
            if analysis.host_read(&name) {
                for (reader, key) in [
                    (ReaderRef::Candidates, prefix.clone()),
                    (
                        ReaderRef::Membership,
                        analysis.physical.headings[&name].clone(),
                    ),
                ] {
                    let read = (name.clone(), reader, key);
                    if !analysis.fallbacks.contains_key(&read)
                        && !analysis.natives.contains_key(&read)
                    {
                        return error(candidate.reader, "missing intersection host fallback type");
                    }
                }
            } else {
                let valid = analysis.candidate_on(&name, &prefix, &v).is_some();
                if !valid {
                    return error(
                        candidate.reader,
                        "smallest requires a two-level candidate structure on each retained side",
                    );
                }
            }
        }
    }
    // Formal rule STR-Range: Const/range supplies always have direct or scan/filter reads without builds.
    // Only stream supplies require reusable storage. Const supplies always
    // have the direct-host or scan-and-filter implementation available.
    let answered = |result: &str, columns: &[String]| -> bool {
        analysis.host_read(result)
            || columns.is_empty()
                && (analysis.retained.contains_key(result)
                    || analysis.selections.contains_key(result))
            || analysis.built_on(result, columns).is_some()
    };
    for (result, reads) in &analysis.physical.keyed_reads {
        for (reader, columns, supply) in reads {
            if *supply != SupplyKind::Stream
                || analysis
                    .physical
                    .merge_reads
                    .get(result)
                    .is_some_and(|merge| {
                        merge.iter().any(|(merge_reader, _)| merge_reader == reader)
                    })
            {
                continue;
            }
            let membership = physical.program.definitions.iter().any(|d| {
                symbol_name(&d.name) == *reader
                    && matches!(d.operator, Operator::Semi(..) | Operator::Anti(..))
            });
            if !(answered(result, columns)
                || membership
                    && analysis
                        .keysets
                        .contains_key(&(result.clone(), columns.clone())))
            {
                return error(
                    &module.structures_token,
                    format!(
                        "`{reader}` reads `{result}` on {columns:?}; no reusable host read or built structure answers it"
                    ),
                );
            }
            if let Some((ty, depth)) = analysis.built_on(result, columns) {
                let payload = physical.program.definitions.iter().any(|d| {
                    symbol_name(&d.name) == *reader && matches!(d.operator, Operator::Probe(..))
                });
                if payload {
                    let ((levels, _), _) = analysis
                        .selected_built(result, columns)
                        .expect("selected build");
                    if depth != levels.len() && !physical.program.definitions.iter().any(|d| symbol_name(&d.name) == *reader && matches!(&d.operator, Operator::Probe(_,k) if matches!(k.choice, Some(physical::Choice::Smallest(_))))) {
                        return error(ty, "a payload probe must read all built levels");
                    }
                }
            }
        }
    }
    for (result, reads) in &analysis.physical.keyed_reads {
        for (_, columns, _) in reads {
            let access = if columns.is_empty()
                && (analysis.host_read(result)
                    || analysis.retained.contains_key(result)
                    || analysis.selections.contains_key(result))
            {
                ReadAccess::Whole
            } else {
                ReadAccess::Keyed
            };
            analysis
                .read_accesses
                .entry(result.clone())
                .or_default()
                .insert(columns.clone(), access);
        }
    }
    // Merge sides: ordered on the key by Γ, or a pipe sorted on it by its own merge.
    let merge_key: BTreeMap<String, Vec<String>> = physical
        .program
        .definitions
        .iter()
        .filter_map(|d| match &d.operator {
            Operator::Merge(_, k) => Some((symbol_name(&d.name), names(&k.columns))),
            _ => None,
        })
        .collect();
    for (side, reads) in &analysis.physical.merge_reads {
        for (reader, columns) in reads {
            let by_structure =
                analysis.host_read(side) || analysis.built_on(side, columns).is_some();
            let by_stream = analysis.physical.modes[side] == ModeKind::Pipe
                && merge_key.get(side) == Some(columns);
            if !by_structure && !by_stream {
                return error(
                    &module.structures_token,
                    format!(
                        "`{reader}` merges `{side}` on {columns:?}; it is neither ordered on that key nor a pipe sorted on it"
                    ),
                );
            }
        }
    }

    // Uniqueness by key inference, then the canonical return.
    for definition in &physical.program.definitions {
        let name = symbol_name(&definition.name);
        if matches!(&definition.operator, Operator::Group(group) if !group.keys.is_empty())
            && !analysis.groups.contains_key(&name)
        {
            return error(&definition.name, "missing group accumulator type");
        }
        if definition.canon.is_some() && !references.contains_key(&name) {
            return error(&definition.name, "missing canonical reference");
        }
        // Formal rule STR-Canon: the reference names one complete ordinary
        // index of the result. Its rows record is a complete-row provider; an
        // existing Built entry qualifies only when every read of the result
        // observes existence alone, so its build need not be duplicate-free;
        // a host carrier must be the complete direct load-verified scan.
        if let Some((entry, index)) = references.get(&name) {
            let mode = analysis.physical.modes[&name];
            let source = match index {
                CanonIndex::Rows(_) => match analysis.retained.get(&name) {
                    Some(provider) if mode != ModeKind::Pipe => CanonSource::Rows(provider.clone()),
                    _ => {
                        return error(
                            entry,
                            "a canonical rows reference requires the stored result's rows record",
                        );
                    }
                },
                CanonIndex::Built(level) => {
                    let columns = names(&level.columns);
                    let mut heading = analysis.physical.headings[&name].clone();
                    heading.sort();
                    let existence = analysis
                        .physical
                        .keyed_reads
                        .get(&name)
                        .into_iter()
                        .flatten()
                        .all(|(reader, read, _)| {
                            *read == columns
                                && (analysis
                                    .membership
                                    .contains(&(name.clone(), columns.clone()))
                                    || physical.program.definitions.iter().any(|d| {
                                        symbol_name(&d.name) == *reader
                                            && matches!(
                                                d.operator,
                                                Operator::Semi(..) | Operator::Anti(..)
                                            )
                                    }))
                        });
                    if columns != heading
                        || mode != ModeKind::Let
                        || analysis
                            .built
                            .get(&name)
                            .is_none_or(|built| built.len() != 1 || built[0].0 != [columns.clone()])
                        || !existence
                        || analysis.physical.scanned.contains(&name)
                        || analysis.retained.contains_key(&name)
                        || analysis.selections.contains_key(&name)
                        || analysis
                            .physical
                            .merge_reads
                            .get(&name)
                            .is_some_and(|reads| !reads.is_empty())
                    {
                        return error(
                            entry,
                            "a canonical Built index must be the result's only storage: one complete-row level read only for existence",
                        );
                    }
                    CanonSource::Index(columns)
                }
                CanonIndex::Host(_) => {
                    let verified_source = match &definition.operator {
                        Operator::Rename(rename) if rename.constants.is_none() => {
                            analysis.physical.hosts.get(&name).is_some_and(|host| {
                                analysis.physical.renames[&name]
                                    .values()
                                    .collect::<BTreeSet<_>>()
                                    == analysis.host_layouts[host].iter().collect::<BTreeSet<_>>()
                            })
                        }
                        _ => false,
                    };
                    if !verified_source
                        || mode != ModeKind::Pipe
                        || analysis.selections.contains_key(&name)
                        || analysis.join_filters.contains_key(&name)
                    {
                        return error(
                            &definition.name,
                            "a canonical host carrier requires a complete direct load-verified host scan",
                        );
                    }
                    CanonSource::Host
                }
            };
            analysis.canonical.insert(name.clone(), source);
        }
        // Formal rule STR-E-Rows: Require reusable rows and host adapters for the recorded Physical demands.
        if (analysis.physical.scanned.contains(&name)
            || analysis
                .built
                .get(&name)
                .is_some_and(|built| built.len() > 1 && !analysis.filtered.contains_key(&name))
            || analysis.physical.modes[&name] == ModeKind::Exists)
            && analysis.physical.modes[&name] != ModeKind::Pipe
            && !analysis.retained.contains_key(&name)
            && !analysis.selections.contains_key(&name)
        {
            return error(&definition.name, "missing retained rows type");
        }
        if analysis.physical.hosts.contains_key(&name) {
            if !analysis
                .fallbacks
                .contains_key(&(name.clone(), ReaderRef::Scan, Vec::new()))
            {
                return error(&definition.name, "missing host scan type");
            }
            for (reader, columns, _) in analysis
                .physical
                .keyed_reads
                .get(&name)
                .into_iter()
                .flatten()
            {
                if columns.is_empty()
                    || !analysis.host_read(&name)
                    || analysis.reads.contains_key(reader)
                {
                    continue;
                }
                let read = (
                    name.clone(),
                    ReaderRef::Result(reader.clone()),
                    columns.clone(),
                );
                if !analysis.fallbacks.contains_key(&read) && !analysis.natives.contains_key(&read)
                {
                    return error(
                        &definition.name,
                        format!("missing host fallback for `{reader}`"),
                    );
                }
            }
        }
    }
    let returned = symbol_name(&physical.program.returned);
    if physical.program.head_dedup() && !analysis.head_dedup.contains_key(&returned) {
        return error(
            &physical.program.returned,
            "missing answer identity index",
        );
    }
    coalesce_join_filters(physical, program, &mut analysis);
    check_items(physical, program, &analysis)?;
    stats::check(program, &mut analysis)?;
    Ok(analysis)
}

/// Formal rule STR-ItemLayout: an item layout belongs to a one-level payload
/// build; a column layout splits a payload of at least two fields. The payload
/// is the filtered schema, else the carried columns, without the key.
fn check_items(physical: &crate::physical::Program, program: &Program, analysis: &Analysis) -> syn::Result<()> {
    let mut carried = None;
    for entry in &program.structures.entries {
        let Structure::Built { keyed, .. } = &entry.structure else {
            continue;
        };
        let Some(items) = &keyed.items else {
            continue;
        };
        if keyed.levels.len() != 1
            || keyed.levels[0].columns.is_empty()
            || keyed.membership.is_some()
        {
            return error(
                items,
                "an item layout needs one nonempty key level and a payload",
            );
        }
        if matches!(items.layout, ItemLayout::Columns(_)) {
            let name = symbol_name(&entry.name);
            let key = keyed.level_names().concat();
            let payload = match analysis.filtered.get(&name) {
                Some(filtered) => filtered.payload.columns.iter().map(symbol_name).collect(),
                None => carried.get_or_insert_with(|| demand::carried_columns(physical, program, analysis))
                    [&name]
                    .clone(),
            };
            if payload
                .iter()
                .filter(|column| !key.contains(column))
                .count()
                < 2
            {
                return error(
                    items,
                    "a column layout splits a payload of at least two fields",
                );
            }
        }
    }
    Ok(())
}

/// Formal rule STR-Group: Exactly keyed Group results record a concrete accumulator table.
fn check_group(physical: &crate::physical::Program, _program: &Program, entry: &Entry) -> syn::Result<()> {
    if !physical
        .program
        .definitions
        .iter()
        .any(|definition| {
            definition.name == entry.name
                && matches!(&definition.operator, Operator::Group(group) if !group.keys.is_empty())
        })
    {
        return error(entry, "group accumulator requires a keyed Group result");
    }
    Ok(())
}

/// Shared syntax used by this language.
pub use cq_syntax as syntax;

/// Concrete representation names may contain inference parameters, never macro slots.
pub fn concrete_type(ty: &syn::Type) -> syn::Result<()> {
    struct Macros(bool);
    impl<'ast> syn::visit::Visit<'ast> for Macros {
        fn visit_type_macro(&mut self, _: &'ast syn::TypeMacro) {
            self.0 = true;
        }
    }
    let mut macros = Macros(false);
    syn::visit::Visit::visit_type(&mut macros, ty);
    if macros.0 || matches!(ty, syn::Type::Infer(_)) {
        return error(ty, "record a concrete Rust representation type");
    }
    Ok(())
}

/// Structures rewrites preserve Physical and discharge every read and boundary.
pub fn check_rewrite(source_physical: &physical::Program, source: &Program, target_physical: &physical::Program, target: &Program) -> syn::Result<()> {
    contract::check(source_physical, source)?;
    contract::check(target_physical, target)?;
    physical::check_rewrite(source_physical, target_physical)?;
    if source.consumer != target.consumer {
        return error(target, "Structures rewrite must preserve original consumer obligations");
    }
    if source.encoded.obligations != target.encoded.obligations
        || source.encoded.blocks.sources != target.encoded.blocks.sources
    {
        return error(target, "Structures rewrite must preserve original encoded source obligations");
    }
    if source_physical != target_physical
        || quote::ToTokens::to_token_stream(source_physical).to_string()
            != quote::ToTokens::to_token_stream(target_physical).to_string()
    {
        return error(
            target_physical,
            "Structures rewrite must preserve Physical AST and tokens exactly",
        );
    }
    Ok(())
}

/// Compare semantic layout while allowing the storage type and its declared support to change.
/// Independent checking still proves live support obligations. Host readers and
/// all non-storage syntax remain exact, including opaque tokens.
pub fn storage_layout_eq(left: &Entry, right: &Entry) -> bool {
    if left.name != right.name
        || std::mem::discriminant(&left.structure) != std::mem::discriminant(&right.structure)
    {
        return false;
    }
    let mut normalized = left.clone();
    match (&mut normalized.structure, &right.structure) {
        (Structure::Built { keyed: left, .. }, Structure::Built { keyed: right, .. }) => {
            left.provider = right.provider.clone();
            left.supports = right.supports.clone();
            left.items = right.items.clone();
        }
        (Structure::Host { .. }, _) => {}
        (left, right) => {
            if let (Some(left), Some((right, _))) = (left.provider_mut(), right.provider()) {
                *left = right.clone();
            }
        }
    }
    normalized == *right
        && normalized.to_token_stream().to_string() == right.to_token_stream().to_string()
}

/// Check catalog retention across the Physical/Structures boundary.
///
/// Unlike an ordinary Structures rewrite, this permits checked host renames
/// to move from `pipe` to `let`, replacing keyed fallbacks with built indexes.
/// Operators, their order, opaque syntax, scan adapters and existing layouts
/// remain unchanged. Storage implementations may be retyped.
pub fn check_retention_rewrite(source_physical: &physical::Program, source: &Program, target_physical: &physical::Program, target: &Program) -> syn::Result<()> {
    contract::check(source_physical, source)?;
    let facts = contract::check(target_physical, target)?;
    if source.consumer != target.consumer {
        return error(target, "catalog retention must preserve original consumer obligations");
    }
    if source.encoded.obligations != target.encoded.obligations
        || source.encoded.blocks.sources != target.encoded.blocks.sources
    {
        return error(target, "catalog retention must preserve original encoded source obligations");
    }
    let mut normalized = target_physical.clone();
    if source_physical.program.definitions.len() != normalized.program.definitions.len() {
        return error(
            target,
            "catalog retention cannot add or remove Physical definitions",
        );
    }
    for (old, new) in source_physical
        .program
        .definitions
        .iter()
        .zip(&mut normalized.program.definitions)
    {
        if old.mode != new.mode {
            if !matches!(
                (&old.mode, &new.mode, &old.operator),
                (
                    physical::Mode::Pipe(_),
                    physical::Mode::Let(_),
                    physical::Operator::Rename(_)
                )
            ) {
                return error(
                    new,
                    "catalog retention only permits a host rename from pipe to let",
                );
            }
            new.mode = old.mode.clone();
        }
    }
    if normalized != *source_physical
        || normalized.to_token_stream().to_string() != source_physical.to_token_stream().to_string()
    {
        return error(
            target,
            "catalog retention must preserve all non-mode Physical syntax exactly",
        );
    }
    physical::check_rewrite(source_physical, target_physical)?;
    let same_entry = storage_layout_eq;
    for entry in &source.structures.entries {
        if target
            .structures
            .entries
            .iter()
            .any(|other| same_entry(entry, other))
        {
            continue;
        }
        let replaced = match &entry.structure {
            Structure::Host { reader, keyed, .. } if !matches!(reader, ReaderRef::Scan) => {
                Some(keyed.level_names().concat())
            }
            Structure::Native { reader, read, .. } if !matches!(reader, ReaderRef::Scan) => {
                Some(read.key.columns.iter().map(symbol_name).collect())
            }
            _ => None,
        };
        if let Some(mut keys) = replaced
            && facts.physical.modes.get(&symbol_name(&entry.name)) == Some(&physical::ModeKind::Let)
        {
            keys.sort();
            if facts.built_on(&symbol_name(&entry.name), &keys).is_some() {
                continue;
            }
        }
        return error(
            entry,
            "catalog retention must preserve scans, opaque adapters and existing storage layouts",
        );
    }
    for entry in &target.structures.entries {
        if !matches!(entry.structure, Structure::Built { .. })
            && !source
                .structures
                .entries
                .iter()
                .any(|other| same_entry(other, entry))
        {
            return error(
                entry,
                "catalog retention may add built indexes, not new opaque adapters",
            );
        }
    }
    Ok(())
}

pub mod demand;
mod native;
pub use native::{Native, NativeRead};
pub mod positional;
pub use positional::*;
mod join_filter;
pub use join_filter::*;
mod repr;
pub use repr::Witness;
pub use repr::{Repr, ReprBase, representation_bases};
mod share;
pub use share::{RetainedHost, Share, retained_host_read};
mod stage;

mod filtered;
pub use filtered::{reconstructible_constants, required_columns, selection_constants};

impl<'a> cq_stage::Annotation<&'a physical::Program> for Program {
    fn parse(input: syn::parse::ParseStream<'_>, physical: &'a physical::Program) -> syn::Result<Self> {
        Self::parse(input, physical)
    }

    fn output_consumer(&self) -> Option<&OutputConsumer> {
        self.consumer.as_ref()
    }

    fn check(&self, physical: &'a physical::Program) -> syn::Result<()> {
        contract::check(physical, self).map(|_| ())
    }

    fn check_rewrite(source_physical: &'a physical::Program, source: &Self, target_physical: &'a physical::Program, target: &Self) -> syn::Result<()> {
        check_rewrite(source_physical, source, target_physical, target)
    }
}
