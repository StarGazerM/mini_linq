//! Provider tokens (STR-Provider): a provider is a declared token, never a Rust type.
//!
//! Every provider-bearing Structures field records a [`Provider`]: the token
//! id chosen by its selecting pass or authored advice, the inner instance of a
//! nested or routed token, and the Rust spelling that printers emit verbatim.
//! The program's `providers { .. }` table declares each referenced token once,
//! with the facts every later pass reads ([`Facts`]). No pass inspects a
//! provider spelling: rustc alone checks it against the declared facts' trait
//! obligations. Comparing a recorded token id with an id is identity, not
//! recognition.
//! ```text
//! Provider    ::= "@" Ident ["(" Provider ")"] [InputSchema] [RadixAdvice] "as" RustType
//! InputSchema ::= "input" "{" "keys" Integer ";" "columns" "[" InputColumn,* "]" "}"
//! InputColumn ::= RustType "=>" RustType ["codec" ["[" CodecId "]"] RustType]
//! RadixAdvice ::= "radix" "{" "word" RustType ";" "offset" RustType ";"
//!                 "ratio" Integer ";" "radix_bits" Integer ";" "merge_tile" Integer "}"
//! Declaration ::= Ident "=" RustType "{" Fact,* "}" ";"
//! Fact ::= "reads" "[" ("exact" | "range" | "membership" | "ordered"
//!              | "distinct" | "resident" | "candidates"),* "]"
//!        | "supports" "[" ("retain" | "expand" | "keys"),* "]"
//!        | "build" ("inferred" | "typed") | "accounting" ("rows" | "keys")
//!        | ("nested" | "routed") Integer | "join_filter" | "shared"
//!        | "packed_bits" Integer | "abi" ("range" | "runs" | "columns" | "radix")
//!        | "contiguous_range" | "bounds" "resident_columns"
//!        | "items" Integer "[" "rows" RustType "," "columns" RustType "]"
//!        | "choice" Integer "[" "eager" GenericArgument "," "lazy" GenericArgument "]"
//!        | "statistics" "[" StatDeclaration,* "]"
//! StatDeclaration ::= Ident ":" Field Unit Precision Producer Accessor
//!                     Placement Availability Computation Aggregation
//! ```

// DESIGN provider-token
// Contract 7 (user decision 2026-10-31, STRICT): selection or authored advice
// records a token id; declared facts travel in the program's table and every
// later pass reads them. Forbidden: matching a provider's Rust type structure,
// path segments or last segment, matching row element types, or deriving any
// capability, ABI, routing, accounting or level from a spelling.
use super::*;

pub mod base;

pub mod kw {
    syn::custom_keyword!(providers);
    syn::custom_keyword!(reads);
    syn::custom_keyword!(supports);
    syn::custom_keyword!(build);
    syn::custom_keyword!(inferred);
    syn::custom_keyword!(typed);
    syn::custom_keyword!(accounting);
    syn::custom_keyword!(rows);
    syn::custom_keyword!(keys);
    syn::custom_keyword!(nested);
    syn::custom_keyword!(routed);
    syn::custom_keyword!(join_filter);
    syn::custom_keyword!(shared);
    syn::custom_keyword!(packed_bits);
    syn::custom_keyword!(exact);
    syn::custom_keyword!(range);
    syn::custom_keyword!(membership);
    syn::custom_keyword!(ordered);
    syn::custom_keyword!(distinct);
    syn::custom_keyword!(resident);
    syn::custom_keyword!(retain);
    syn::custom_keyword!(expand);
    syn::custom_keyword!(input);
    syn::custom_keyword!(columns);
    syn::custom_keyword!(codec);
    syn::custom_keyword!(abi);
    syn::custom_keyword!(runs);
    syn::custom_keyword!(contiguous_range);
    syn::custom_keyword!(bounds);
    syn::custom_keyword!(resident_columns);
    syn::custom_keyword!(radix);
    syn::custom_keyword!(word);
    syn::custom_keyword!(offset);
    syn::custom_keyword!(ratio);
    syn::custom_keyword!(radix_bits);
    syn::custom_keyword!(merge_tile);
    syn::custom_keyword!(items);
    syn::custom_keyword!(choice);
    syn::custom_keyword!(eager);
    syn::custom_keyword!(lazy);
    syn::custom_keyword!(statistics);
}

/// Declared read capabilities, discharged against Sequence obligations.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    Exact,
    Range,
    Membership,
    Ordered,
    /// The native input handle already denotes immutable target-resident storage.
    /// Its recorded read levels need no generated host-adapter retention.
    Resident,
    /// Formal rule PL-Distinct: the provider's build from any row stream
    /// yields exactly that stream's distinct complete rows, and its scan reads
    /// each stored row once. Such a provider may type a canonical rows record
    /// (a canon result's identity index, SRDatalog's sorted array).
    Distinct,
    /// Formal rule GPU-RadixCandidates: for a two-level `[prefix][v]` instance,
    /// enumerate each distinct `v` under a bound prefix exactly once, with the
    /// exact distinct count across every publication (union, not a sum of
    /// per-run counts) and membership of `(prefix, v)`. `Range` alone never
    /// discharges this obligation of a PH-Smallest candidate side.
    Candidates,
}

impl Capability {
    const ALL: [(Capability, &'static str); 7] = [
        (Capability::Exact, "exact"),
        (Capability::Range, "range"),
        (Capability::Membership, "membership"),
        (Capability::Ordered, "ordered"),
        (Capability::Distinct, "distinct"),
        (Capability::Resident, "resident"),
        (Capability::Candidates, "candidates"),
    ];

    fn name(&self) -> &'static str {
        Self::ALL.iter().find(|(c, _)| c == self).expect("listed").1
    }
}

/// How generated Rust constructs a built instance from its key/payload pairs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Build {
    /// `Type::build(pairs)`: the spelling is a bare path whose generics Rust infers.
    Inferred,
    /// `<Type as cq_storage::Build<_>>::build(pairs)`: one direct call on the
    /// recorded spelling, which fixes the provider and its codec.
    Typed,
}

/// Allocation accounting of a retained instance (`cq_storage::retention`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accounting {
    None,
    /// A collected row vector: `retain_rows`.
    Rows,
    /// A hashed key set: `retain_keys`, profiled as a key-set build.
    Keys,
}

/// A composite token's inner instance and the generic argument (counting
/// lifetimes) of its default spelling that the inner spelling occupies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Composition {
    /// Curried levels: the inner instance stores each outer key's next level.
    Nested(usize),
    /// Formal rule STR-Routed: disjoint hash-routed parts, one inner instance
    /// per part. It reads `inner.reads ∩ {exact, membership}`, supports
    /// `inner.supports ∩ {keys}` and enumerates a supplementary join filter
    /// iff its inner does; a routed instance is never routed again.
    Routed(usize),
}

/// Formal rule GPU-NativeAbi: native ABI and input representation are recorded
/// facts; no Rust type structure selects a protocol, encoding or bounds source.
/// Native binding protocol declared independently of its opaque Rust spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeAbi {
    Range,
    Runs,
    Columns,
    /// Grouped radix index (`RadixLayout`): whole packed order keys, explicit
    /// item lanes; every instance records its [`RadixAdvice`].
    Radix,
}

/// Formal rule GPU-RadixAdvice: authored per-instance parameters of a radix
/// provider. The key word and offset carriers are opaque spellings that the
/// printer passes as the instance's generic arguments (rustc checks them
/// against the instance spelling); the runtime chooses none of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadixAdvice {
    pub word: syn::Type,
    pub offset: syn::Type,
    pub ratio: usize,
    pub radix_bits: u32,
    pub merge_tile: u32,
}

/// The recorded source of native input column bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundsSource {
    ResidentColumns,
}

/// Loader-authored input layout; the first `key_count` columns form the key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputSchema {
    pub key_count: usize,
    pub columns: Vec<InputColumn>,
}

/// Declaration-table identity; Rust spelling is never an admission key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CodecId(pub u32);

impl syn::parse::Parse for CodecId {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let content;
        syn::bracketed!(content in input);
        let value: syn::LitInt = content.parse()?;
        if !content.is_empty() {
            return Err(content.error("a codec identity is one integer"));
        }
        Ok(Self(value.base10_parse()?))
    }
}

impl ToTokens for CodecId {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let value = syn::Index::from(self.0 as usize);
        tokens.extend(quote::quote!([#value]));
    }
}

/// All three types are opaque. Codec presence, not spelling, records encoding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputColumn {
    pub logical: syn::Type,
    pub storage: syn::Type,
    pub codec: Option<syn::Type>,
    pub codec_id: Option<CodecId>,
}

/// Intern authored codec spellings once; already recorded identities are opaque.
/// Exhausting the declaration identity space is a parse error, never wraparound.
pub fn intern_codecs<'a>(
    columns: impl IntoIterator<Item = &'a mut InputColumn>,
) -> syn::Result<()> {
    let mut columns: Vec<_> = columns.into_iter().collect();
    if columns
        .iter()
        .all(|column| column.codec.is_none() || column.codec_id.is_some())
    {
        return Ok(());
    }
    let mut identities = std::collections::BTreeMap::new();
    let mut next = Some(0u32);
    for column in &columns {
        if let (Some(codec), Some(id)) = (&column.codec, column.codec_id) {
            identities
                .entry(codec.to_token_stream().to_string())
                .or_insert(id);
            next = next.and_then(|next| id.0.checked_add(1).map(|after| next.max(after)));
        }
    }
    for column in &mut columns {
        if column.codec_id.is_none() {
            if let Some(codec) = &column.codec {
                let spelling = codec.to_token_stream().to_string();
                let id = match identities.entry(spelling) {
                    std::collections::btree_map::Entry::Occupied(entry) => *entry.get(),
                    std::collections::btree_map::Entry::Vacant(entry) => {
                        let value = next.ok_or_else(|| {
                            syn::Error::new_spanned(codec, "codec identity space exhausted")
                        })?;
                        next = value.checked_add(1);
                        *entry.insert(CodecId(value))
                    }
                };
                column.codec_id = Some(id);
            }
        }
    }
    Ok(())
}

/// The generic arguments of `ty`'s last path segment when the spelling ends
/// exactly before argument `index` (lifetimes counted): the position of a
/// declared argument slot that a recorded fact prints. Navigation only, like
/// [`Declaration::compose`]; no argument is inspected.
fn slot_arguments(
    ty: &mut syn::Type,
    index: usize,
) -> Option<&mut syn::punctuated::Punctuated<syn::GenericArgument, Token![,]>> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last_mut()?;
    if matches!(segment.arguments, syn::PathArguments::None) {
        segment.arguments = syn::PathArguments::AngleBracketed(syn::parse_quote!(<>));
    }
    match &mut segment.arguments {
        syn::PathArguments::AngleBracketed(arguments) if arguments.args.len() == index => {
            Some(&mut arguments.args)
        }
        _ => None,
    }
}

/// Check that an instance spelling ends before slot `index`, which `rule`'s
/// recorded fact prints.
fn check_slot(provider: &Provider, index: usize, rule: &str) -> syn::Result<()> {
    if slot_arguments(&mut provider.ty.clone(), index).is_none() {
        return error(
            &provider.ty,
            format!(
                "{rule}: spell exactly {index} generic arguments; the recorded fact prints the next one"
            ),
        );
    }
    Ok(())
}

/// `provider` with `argument` printed at its checked slot `index`.
fn print_slot(provider: &Provider, index: usize, argument: syn::GenericArgument) -> Provider {
    let mut printed = provider.clone();
    slot_arguments(&mut printed.ty, index)
        .expect("checked instance spelling ends before its declared slot")
        .push(argument);
    printed
}

/// Formal rule STR-ItemLayout: the item-layout argument of a payload
/// provider. An instance spelling ends right before argument `index`: the
/// recorded `items` fact alone states the layout, printed as the declared
/// `rows` or `columns` spelling at that argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemSlot {
    pub index: usize,
    pub rows: syn::Type,
    pub columns: syn::Type,
}

impl ItemSlot {
    /// The printed spelling of `provider` storing items in `layout`.
    pub fn print(&self, provider: &Provider, layout: &ItemLayout) -> Provider {
        let spelling = match layout {
            ItemLayout::Rows(_) => &self.rows,
            ItemLayout::Columns(_) => &self.columns,
        };
        print_slot(
            provider,
            self.index,
            syn::GenericArgument::Type(spelling.clone()),
        )
    }
}

/// Formal rule STR-ReaderChoice: the argument of a host reader's provider
/// that realizes the reader's recorded Physical `eager`/`lazy` choice. An
/// instance spelling ends right before argument `index`; the served reader's
/// choice alone selects the declared `eager` or `lazy` argument (a reader
/// recording no `lazy` choice prints `eager`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceSlot {
    pub index: usize,
    pub eager: syn::GenericArgument,
    pub lazy: syn::GenericArgument,
}

impl ChoiceSlot {
    /// The printed spelling of `provider` serving a reader whose recorded
    /// choice is lazy iff `lazy`.
    pub fn print(&self, provider: &Provider, lazy: bool) -> Provider {
        let argument = if lazy { &self.lazy } else { &self.eager };
        print_slot(provider, self.index, argument.clone())
    }
}

/// Declared facts of one provider token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    pub reads: Vec<Capability>,
    /// The strongest support an instance may declare (`supports [..]` ⊆ this).
    pub supports: Vec<ProviderCapability>,
    pub build: Build,
    pub accounting: Accounting,
    pub composition: Option<Composition>,
    /// Payload-directory key enumeration eligible as a supplementary join filter source.
    pub join_filter: bool,
    /// Reads may be captured by pool and task readers (the instance is `Sync`).
    pub shared: bool,
    /// Packed equality-key width limit, checked against codec bounds at setup.
    pub packed_bits: Option<u32>,
    pub abi: Option<NativeAbi>,
    pub contiguous_range: bool,
    pub bounds: Option<BoundsSource>,
    /// STR-ItemLayout: the declared item-layout argument of a payload provider.
    pub items: Option<ItemSlot>,
    /// STR-ReaderChoice: the declared reader-choice argument of a host provider.
    pub choice: Option<ChoiceSlot>,
    /// Optional statistic ABI declarations; declaring a field never selects it.
    pub statistics: Vec<stats::StatDeclaration>,
}

impl Facts {
    pub fn reads(&self, capability: &Capability) -> bool {
        self.reads.contains(capability)
    }

    pub fn supports(&self, capability: ProviderCapability) -> bool {
        self.supports.contains(&capability)
    }

    /// The declared argument slot a recorded fact prints, with its rule.
    fn slot(&self) -> Option<(usize, &'static str)> {
        match (&self.items, &self.choice) {
            (Some(slot), _) => Some((slot.index, "STR-ItemLayout")),
            (None, Some(slot)) => Some((slot.index, "STR-ReaderChoice")),
            (None, None) => None,
        }
    }

    /// STR-Routed: the facts of a routed instance over `inner`.
    pub fn routed(&self, inner: &Facts) -> Facts {
        Facts {
            reads: inner
                .reads
                .iter()
                .filter(|capability| {
                    matches!(capability, Capability::Exact | Capability::Membership)
                })
                .cloned()
                .collect(),
            supports: inner
                .supports
                .iter()
                .filter(|support| **support == ProviderCapability::Keys)
                .copied()
                .collect(),
            join_filter: inner.join_filter,
            ..self.clone()
        }
    }
}

/// One declared token: id, default spelling and facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declaration {
    pub token: Ident,
    pub ty: syn::Type,
    pub facts: Facts,
}

impl Declaration {
    /// The default instance of a non-composite token.
    pub fn instance(&self) -> Provider {
        Provider {
            token: self.token.clone(),
            inner: None,
            input: None,
            radix: None,
            ty: self.ty.clone(),
        }
    }

    /// A composite instance over `inner`: the declared default spelling with
    /// the inner spelling at the declared argument. This edits the
    /// declaration's own spelling, never an input type.
    pub fn compose(&self, inner: Provider) -> syn::Result<Provider> {
        let index = match self.facts.composition {
            Some(Composition::Nested(index) | Composition::Routed(index)) => index,
            None => return error(&self.token, "token is not composite"),
        };
        let mut ty = self.ty.clone();
        let slot = match &mut ty {
            syn::Type::Path(path) => {
                path.path
                    .segments
                    .last_mut()
                    .and_then(|segment| match &mut segment.arguments {
                        syn::PathArguments::AngleBracketed(arguments) => {
                            arguments.args.iter_mut().nth(index)
                        }
                        _ => None,
                    })
            }
            _ => None,
        };
        let Some(syn::GenericArgument::Type(slot)) = slot else {
            return error(
                &self.ty,
                "declared composition argument is not a type argument",
            );
        };
        *slot = inner.ty.clone();
        Ok(Provider {
            token: self.token.clone(),
            inner: Some(Box::new(inner)),
            input: None,
            radix: None,
            ty,
        })
    }
}

/// A recorded provider instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provider {
    pub token: Ident,
    pub inner: Option<Box<Provider>>,
    pub input: Option<InputSchema>,
    pub radix: Option<RadixAdvice>,
    /// Printed spelling only.
    pub ty: syn::Type,
}

impl Provider {
    /// Same token, recursively through inner instances; spellings may differ.
    pub fn same_token(&self, other: &Provider) -> bool {
        self.token == other.token
            && match (&self.inner, &other.inner) {
                (None, None) => true,
                (Some(left), Some(right)) => left.same_token(right),
                _ => false,
            }
    }

    /// Every token of this instance, outermost first.
    pub fn tokens(&self) -> impl Iterator<Item = &Ident> {
        std::iter::successors(Some(self), |provider| provider.inner.as_deref())
            .map(|provider| &provider.token)
    }
}

impl syn::parse::Parse for Provider {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<Token![@]>()?;
        let token = input.parse()?;
        let inner = if input.peek(token::Paren) {
            let body;
            syn::parenthesized!(body in input);
            let inner = body.parse()?;
            if !body.is_empty() {
                return Err(body.error("expected one inner provider"));
            }
            Some(Box::new(inner))
        } else {
            None
        };
        let schema = if input.peek(kw::input) {
            Some(input.parse()?)
        } else {
            None
        };
        let radix = if input.peek(kw::radix) {
            Some(input.parse()?)
        } else {
            None
        };
        input.parse::<Token![as]>()?;
        Ok(Self {
            token,
            inner,
            input: schema,
            radix,
            ty: input.parse()?,
        })
    }
}

impl ToTokens for Provider {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let token = &self.token;
        let ty = &self.ty;
        let input = &self.input;
        let radix = &self.radix;
        tokens.extend(match &self.inner {
            Some(inner) => quote::quote!(@#token(#inner) #input #radix as #ty),
            None => quote::quote!(@#token #input #radix as #ty),
        });
    }
}

impl syn::parse::Parse for InputSchema {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::input>()?;
        let body;
        syn::braced!(body in input);
        body.parse::<kw::keys>()?;
        let key_count = body.parse::<syn::LitInt>()?.base10_parse()?;
        body.parse::<Token![;]>()?;
        body.parse::<kw::columns>()?;
        let columns;
        syn::bracketed!(columns in body);
        let columns = CommaList::<InputColumn>::parse_terminated(&columns)?
            .into_iter()
            .collect();
        if !body.is_empty() {
            return Err(body.error("expected end of input schema"));
        }
        Ok(Self { key_count, columns })
    }
}

impl syn::parse::Parse for RadixAdvice {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::radix>()?;
        let body;
        syn::braced!(body in input);
        body.parse::<kw::word>()?;
        let word = body.parse()?;
        body.parse::<Token![;]>()?;
        body.parse::<kw::offset>()?;
        let offset = body.parse()?;
        body.parse::<Token![;]>()?;
        body.parse::<kw::ratio>()?;
        let ratio = body.parse::<syn::LitInt>()?.base10_parse()?;
        body.parse::<Token![;]>()?;
        body.parse::<kw::radix_bits>()?;
        let radix_bits = body.parse::<syn::LitInt>()?.base10_parse()?;
        body.parse::<Token![;]>()?;
        body.parse::<kw::merge_tile>()?;
        let merge_tile = body.parse::<syn::LitInt>()?.base10_parse()?;
        if !body.is_empty() {
            return Err(body.error("expected end of radix advice"));
        }
        Ok(Self {
            word,
            offset,
            ratio,
            radix_bits,
            merge_tile,
        })
    }
}

impl ToTokens for RadixAdvice {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let Self {
            word,
            offset,
            ratio,
            radix_bits,
            merge_tile,
        } = self;
        let ratio = proc_macro2::Literal::usize_unsuffixed(*ratio);
        let radix_bits = proc_macro2::Literal::u32_unsuffixed(*radix_bits);
        let merge_tile = proc_macro2::Literal::u32_unsuffixed(*merge_tile);
        tokens.extend(quote::quote!(radix {
            word #word; offset #offset; ratio #ratio; radix_bits #radix_bits; merge_tile #merge_tile
        }));
    }
}

impl syn::parse::Parse for InputColumn {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let logical = input.parse()?;
        input.parse::<Token![=>]>()?;
        let storage = input.parse()?;
        let (codec_id, codec) = if input.peek(kw::codec) {
            input.parse::<kw::codec>()?;
            let id = if input.peek(syn::token::Bracket) {
                Some(input.parse()?)
            } else {
                None
            };
            (id, Some(input.parse()?))
        } else {
            (None, None)
        };
        Ok(Self {
            logical,
            storage,
            codec,
            codec_id,
        })
    }
}

impl ToTokens for InputSchema {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let keys = syn::Index::from(self.key_count);
        let columns = &self.columns;
        tokens.extend(quote::quote!(input { keys #keys; columns [#(#columns),*] }));
    }
}

impl ToTokens for InputColumn {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let Self {
            logical,
            storage,
            codec,
            codec_id,
        } = self;
        tokens.extend(quote::quote!(#logical => #storage));
        if let Some(codec) = codec {
            tokens.extend(quote::quote!(codec #codec_id #codec));
        }
    }
}

/// The program's token declarations: exactly the referenced tokens.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Providers {
    pub declarations: Vec<Declaration>,
}

impl Providers {
    pub fn get(&self, token: &Ident) -> Option<&Declaration> {
        self.declarations
            .iter()
            .find(|declaration| declaration.token == *token)
    }

    /// Facts of an instance, applying STR-Routed to routed tokens.
    pub fn facts(&self, provider: &Provider) -> Option<Facts> {
        facts(|token| self.get(token), provider)
    }

    /// Add a declaration unless its token is already declared.
    pub fn declare(&mut self, declaration: &Declaration) {
        if self.get(&declaration.token).is_none() {
            self.declarations.push(declaration.clone());
        }
    }
}

/// Facts of `provider` under `declared`, applying STR-Routed.
pub fn facts<'a>(
    declared: impl Fn(&Ident) -> Option<&'a Declaration> + Copy,
    provider: &Provider,
) -> Option<Facts> {
    let facts = &declared(&provider.token)?.facts;
    match (facts.composition, &provider.inner) {
        (Some(Composition::Routed(_)), Some(inner)) => {
            Some(facts.routed(&self::facts(declared, inner)?))
        }
        _ => Some(facts.clone()),
    }
}

impl syn::parse::Parse for Providers {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<kw::providers>()?;
        let body;
        syn::braced!(body in input);
        let mut declarations = Vec::new();
        while !body.is_empty() {
            declarations.push(body.parse()?);
        }
        Ok(Self { declarations })
    }
}

impl ToTokens for Providers {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let declarations = &self.declarations;
        tokens.extend(quote::quote!(providers { #(#declarations)* }));
    }
}

impl syn::parse::Parse for Declaration {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let token = input.parse()?;
        input.parse::<Token![=]>()?;
        let ty = input.parse()?;
        let body;
        syn::braced!(body in input);
        let mut facts = Facts {
            reads: Vec::new(),
            supports: Vec::new(),
            build: Build::Typed,
            accounting: Accounting::None,
            composition: None,
            join_filter: false,
            shared: false,
            packed_bits: None,
            abi: None,
            contiguous_range: false,
            bounds: None,
            items: None,
            choice: None,
            statistics: Vec::new(),
        };
        let mut seen = BTreeSet::new();
        while !body.is_empty() {
            let lookahead = body.lookahead1();
            let fact: Ident = body.fork().parse()?;
            if !seen.insert(fact.to_string()) {
                return error(&fact, "duplicate provider fact");
            }
            if lookahead.peek(kw::reads) {
                body.parse::<kw::reads>()?;
                let list;
                syn::bracketed!(list in body);
                for capability in CommaList::<Ident>::parse_terminated(&list)? {
                    let Some((capability, _)) =
                        Capability::ALL.iter().find(|(_, name)| capability == name)
                    else {
                        return error(&capability, "unknown read capability");
                    };
                    if facts.reads.contains(capability) {
                        return error(&fact, "duplicate read capability");
                    }
                    facts.reads.push(capability.clone());
                }
            } else if lookahead.peek(kw::supports) {
                body.parse::<kw::supports>()?;
                let list;
                syn::bracketed!(list in body);
                for support in CommaList::<ProviderCapabilityToken>::parse_terminated(&list)? {
                    if facts.supports.contains(&support.capability()) {
                        return error(&support, "duplicate provider capability");
                    }
                    facts.supports.push(support.capability());
                }
            } else if lookahead.peek(kw::build) {
                body.parse::<kw::build>()?;
                if body.peek(kw::inferred) {
                    body.parse::<kw::inferred>()?;
                    facts.build = Build::Inferred;
                } else {
                    body.parse::<kw::typed>()?;
                }
            } else if lookahead.peek(kw::accounting) {
                body.parse::<kw::accounting>()?;
                if body.peek(kw::rows) {
                    body.parse::<kw::rows>()?;
                    facts.accounting = Accounting::Rows;
                } else {
                    body.parse::<kw::keys>()?;
                    facts.accounting = Accounting::Keys;
                }
            } else if lookahead.peek(kw::nested) || lookahead.peek(kw::routed) {
                if facts.composition.is_some() {
                    return error(&fact, "a token has one composition");
                }
                let routed = body.peek(kw::routed);
                body.parse::<Ident>()?;
                let index = body.parse::<syn::LitInt>()?.base10_parse()?;
                facts.composition = Some(if routed {
                    Composition::Routed(index)
                } else {
                    Composition::Nested(index)
                });
            } else if lookahead.peek(kw::join_filter) {
                body.parse::<kw::join_filter>()?;
                facts.join_filter = true;
            } else if lookahead.peek(kw::shared) {
                body.parse::<kw::shared>()?;
                facts.shared = true;
            } else if lookahead.peek(kw::packed_bits) {
                body.parse::<kw::packed_bits>()?;
                facts.packed_bits = Some(body.parse::<syn::LitInt>()?.base10_parse()?);
            } else if lookahead.peek(kw::abi) {
                body.parse::<kw::abi>()?;
                let abi: Ident = body.parse()?;
                facts.abi = Some(match abi.to_string().as_str() {
                    "range" => NativeAbi::Range,
                    "runs" => NativeAbi::Runs,
                    "columns" => NativeAbi::Columns,
                    "radix" => NativeAbi::Radix,
                    _ => return error(&abi, "unknown native ABI"),
                });
            } else if lookahead.peek(kw::contiguous_range) {
                body.parse::<kw::contiguous_range>()?;
                facts.contiguous_range = true;
            } else if lookahead.peek(kw::bounds) {
                body.parse::<kw::bounds>()?;
                body.parse::<kw::resident_columns>()?;
                facts.bounds = Some(BoundsSource::ResidentColumns);
            } else if lookahead.peek(kw::items) {
                body.parse::<kw::items>()?;
                let index = body.parse::<syn::LitInt>()?.base10_parse()?;
                let list;
                syn::bracketed!(list in body);
                list.parse::<kw::rows>()?;
                let rows = list.parse()?;
                list.parse::<Token![,]>()?;
                list.parse::<kw::columns>()?;
                let columns = list.parse()?;
                if !list.is_empty() {
                    return Err(list.error("expected `rows Type, columns Type`"));
                }
                facts.items = Some(ItemSlot {
                    index,
                    rows,
                    columns,
                });
            } else if lookahead.peek(kw::choice) {
                body.parse::<kw::choice>()?;
                let index = body.parse::<syn::LitInt>()?.base10_parse()?;
                let list;
                syn::bracketed!(list in body);
                list.parse::<kw::eager>()?;
                let eager = list.parse()?;
                list.parse::<Token![,]>()?;
                list.parse::<kw::lazy>()?;
                let lazy = list.parse()?;
                if !list.is_empty() {
                    return Err(list.error("expected `eager Argument, lazy Argument`"));
                }
                facts.choice = Some(ChoiceSlot { index, eager, lazy });
            } else if lookahead.peek(kw::statistics) {
                body.parse::<kw::statistics>()?;
                let list;
                syn::bracketed!(list in body);
                facts.statistics = CommaList::<stats::StatDeclaration>::parse_terminated(&list)?
                    .into_iter()
                    .collect();
            } else {
                return Err(lookahead.error());
            }
            if !body.is_empty() {
                body.parse::<Token![,]>()?;
            }
        }
        input.parse::<Token![;]>()?;
        Ok(Self { token, ty, facts })
    }
}

impl ToTokens for Declaration {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let Facts {
            reads,
            supports,
            build,
            accounting,
            composition,
            join_filter,
            shared,
            packed_bits,
            abi,
            contiguous_range,
            bounds,
            items,
            choice,
            statistics,
        } = &self.facts;
        let reads = reads
            .iter()
            .map(|capability| quote::format_ident!("{}", capability.name()));
        let mut facts = vec![quote::quote!(reads [#(#reads),*])];
        if !supports.is_empty() {
            let supports = supports.iter().map(|support| match support {
                ProviderCapability::Retain => quote::quote!(retain),
                ProviderCapability::Expand => quote::quote!(expand),
                ProviderCapability::Keys => quote::quote!(keys),
            });
            facts.push(quote::quote!(supports [#(#supports),*]));
        }
        facts.push(match build {
            Build::Inferred => quote::quote!(build inferred),
            Build::Typed => quote::quote!(build typed),
        });
        match accounting {
            Accounting::None => {}
            Accounting::Rows => facts.push(quote::quote!(accounting rows)),
            Accounting::Keys => facts.push(quote::quote!(accounting keys)),
        }
        match composition {
            None => {}
            Some(Composition::Nested(index)) => {
                let index = syn::Index::from(*index);
                facts.push(quote::quote!(nested #index));
            }
            Some(Composition::Routed(index)) => {
                let index = syn::Index::from(*index);
                facts.push(quote::quote!(routed #index));
            }
        }
        if *join_filter {
            facts.push(quote::quote!(join_filter));
        }
        if *shared {
            facts.push(quote::quote!(shared));
        }
        if let Some(bits) = packed_bits {
            let bits = proc_macro2::Literal::u32_unsuffixed(*bits);
            facts.push(quote::quote!(packed_bits #bits));
        }
        if let Some(abi) = abi {
            facts.push(match abi {
                NativeAbi::Range => quote::quote!(abi range),
                NativeAbi::Runs => quote::quote!(abi runs),
                NativeAbi::Columns => quote::quote!(abi columns),
                NativeAbi::Radix => quote::quote!(abi radix),
            });
        }
        if *contiguous_range {
            facts.push(quote::quote!(contiguous_range));
        }
        if let Some(BoundsSource::ResidentColumns) = bounds {
            facts.push(quote::quote!(bounds resident_columns));
        }
        if let Some(ItemSlot {
            index,
            rows,
            columns,
        }) = items
        {
            let index = syn::Index::from(*index);
            facts.push(quote::quote!(items #index [rows #rows, columns #columns]));
        }
        if let Some(ChoiceSlot { index, eager, lazy }) = choice {
            let index = syn::Index::from(*index);
            facts.push(quote::quote!(choice #index [eager #eager, lazy #lazy]));
        }
        if !statistics.is_empty() {
            facts.push(quote::quote!(statistics [#(#statistics),*]));
        }
        let token = &self.token;
        let ty = &self.ty;
        tokens.extend(quote::quote!(#token = #ty { #(#facts),* };));
    }
}

/// Formal rule STR-Provider: unique declarations, exactly the referenced
/// tokens, composite instances iff composite tokens, a routed instance never
/// routed again, and instance support within the declared support.
pub(crate) fn check(structures: &Structures) -> syn::Result<()> {
    let mut declared = BTreeSet::new();
    for declaration in &structures.providers.declarations {
        if !declared.insert(declaration.token.to_string()) {
            return error(&declaration.token, "duplicate provider declaration");
        }
        concrete_type(&declaration.ty)?;
        let facts = &declaration.facts;
        stats::check_declarations(&facts.statistics)?;
        if facts
            .statistics
            .iter()
            .any(|statistic| statistic.field == stats::Field::Bounds)
            && !facts.reads(&Capability::Ordered)
        {
            return error(
                &declaration.token,
                "STR-BuildStats: code bounds require the declared ordered capability",
            );
        }
        if facts.contiguous_range
            && (facts.abi != Some(NativeAbi::Range) || !facts.reads(&Capability::Range))
        {
            return error(
                &declaration.token,
                "contiguous_range requires range ABI and range reads",
            );
        }
        if facts.bounds.is_some() && facts.abi != Some(NativeAbi::Columns) {
            return error(
                &declaration.token,
                "resident column bounds require columns ABI",
            );
        }
        if facts.abi.is_some() && facts.composition.is_some() {
            return error(&declaration.token, "native ABI requires a simple provider");
        }
        if facts.slot().is_some() && facts.composition.is_some() {
            return error(
                &declaration.token,
                "an argument slot requires a simple provider",
            );
        }
        if facts.items.is_some() && facts.choice.is_some() {
            return error(
                &declaration.token,
                "a token declares at most one argument slot",
            );
        }
        if let Some((index, rule)) = facts.slot() {
            check_slot(&declaration.instance(), index, rule)?;
        }
        if declaration.facts.composition.is_some() {
            declaration.compose(Provider {
                token: declaration.token.clone(),
                inner: None,
                input: None,
                radix: None,
                ty: syn::parse_quote!(_),
            })?;
        }
    }
    let mut referenced = BTreeSet::new();
    for entry in &structures.entries {
        let Some((provider, support)) = entry.structure.provider() else {
            continue;
        };
        check_instance(&structures.providers, provider, false)?;
        referenced.extend(provider.tokens().map(ToString::to_string));
        let facts = structures
            .providers
            .facts(provider)
            .expect("checked instance");
        if matches!(
            &entry.structure,
            Structure::Host {
                reader: ReaderRef::Scan,
                ..
            }
        ) && matches!(facts.abi, Some(NativeAbi::Runs | NativeAbi::Columns))
            && provider.input.is_none()
        {
            return error(
                &provider.token,
                "native runs and columns scans require an input schema",
            );
        }
        // Formal rule STR-ItemLayout: a built payload (non-membership)
        // instance of a token declaring an item-layout argument records its
        // layout, and only such an instance does; the fact is that argument's
        // only source. Key-only membership storage and host readers keep the
        // type's default layout. STR-ReaderChoice: a choice argument is
        // printed from the reader a host or built entry serves.
        let declared = &structures
            .providers
            .get(&provider.token)
            .expect("checked instance")
            .facts;
        match &entry.structure {
            Structure::Built { keyed, .. } => match (&keyed.items, &declared.items) {
                (None, Some(_)) if keyed.membership.is_none() => {
                    return error(
                        &provider.token,
                        "STR-ItemLayout: record `items rows` or `items columns` for this provider's layout argument",
                    );
                }
                (Some(items), None) => {
                    return error(
                        items,
                        "STR-ItemLayout: the provider token declares no item-layout argument",
                    );
                }
                _ => {}
            },
            Structure::Host { .. } => {}
            _ if declared.slot().is_some() => {
                return error(
                    &provider.token,
                    "a token declaring an argument slot types only built and host keyed entries",
                );
            }
            _ => {}
        }
        for capability in support
            .into_iter()
            .flat_map(|support| &support.capabilities)
        {
            if !facts.supports(capability.capability()) {
                return error(capability, "support is not declared by the provider token");
            }
        }
    }
    if let Some(unused) = structures
        .providers
        .declarations
        .iter()
        .find(|declaration| !referenced.contains(&declaration.token.to_string()))
    {
        return error(&unused.token, "provider declaration is not referenced");
    }
    Ok(())
}

fn check_instance(providers: &Providers, provider: &Provider, in_routed: bool) -> syn::Result<()> {
    let Some(declaration) = providers.get(&provider.token) else {
        return error(&provider.token, "undeclared provider token");
    };
    concrete_type(&provider.ty)?;
    if let Some((index, rule)) = declaration.facts.slot() {
        check_slot(provider, index, rule)?;
    }
    if let Some(input) = &provider.input {
        if input
            .columns
            .iter()
            .any(|column| column.codec.is_some() != column.codec_id.is_some())
        {
            return error(
                &provider.token,
                "codec identity must accompany its declared spelling",
            );
        }
        if input.key_count > input.columns.len() {
            return error(&provider.token, "input key count exceeds column count");
        }
    }
    let radix = declaration.facts.abi == Some(NativeAbi::Radix);
    match &provider.radix {
        None if radix => {
            return error(
                &provider.token,
                "a radix provider instance records its radix advice",
            );
        }
        Some(_) if !radix => {
            return error(&provider.token, "radix advice requires the radix ABI");
        }
        Some(advice) => {
            concrete_type(&advice.word)?;
            concrete_type(&advice.offset)?;
            if advice.ratio < 2 || advice.radix_bits > 32 || advice.merge_tile == 0 {
                return error(
                    &provider.token,
                    "radix advice needs ratio >= 2, radix_bits <= 32 and merge_tile >= 1",
                );
            }
        }
        None => {}
    }
    match (declaration.facts.composition, &provider.inner) {
        (None, None) => Ok(()),
        (Some(Composition::Routed(_)), Some(_)) if in_routed => {
            error(&provider.token, "a routed provider is not routed again")
        }
        (Some(composition), Some(inner)) => check_instance(
            providers,
            inner,
            in_routed || matches!(composition, Composition::Routed(_)),
        ),
        (Some(_), None) => error(
            &provider.token,
            "a composite provider records its inner instance",
        ),
        (None, Some(_)) => error(&provider.token, "a simple provider has no inner instance"),
    }
}
