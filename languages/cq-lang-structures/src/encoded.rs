//! Encoded representation/publication contracts, carried by `Program` but absent
//! from ordinary programs. This module checks original obligations; it neither
//! selects a representation nor interprets Rust spellings. Boundary facts are
//! references imported from Sequence/GPU: this is not a second scheduler,
//! allocator, live-count protocol, or runtime metadata validator.
use std::collections::{BTreeMap, BTreeSet};

use cq_syntax::DomainId;
use proc_macro2::{Span, TokenStream};
use quote::{ToTokens, quote};
use syn::parse::ParseStream;
use syn::{Token, braced, parenthesized};

use crate::{Analysis, Program, representation_bases, symbol_name};

mod kw {
    syn::custom_keyword!(encoded);
}

macro_rules! ids {
    ($($name:ident),* $(,)?) => {$ (
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        pub struct $name(pub u64);
        impl Wire for $name {
            fn read(input: ParseStream<'_>) -> syn::Result<Self> {
                Ok(Self(u64::read(input)?))
            }
            fn emit(&self, tokens: &mut TokenStream) { self.0.emit(tokens); }
        }
    )*};
}
ids!(
    DeclarationId,
    RepresentationId,
    SourceId,
    SnapshotId,
    PublicationId,
    ReadId,
    CopyId,
    BoundaryId,
    BufferId
);

/// Only supported shared S1 families and the existing byte-addressable index
/// family are expressible. FastLanes is deliberately not a candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Plain,
    Bytes,
    IndexByteAddressable,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    S1,
    ProviderIndex,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    ScanColumn,
    IndexRun,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Carrier {
    U8,
    U16,
    U32,
    U64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Preservation {
    Equality,
    Order,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BufferKind {
    Payload,
    Headers,
    BlockSummaries,
    VectorSummaries,
    Dictionary,
    Positions,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    Host,
    Device,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReadPurpose {
    PreserveCarrier,
    OpaqueRustRead,
    FinalHostAnswer,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyReason {
    CrossDevice,
    Reorganize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dictionary {
    pub identity: String,
    pub version: u64,
}

/// `code` references the existing declaration-table identity. `None` denotes
/// the logical domain of the source column declaration, not an inferred type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Domain {
    pub code: Option<DomainId>,
    pub dictionary: Option<Dictionary>,
}

/// Binder/provider catalog facts, owned once and referenced by decisions.
/// A used declaration must name the actual source scan or publication index
/// provider; another declared token cannot lend its representation guarantees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    pub provider: String,
    pub domain: Domain,
    pub role: Role,
    pub family: Family,
    pub layout: Layout,
    pub carrier: Carrier,
    /// Semantic identity is explicit; Plain storage of dictionary codes is not
    /// an identity representation merely because it is byte-addressable.
    pub identity: bool,
    pub preserves: BTreeSet<Preservation>,
    pub buffers: BTreeSet<BufferKind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub relation: String,
    pub occurrence: String,
    pub snapshot: SnapshotId,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct RetainedSource {
    pub source: SourceId,
    pub snapshot: SnapshotId,
}

/// Symbolic initialized extent and owner are boundary-owned references, not
/// allocation capacity or a guessed compressed length.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferFacts {
    pub source: RetainedSource,
    pub column: String,
    pub kind: BufferKind,
    pub initialized_bytes: String,
    pub owner: String,
}

/// Imported boundary facts. Sequence/GPU alone validates concrete count <=
/// capacity, initialized extents, stream/event edges and owner lifetimes.
/// Publications/read packets reference this id rather than copying those facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundaryFacts {
    pub location: Location,
    pub live_count: String,
    pub capacity: String,
    pub readiness: String,
    pub retained_sources: BTreeSet<RetainedSource>,
    pub buffers: BTreeMap<BufferId, BufferFacts>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldRequirement {
    pub source: SourceId,
    pub column: String,
    pub domain: Domain,
    pub role: Role,
    pub preserves: BTreeSet<Preservation>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicationObligation {
    pub producer: String,
    pub boundary: BoundaryId,
    /// Exact demanded logical fields, with original source lineage.
    pub fields: BTreeMap<String, FieldRequirement>,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Consumer {
    Relational(String),
    OpaqueRust(String),
    FinalAnswer(String),
    Transport(CopyId),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadPacket {
    pub publication: PublicationId,
    pub boundary: BoundaryId,
    pub consumer: Consumer,
    pub purpose: ReadPurpose,
    pub fields: BTreeSet<String>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Copy {
    pub publication: PublicationId,
    pub from: BoundaryId,
    pub to: BoundaryId,
    pub reason: CopyReason,
    /// Exact initialized source and destination buffers, never whole capacity.
    pub buffers: BTreeMap<BufferId, BufferId>,
}

/// Original physical/binder/boundary obligations, never selector output. Read
/// obligations fix consumer identity, location and exact free-variable demand.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Obligations {
    pub declarations: BTreeMap<DeclarationId, Declaration>,
    pub sources: BTreeMap<SourceId, Source>,
    pub boundaries: BTreeMap<BoundaryId, BoundaryFacts>,
    pub publications: BTreeMap<PublicationId, PublicationObligation>,
    pub reads: BTreeMap<ReadId, ReadPacket>,
    pub copies: BTreeMap<CopyId, Copy>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Representation {
    pub declaration: DeclarationId,
    pub source: SourceId,
    pub column: String,
    pub snapshot: SnapshotId,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedField {
    pub representation: RepresentationId,
    pub buffers: BTreeMap<BufferKind, BufferId>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Publication {
    pub boundary: BoundaryId,
    pub fields: BTreeMap<String, PublishedField>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Records {
    pub representations: BTreeMap<RepresentationId, Representation>,
    pub publications: BTreeMap<PublicationId, Publication>,
    pub reads: BTreeMap<ReadId, ReadPacket>,
    pub copies: BTreeMap<CopyId, Copy>,
}

/// Optional facts in the existing Structures program. The ordinary contract
/// always checks these when present; no separate unchecked pipeline exists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contract {
    pub obligations: Obligations,
    pub records: Records,
    pub blocks: crate::blocks::Contract,
}

fn reject(message: &str) -> syn::Error {
    syn::Error::new(Span::call_site(), format!("encoded contract: {message}"))
}
fn require(condition: bool, message: &str) -> syn::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(reject(message))
    }
}
fn get<'a, K: Ord, V>(map: &'a BTreeMap<K, V>, key: &K, message: &str) -> syn::Result<&'a V> {
    map.get(key).ok_or_else(|| reject(message))
}

/// Formal rule STR-CarrierRead: check original obligations and recorded facts,
/// not equality to a preferred realization. No selector is called here.
pub(crate) fn check(program: &Program, facts: &Analysis) -> syn::Result<()> {
    if program.encoded == Contract::default() {
        return Ok(());
    }
    let Contract {
        obligations: o,
        records: r,
        ..
    } = &program.encoded;
    require(
        r.publications.keys().eq(o.publications.keys()),
        "publication coverage",
    )?;
    require(r.reads.keys().eq(o.reads.keys()), "read coverage")?;
    require(r.copies.keys().eq(o.copies.keys()), "copy coverage")?;
    let origins = representation_bases(program, facts);
    for declaration in o.declarations.values() {
        require(
            facts
                .providers
                .declarations
                .iter()
                .any(|provider| symbol_name(&provider.token) == declaration.provider),
            "unknown provider declaration",
        )?;
        require(
            matches!(
                (declaration.role, declaration.family, declaration.layout),
                (Role::ScanColumn, Family::Plain | Family::Bytes, Layout::S1)
                    | (
                        Role::IndexRun,
                        Family::IndexByteAddressable,
                        Layout::ProviderIndex
                    )
            ),
            "role/family/layout mismatch",
        )?;
        require(
            !declaration.identity || declaration.family != Family::Bytes,
            "framed Bytes storage is not identity",
        )?;
        require(
            declaration.buffers.contains(&BufferKind::Payload),
            "missing payload declaration",
        )?;
        require(
            declaration.layout != Layout::S1 || declaration.buffers.contains(&BufferKind::Headers),
            "S1 requires headers",
        )?;
        require(
            declaration.domain.dictionary.is_none()
                || declaration.buffers.contains(&BufferKind::Dictionary),
            "dictionary descriptor required",
        )?;
    }
    for boundary in o.boundaries.values() {
        require(
            !boundary.live_count.is_empty()
                && !boundary.capacity.is_empty()
                && !boundary.readiness.is_empty(),
            "missing boundary count/capacity/readiness reference",
        )?;
        for retained in &boundary.retained_sources {
            let source = get(&o.sources, &retained.source, "unknown retained source")?;
            require(
                source.snapshot == retained.snapshot,
                "stale retained snapshot",
            )?;
        }
        for buffer in boundary.buffers.values() {
            require(
                boundary.retained_sources.contains(&buffer.source),
                "buffer owner not retained",
            )?;
            require(
                !buffer.initialized_bytes.is_empty() && !buffer.owner.is_empty(),
                "missing initialized extent or retained owner",
            )?;
        }
    }
    let mut used = crate::blocks::check(program, facts)?;
    for (id, publication) in &r.publications {
        let expected = &o.publications[id];
        require(
            publication.boundary == expected.boundary,
            "changed publication boundary",
        )?;
        require(
            publication.fields.keys().eq(expected.fields.keys()),
            "publication field coverage",
        )?;
        let boundary = get(
            &o.boundaries,
            &publication.boundary,
            "unknown publication boundary",
        )?;
        for (logical, field) in &publication.fields {
            let demand = &expected.fields[logical];
            let representation = get(
                &r.representations,
                &field.representation,
                "unknown representation",
            )?;
            used.insert(field.representation);
            let declaration = get(
                &o.declarations,
                &representation.declaration,
                "unknown representation declaration",
            )?;
            let source = get(&o.sources, &demand.source, "unknown source")?;
            let serving_provider = match declaration.role {
                Role::ScanColumn => {
                    facts
                        .fallbacks
                        .iter()
                        .any(|((occurrence, reader, key), provider)| {
                            occurrence == &source.occurrence
                                && matches!(reader, crate::ReaderRef::Scan)
                                && key.is_empty()
                                && symbol_name(&provider.token) == declaration.provider
                        })
                }
                Role::IndexRun => facts
                    .built
                    .get(&expected.producer)
                    .into_iter()
                    .flatten()
                    .any(|(_, provider)| symbol_name(&provider.token) == declaration.provider),
            };
            require(
                serving_provider,
                "declaration provider does not serve this representation",
            )?;
            require(
                representation.source == demand.source && representation.column == demand.column,
                "changed source column",
            )?;
            require(
                representation.snapshot == source.snapshot,
                "stale representation snapshot",
            )?;
            require(
                declaration.domain == demand.domain,
                "changed domain or dictionary version",
            )?;
            require(declaration.role == demand.role, "changed storage role")?;
            require(
                demand.preserves.is_subset(&declaration.preserves),
                "missing preserved operation",
            )?;
            let column = program
                .physical
                .program
                .declarations
                .iter()
                .find(|relation| symbol_name(&relation.name) == source.relation)
                .and_then(|relation| {
                    relation
                        .columns
                        .iter()
                        .find(|column| symbol_name(&column.name) == demand.column)
                })
                .ok_or_else(|| reject("unknown source column declaration"))?;
            require(
                column.domain.as_ref().and_then(|domain| domain.id) == demand.domain.code
                    && (column.domain.is_none() || demand.domain.code.is_some()),
                "domain does not match source declaration",
            )?;
            require(
                origins
                    .get(&expected.producer)
                    .into_iter()
                    .flatten()
                    .any(|origin| {
                        origin.base == source.relation
                            && origin.occurrence == source.occurrence
                            && origin.columns.get(logical) == Some(&demand.column)
                    }),
                "changed consumer lineage",
            )?;
            let retained = RetainedSource {
                source: demand.source,
                snapshot: source.snapshot,
            };
            require(
                boundary.retained_sources.contains(&retained),
                "source snapshot not retained",
            )?;
            require(
                field
                    .buffers
                    .keys()
                    .copied()
                    .eq(declaration.buffers.iter().copied()),
                "buffer coverage",
            )?;
            for (kind, buffer_id) in &field.buffers {
                let buffer = get(&boundary.buffers, buffer_id, "unknown published buffer")?;
                require(
                    buffer.source == retained
                        && buffer.column == demand.column
                        && buffer.kind == *kind,
                    "wrong buffer source, column or kind",
                )?;
            }
        }
    }
    require(
        used.iter().eq(r.representations.keys()),
        "unreferenced representation decision",
    )?;
    for (id, copy) in &r.copies {
        require(copy == &o.copies[id], "changed copy obligation")?;
        require(
            copy.reason == CopyReason::CrossDevice,
            "transport requires CrossDevice reason",
        )?;
        let publication = get(
            &r.publications,
            &copy.publication,
            "unknown copy publication",
        )?;
        require(
            copy.from == publication.boundary,
            "copy source publication mismatch",
        )?;
        let from = get(&o.boundaries, &copy.from, "unknown copy source boundary")?;
        let to = get(&o.boundaries, &copy.to, "unknown copy destination boundary")?;
        require(
            from.location != to.location,
            "CrossDevice requires distinct host/device locations",
        )?;
        require(
            from.live_count == to.live_count,
            "copy changes live count identity",
        )?;
        let mut demanded = BTreeSet::new();
        for read in o
            .reads
            .values()
            .filter(|read| read.consumer == Consumer::Transport(*id))
        {
            require(
                read.publication == copy.publication
                    && read.boundary == copy.from
                    && read.purpose == ReadPurpose::PreserveCarrier,
                "transport is not a final answer",
            )?;
            for field in &read.fields {
                demanded.extend(
                    get(&publication.fields, field, "missing transport field")?
                        .buffers
                        .values()
                        .copied(),
                );
            }
        }
        require(
            demanded.iter().eq(copy.buffers.keys()),
            "copy must contain exactly demanded buffers",
        )?;
        require(
            copy.buffers.values().collect::<BTreeSet<_>>().len() == copy.buffers.len(),
            "copy destinations alias",
        )?;
        for (source, target) in &copy.buffers {
            let source = get(&from.buffers, source, "unknown source buffer")?;
            let target = get(&to.buffers, target, "unknown destination buffer")?;
            require(
                source.source == target.source
                    && source.column == target.column
                    && source.kind == target.kind
                    && source.initialized_bytes == target.initialized_bytes,
                "copy changes snapshot, column, buffer kind or initialized extent",
            )?;
        }
    }
    let mut consumers = BTreeSet::new();
    for (id, read) in &r.reads {
        require(
            read == &o.reads[id],
            "changed read consumer, purpose, boundary or fields",
        )?;
        require(
            consumers.insert((
                read.publication,
                read.boundary,
                &read.consumer,
                read.purpose,
            )),
            "duplicate consumer read authorization",
        )?;
        let publication = get(
            &r.publications,
            &read.publication,
            "unknown read publication",
        )?;
        let expected = &o.publications[&read.publication];
        let boundary = get(&o.boundaries, &read.boundary, "unknown read boundary")?;
        require(
            read.fields
                .iter()
                .all(|field| publication.fields.contains_key(field)),
            "missing read field",
        )?;
        if read.boundary != publication.boundary {
            require(
                r.copies.values().any(|copy| {
                    copy.publication == read.publication
                        && copy.to == read.boundary
                        && read.fields.iter().all(|name| {
                            publication.fields[name]
                                .buffers
                                .values()
                                .all(|buffer| copy.buffers.contains_key(buffer))
                        })
                }),
                "read has no demanded-buffer transfer",
            )?;
        }
        match (&read.consumer, read.purpose) {
            (Consumer::Transport(copy), ReadPurpose::PreserveCarrier) => {
                require(r.copies.contains_key(copy), "unknown transport copy")?;
            }
            (Consumer::Relational(consumer), ReadPurpose::PreserveCarrier) => {
                require(
                    program
                        .physical
                        .program
                        .definitions
                        .iter()
                        .any(|definition| {
                            symbol_name(&definition.name) == *consumer
                                && (consumer == &expected.producer
                                    || definition
                                        .operator
                                        .operands()
                                        .iter()
                                        .any(|input| symbol_name(input) == expected.producer))
                        }),
                    "unrelated relational consumer",
                )?;
            }
            (Consumer::OpaqueRust(consumer), ReadPurpose::OpaqueRustRead) => {
                let definition = program
                    .physical
                    .program
                    .definitions
                    .iter()
                    .find(|definition| symbol_name(&definition.name) == *consumer)
                    .ok_or_else(|| reject("unknown opaque consumer"))?;
                require(
                    definition
                        .operator
                        .input()
                        .is_some_and(|input| symbol_name(input) == expected.producer),
                    "opaque consumer input lineage mismatch",
                )?;
                require(
                    facts.physical.rust_reads(&definition.operator).as_ref() == Some(&read.fields),
                    "opaque read must cover exactly its free variables",
                )?;
                if boundary.location == Location::Device {
                    require(
                        read.fields.iter().all(|name| {
                            let representation =
                                &r.representations[&publication.fields[name].representation];
                            o.declarations[&representation.declaration].identity
                        }),
                        "opaque device read requires explicit identity provider",
                    )?;
                }
            }
            (Consumer::FinalAnswer(consumer), ReadPurpose::FinalHostAnswer) => {
                require(
                    boundary.location == Location::Host
                        && expected.producer == symbol_name(&program.physical.program.returned)
                        && consumer == &symbol_name(&program.physical.program.head.relation),
                    "final reconstruction requires the actual host answer consumer",
                )?;
                let fields = program
                    .physical
                    .program
                    .head
                    .args
                    .iter()
                    .flat_map(|expression| {
                        crate::demand::columns_used_rust(
                            expression,
                            &facts.physical.headings[&expected.producer],
                            None,
                        )
                    })
                    .collect::<BTreeSet<_>>();
                require(fields == read.fields, "final answer field coverage")?;
            }
            _ => return Err(reject("read purpose does not authorize this consumer")),
        }
    }
    Ok(())
}

// The optional IR envelope uses a deliberately uniform, lossless grammar:
// records are parenthesized field lists, maps/sets are braced lists, names are
// string literals, and enums have explicit atom tags. This is recorded data,
// not Rust expressions. Empty contracts print no tokens.
trait Wire: Sized {
    fn read(input: ParseStream<'_>) -> syn::Result<Self>;
    fn emit(&self, tokens: &mut TokenStream);
}
impl Wire for u64 {
    fn read(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<syn::LitInt>()?.base10_parse()
    }
    fn emit(&self, tokens: &mut TokenStream) {
        syn::LitInt::new(&self.to_string(), Span::call_site()).to_tokens(tokens);
    }
}
impl Wire for bool {
    fn read(input: ParseStream<'_>) -> syn::Result<Self> {
        Ok(input.parse::<syn::LitBool>()?.value)
    }
    fn emit(&self, tokens: &mut TokenStream) {
        syn::LitBool::new(*self, Span::call_site()).to_tokens(tokens);
    }
}
impl Wire for String {
    fn read(input: ParseStream<'_>) -> syn::Result<Self> {
        Ok(input.parse::<syn::LitStr>()?.value())
    }
    fn emit(&self, tokens: &mut TokenStream) {
        syn::LitStr::new(self, Span::call_site()).to_tokens(tokens);
    }
}
impl Wire for DomainId {
    fn read(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse()
    }
    fn emit(&self, tokens: &mut TokenStream) {
        self.to_tokens(tokens);
    }
}
impl Wire for cq_syntax::LogicalTypeId {
    fn read(input: ParseStream<'_>) -> syn::Result<Self> { input.parse() }
    fn emit(&self, tokens: &mut TokenStream) { self.to_tokens(tokens); }
}
impl<T: Wire> Wire for Option<T> {
    fn read(input: ParseStream<'_>) -> syn::Result<Self> {
        let body;
        parenthesized!(body in input);
        let result = if body.is_empty() {
            None
        } else {
            Some(T::read(&body)?)
        };
        require(body.is_empty(), "extra option fields")?;
        Ok(result)
    }
    fn emit(&self, tokens: &mut TokenStream) {
        let mut body = TokenStream::new();
        if let Some(value) = self {
            value.emit(&mut body);
        }
        tokens.extend(quote!((#body)));
    }
}
impl<K: Wire + Ord, V: Wire> Wire for BTreeMap<K, V> {
    fn read(input: ParseStream<'_>) -> syn::Result<Self> {
        let body;
        braced!(body in input);
        let mut result = Self::new();
        while !body.is_empty() {
            let key = K::read(&body)?;
            body.parse::<Token![:]>()?;
            require(
                result.insert(key, V::read(&body)?).is_none(),
                "duplicate map key",
            )?;
            if !body.is_empty() {
                body.parse::<Token![,]>()?;
            }
        }
        Ok(result)
    }
    fn emit(&self, tokens: &mut TokenStream) {
        let mut body = TokenStream::new();
        for (key, value) in self {
            key.emit(&mut body);
            body.extend(quote!(:));
            value.emit(&mut body);
            body.extend(quote!(,));
        }
        tokens.extend(quote!({#body}));
    }
}
impl<T: Wire + Ord> Wire for BTreeSet<T> {
    fn read(input: ParseStream<'_>) -> syn::Result<Self> {
        let body;
        braced!(body in input);
        let mut result = Self::new();
        while !body.is_empty() {
            require(result.insert(T::read(&body)?), "duplicate set member")?;
            if !body.is_empty() {
                body.parse::<Token![,]>()?;
            }
        }
        Ok(result)
    }
    fn emit(&self, tokens: &mut TokenStream) {
        let mut body = TokenStream::new();
        for value in self {
            value.emit(&mut body);
            body.extend(quote!(,));
        }
        tokens.extend(quote!({#body}));
    }
}
macro_rules! record {
    ($name:ident { $($field:ident),* $(,)? }) => {
        impl Wire for $name {
            fn read(input: ParseStream<'_>) -> syn::Result<Self> {
                let body; parenthesized!(body in input);
                let result = Self { $($field: { let value = Wire::read(&body)?; body.parse::<Token![,]>()?; value }),* };
                require(body.is_empty(), "extra record fields")?;
                Ok(result)
            }
            fn emit(&self, tokens: &mut TokenStream) {
                let mut body = TokenStream::new();
                $(self.$field.emit(&mut body); body.extend(quote!(,));)*
                tokens.extend(quote!((#body)));
            }
        }
    };
}
macro_rules! atoms {
    ($name:ident { $($variant:ident),* $(,)? }) => {
        impl Wire for $name {
            fn read(input: ParseStream<'_>) -> syn::Result<Self> {
                let tag: syn::Ident = input.parse()?;
                match tag.to_string().as_str() {
                    $(stringify!($variant) => Ok(Self::$variant),)*
                    _ => Err(syn::Error::new_spanned(tag, concat!("unknown ", stringify!($name)))),
                }
            }
            fn emit(&self, tokens: &mut TokenStream) {
                let tag = match self { $(Self::$variant => stringify!($variant),)* };
                syn::Ident::new(tag, Span::call_site()).to_tokens(tokens);
            }
        }
    };
}
atoms!(Family {
    Plain,
    Bytes,
    IndexByteAddressable
});
atoms!(Layout { S1, ProviderIndex });
atoms!(Role {
    ScanColumn,
    IndexRun
});
atoms!(Carrier { U8, U16, U32, U64 });
atoms!(Preservation { Equality, Order });
atoms!(BufferKind {
    Payload,
    Headers,
    BlockSummaries,
    VectorSummaries,
    Dictionary,
    Positions
});
atoms!(Location { Host, Device });
atoms!(ReadPurpose {
    PreserveCarrier,
    OpaqueRustRead,
    FinalHostAnswer
});
atoms!(CopyReason {
    CrossDevice,
    Reorganize
});
record!(Dictionary { identity, version });
record!(Domain { code, dictionary });
record!(Declaration {
    provider,
    domain,
    role,
    family,
    layout,
    carrier,
    identity,
    preserves,
    buffers
});
record!(Source {
    relation,
    occurrence,
    snapshot
});
record!(RetainedSource { source, snapshot });
record!(BufferFacts {
    source,
    column,
    kind,
    initialized_bytes,
    owner
});
record!(BoundaryFacts {
    location,
    live_count,
    capacity,
    readiness,
    retained_sources,
    buffers
});
record!(FieldRequirement {
    source,
    column,
    domain,
    role,
    preserves
});
record!(PublicationObligation {
    producer,
    boundary,
    fields
});
record!(ReadPacket {
    publication,
    boundary,
    consumer,
    purpose,
    fields
});
record!(Copy {
    publication,
    from,
    to,
    reason,
    buffers
});
record!(Obligations {
    declarations,
    sources,
    boundaries,
    publications,
    reads,
    copies
});
record!(Representation {
    declaration,
    source,
    column,
    snapshot
});
record!(PublishedField {
    representation,
    buffers
});
record!(Publication { boundary, fields });
record!(Records {
    representations,
    publications,
    reads,
    copies
});
record!(Contract {
    obligations,
    records,
    blocks
});
mod blocked_wire {
    use super::*;
    use crate::blocks::{Column, Source, Scan, Predicate, FieldRead, Read, Contract, Summary, Prune, Test, Translation, Strategy, ReadAction};
    use crate::blocks::{CanonicalKeys, Operand};
    atoms!(Summary { None, MinMax });
    atoms!(Prune { None, MinMax });
    atoms!(Test { Encoded, DecodeCodes });
    atoms!(Translation { S1Ordinal });
    atoms!(Strategy { Window, Point, Formula });
    atoms!(ReadAction { Owned, BorrowedScoped, Ordinal });
    record!(Column { slot, logical, declaration, widths });
    record!(Source { block_rows, summary, columns });
    record!(Scan { source, columns });
    record!(Predicate { input, source, columns, prune, test, strategy, translation, preserves });
    record!(FieldRead { representation, action });
    record!(Read { id, input, consumer, purpose, fields });
    record!(CanonicalKeys { input, fields, ordinary });
    impl Wire for Operand {
        fn read(input: ParseStream<'_>) -> syn::Result<Self> {
            let tag: syn::Ident = input.parse()?;
            match tag.to_string().as_str() {
                "Owned" => Ok(Self::Owned { column: String::read(input)? }),
                _ => Err(syn::Error::new_spanned(tag, "unknown blocked operand")),
            }
        }
        fn emit(&self, tokens: &mut TokenStream) {
            match self {
                Self::Owned { column } => { syn::Ident::new("Owned", Span::call_site()).to_tokens(tokens); column.emit(tokens); }
            }
        }
    }
    record!(Contract { sources, scans, predicates, canonical_keys, reads });
}
impl Wire for Consumer {
    fn read(input: ParseStream<'_>) -> syn::Result<Self> {
        let tag: syn::Ident = input.parse()?;
        match tag.to_string().as_str() {
            "Relational" => Ok(Self::Relational(String::read(input)?)),
            "OpaqueRust" => Ok(Self::OpaqueRust(String::read(input)?)),
            "FinalAnswer" => Ok(Self::FinalAnswer(String::read(input)?)),
            "Transport" => Ok(Self::Transport(CopyId::read(input)?)),
            _ => Err(syn::Error::new_spanned(tag, "unknown encoded consumer")),
        }
    }
    fn emit(&self, tokens: &mut TokenStream) {
        let (tag, value) = match self {
            Self::Relational(value) => ("Relational", {
                let mut tokens = TokenStream::new();
                value.emit(&mut tokens);
                tokens
            }),
            Self::OpaqueRust(value) => ("OpaqueRust", {
                let mut tokens = TokenStream::new();
                value.emit(&mut tokens);
                tokens
            }),
            Self::FinalAnswer(value) => ("FinalAnswer", {
                let mut tokens = TokenStream::new();
                value.emit(&mut tokens);
                tokens
            }),
            Self::Transport(value) => ("Transport", {
                let mut tokens = TokenStream::new();
                value.emit(&mut tokens);
                tokens
            }),
        };
        syn::Ident::new(tag, Span::call_site()).to_tokens(tokens);
        tokens.extend(value);
    }
}
impl Contract {
    pub(crate) fn parse_optional(input: ParseStream<'_>) -> syn::Result<Self> {
        if !input.peek(kw::encoded) {
            return Ok(Self::default());
        }
        input.parse::<kw::encoded>()?;
        let body;
        braced!(body in input);
        let result = Self::read(&body)?;
        require(body.is_empty(), "extra encoded contract fields")?;
        Ok(result)
    }
}
impl ToTokens for Contract {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        if self == &Self::default() {
            return;
        }
        let mut body = TokenStream::new();
        self.emit(&mut body);
        tokens.extend(quote!(encoded { #body }));
    }
}
