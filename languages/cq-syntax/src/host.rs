//! Declared host read capabilities (IF-HostDecl).
//!
//! A host relation is a plain row source unless its declaration states which
//! keyed reads the host value answers itself. Structure selection records a
//! native read only for a capability declared here on exactly the read's key;
//! every other keyed read uses its recorded fallback index, and constant
//! points and windows scan and filter. The compiler never inspects the host's
//! Rust type: generated code makes one direct call per recorded read, so a
//! host whose type lacks the declared capability is a rustc trait error.
//!
//! ```text
//! HostDecl ::= "host" "[" Ident,* "]" ["borrowed"] "reads" "[" HostRead,* "]"
//!              ["supports" "[" ("retain" | "expand"),* "]"]
//! HostRead ::= "exact" | "membership" | "candidates" | "range" | "ordered"
//! ```
//!
//! `borrowed` declares that the host stores borrowed key columns (for example
//! a set of `(&T,)` rows): its membership reads take the query's borrowed key
//! `(&T, ..)` unchanged (`ExactMembership<(&T, ..)>`). Otherwise `K` below is the tuple of owned column types and each
//! read owns the query key once. The key names declared columns. It is either a declaration prefix (as a
//! set) or one non-leading column read through an explicitly indexed slot
//! (`cq_storage::dispatch::IndexedColumn`). Rust obligations of each read, on
//! the host value `H` with declared key tuple `K` of owned column types:
//! `exact` — `IndexRead<Key = K>` for stream probes, `PointLookup<Key = K>`
//! for constant points; `membership` — `ExactMembership<K>`; `candidates` —
//! `Candidates<Key = K>`; `range` — `SeekRangeLookup<(&T,)>` on a one-column
//! prefix; `ordered` — `OrderedRead` ordered by the prefix. `supports
//! [retain]` requires `exact` and a `Copy` cursor; `expand` additionally an
//! exact-length cursor.
use super::*;

pub mod kw {
    syn::custom_keyword!(host);
    syn::custom_keyword!(reads);
    syn::custom_keyword!(supports);
    syn::custom_keyword!(exact);
    syn::custom_keyword!(membership);
    syn::custom_keyword!(candidates);
    syn::custom_keyword!(range);
    syn::custom_keyword!(ordered);
    syn::custom_keyword!(retain);
    syn::custom_keyword!(expand);
    syn::custom_keyword!(borrowed);
}

/// One declared host read capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostRead {
    Exact,
    Membership,
    Candidates,
    Range,
    Ordered,
}

impl HostRead {
    const ALL: [(HostRead, &'static str); 5] = [
        (HostRead::Exact, "exact"),
        (HostRead::Membership, "membership"),
        (HostRead::Candidates, "candidates"),
        (HostRead::Range, "range"),
        (HostRead::Ordered, "ordered"),
    ];

    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(read, _)| *read == self)
            .expect("listed")
            .1
    }

    /// Reads that need the key to be the declaration prefix.
    pub fn prefix_only(self) -> bool {
        matches!(self, Self::Candidates | Self::Range | Self::Ordered)
    }
}

/// A capability keyword with its span.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostReadToken {
    pub read: HostRead,
    pub span: Ident,
}

impl syn::parse::Parse for HostReadToken {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let span: Ident = input.parse()?;
        let read = HostRead::ALL
            .iter()
            .find(|(_, name)| span == name)
            .map(|(read, _)| *read)
            .ok_or_else(|| {
                syn::Error::new_spanned(
                    &span,
                    "expected a host read: exact, membership, candidates, range or ordered",
                )
            })?;
        Ok(Self { read, span })
    }
}

impl quote::ToTokens for HostReadToken {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        self.span.to_tokens(tokens);
    }
}

/// Declared cursor contracts of a host's `exact` read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostSupport {
    Retain,
    Expand,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum HostSupportToken {
    #[parse(peek = kw::retain)]
    Retain(kw::retain),
    #[parse(peek = kw::expand)]
    Expand(kw::expand),
}

impl HostSupportToken {
    pub fn support(&self) -> HostSupport {
        match self {
            Self::Retain(_) => HostSupport::Retain,
            Self::Expand(_) => HostSupport::Expand,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct HostSupports {
    pub supports_token: kw::supports,
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    #[parse(CommaList::parse_terminated)]
    pub supports: CommaList<HostSupportToken>,
}

/// One declared keyed read surface of a host value.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct HostDecl {
    pub host_token: kw::host,
    #[syn(bracketed)]
    pub key_bracket: token::Bracket,
    #[syn(in = key_bracket)]
    #[parse(CommaList::parse_terminated)]
    pub key: CommaList<Ident>,
    #[syn(optional, peek = kw::borrowed)]
    pub borrowed: Option<kw::borrowed>,
    pub reads_token: kw::reads,
    #[syn(bracketed)]
    pub reads_bracket: token::Bracket,
    #[syn(in = reads_bracket)]
    #[parse(CommaList::parse_terminated)]
    pub reads: CommaList<HostReadToken>,
    #[syn(optional, peek = kw::supports)]
    pub supports: Option<HostSupports>,
}

impl HostDecl {
    /// Sorted declared key column names.
    pub fn key_names(&self) -> Vec<String> {
        let mut key: Vec<String> = self.key.iter().map(symbol_name).collect();
        key.sort();
        key
    }

    pub fn reads(&self, read: HostRead) -> bool {
        self.reads.iter().any(|token| token.read == read)
    }

    pub fn supports(&self, support: HostSupport) -> bool {
        self.supports.as_ref().is_some_and(|supports| {
            supports.supports.iter().any(|token| {
                let declared = token.support();
                declared == support
                    || support == HostSupport::Retain && declared == HostSupport::Expand
            })
        })
    }
}

pub(crate) fn parse_hosts(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<HostDecl>> {
    let mut hosts = Vec::new();
    while input.peek(kw::host) {
        hosts.push(input.parse()?);
    }
    Ok(hosts)
}

impl RelationDecl {
    /// The declared host surface whose key set is exactly `key` (sorted names).
    pub fn host(&self, key: &[String]) -> Option<&HostDecl> {
        self.hosts.iter().find(|host| host.key_names() == key)
    }

    /// Whether `key` (sorted names) is a declaration prefix of this relation.
    pub fn prefix(&self, key: &[String]) -> bool {
        let mut prefix: Vec<String> = self
            .columns
            .iter()
            .take(key.len())
            .map(|c| symbol_name(&c.name))
            .collect();
        prefix.sort();
        prefix == key
    }

    /// Formal rule IF-HostDecl: declared keys name distinct columns, form a
    /// prefix or one indexed slot, and are declared once; prefix-only reads
    /// and cursor contracts have their required key shape and `exact` read.
    pub(crate) fn check_hosts(&self) -> syn::Result<()> {
        let fail = |spanned: &dyn quote::ToTokens, message: &str| {
            Err(syn::Error::new_spanned(spanned, message))
        };
        let declared: BTreeSet<String> =
            self.columns.iter().map(|c| symbol_name(&c.name)).collect();
        let mut keys = BTreeSet::new();
        for host in &self.hosts {
            let key = host.key_names();
            if key.is_empty() {
                return fail(host, "a host read declares a nonempty key");
            }
            if key.windows(2).any(|pair| pair[0] == pair[1]) {
                return fail(host, "a host read key repeats a column");
            }
            if let Some(column) = host
                .key
                .iter()
                .find(|c| !declared.contains(&symbol_name(c)))
            {
                return fail(column, "a host read key names a declared column");
            }
            let prefix = self.prefix(&key);
            if !prefix && key.len() != 1 {
                return fail(
                    host,
                    "a host read key is a declaration prefix or one indexed column",
                );
            }
            if !keys.insert(key) {
                return fail(host, "a host read key is declared once");
            }
            let mut reads = BTreeSet::new();
            for token in &host.reads {
                if !reads.insert(token.read) {
                    return fail(token, "duplicate host read");
                }
                if token.read.prefix_only() && !prefix {
                    return fail(
                        token,
                        "candidates, range and ordered reads need a prefix key",
                    );
                }
                if token.read == HostRead::Range && host.key.len() != 1 {
                    return fail(token, "a range read is declared on one column");
                }
                if host.borrowed.is_some() && token.read != HostRead::Membership {
                    return fail(token, "a borrowed-key host declares membership reads only");
                }
            }
            if let Some(supports) = &host.supports {
                let mut seen = BTreeSet::new();
                for token in &supports.supports {
                    if !seen.insert(token.support()) {
                        return fail(token, "duplicate host support");
                    }
                }
                if !host.reads(HostRead::Exact) {
                    return fail(supports, "retained host cursors require an exact read");
                }
            }
        }
        Ok(())
    }
}
