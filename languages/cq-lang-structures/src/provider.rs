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
//! Provider    ::= "@" Ident ["(" Provider ")"] "as" RustType
//! Declaration ::= Ident "=" RustType "{" Fact,* "}" ";"
//! Fact ::= "reads" "[" ("exact" | "range" | "membership" | "ordered"
//!              | "distinct" | "resident"),* "]"
//!        | "supports" "[" ("retain" | "expand" | "keys"),* "]"
//!        | "build" ("inferred" | "macro") | "accounting" ("rows" | "keys")
//!        | ("nested" | "routed") Integer | "join_filter" | "shared"
//!        | "packed_bits" Integer
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
}

impl Capability {
    const ALL: [(Capability, &'static str); 6] = [
        (Capability::Exact, "exact"),
        (Capability::Range, "range"),
        (Capability::Membership, "membership"),
        (Capability::Ordered, "ordered"),
        (Capability::Distinct, "distinct"),
        (Capability::Resident, "resident"),
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
    /// `cq_storage::build!(Type, pairs)`: explicit providers dispatch at the call site.
    Macro,
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
}

impl Facts {
    pub fn reads(&self, capability: &Capability) -> bool {
        self.reads.contains(capability)
    }

    pub fn supports(&self, capability: ProviderCapability) -> bool {
        self.supports.contains(&capability)
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
            ty,
        })
    }
}

/// A recorded provider instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provider {
    pub token: Ident,
    pub inner: Option<Box<Provider>>,
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
        input.parse::<Token![as]>()?;
        Ok(Self {
            token,
            inner,
            ty: input.parse()?,
        })
    }
}

impl ToTokens for Provider {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let token = &self.token;
        let ty = &self.ty;
        tokens.extend(match &self.inner {
            Some(inner) => quote::quote!(@#token(#inner) as #ty),
            None => quote::quote!(@#token as #ty),
        });
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
            build: Build::Macro,
            accounting: Accounting::None,
            composition: None,
            join_filter: false,
            shared: false,
            packed_bits: None,
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
                    body.parse::<syn::Token![macro]>()?;
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
            Build::Macro => quote::quote!(build macro),
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
        if declaration.facts.composition.is_some() {
            declaration.compose(Provider {
                token: declaration.token.clone(),
                inner: None,
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
