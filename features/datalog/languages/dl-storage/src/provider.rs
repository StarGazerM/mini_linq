//! Provider tokens (WF-DL-Storage, DL-LayoutProtocol): a Storage provider is a
//! declared token, never a Rust type.
//!
//! Every provider-bearing Storage/Round field records a [`Provider`]: the
//! token id chosen by its selecting pass and the Rust spelling printers emit
//! verbatim. Layouts and key sets also record their [`Hasher`] state. The
//! program's `tokens` table declares each token once with the facts later
//! passes read: maintenance protocol, canonical admission strategy, whether
//! the provider accepts a hasher state, the task-candidate hasher, the
//! spelling function that produces its Rust type from the recorded key/payload
//! schema and hasher state, and the CQ host reads (IF-HostDecl) its instances
//! answer on their recorded key. No pass inspects a provider spelling; the
//! contract checks each recorded spelling against its token's spelling function.
//! ```text
//! Provider ::= "@" Ident "as" RustType
//! Token    ::= token {Ident} spelling {std_set | std_map | rows | sharded | runs | verbatim}
//!              protocol {[set | grouped_map | bulk]} admission {sharded_hash | sorted_subtract | none}
//!              hashed {bool} task_hasher {layout | fx}
//!              reads {[HostRead,*]} supports {[(retain | expand),*]}
//! ```
// DESIGN datalog-provider-token
// Contract 7: selection records a token id; facts travel in the program's
// `tokens` table. Forbidden: matching a provider's Rust path, segments or
// generic arguments to recover its family, hasher, protocol or admission.
use crate::{LayoutProtocol, text::Text};
pub use cq_syntax::{HostRead, HostSupport};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Ident, Type, parse::ParseStream};

/// A recorded provider instance: `@token as Type`. `ty` is the printed spelling only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provider {
    pub token: Ident,
    pub ty: Type,
}
impl Text for Provider {
    fn parse_text(input: ParseStream<'_>) -> syn::Result<Self> {
        input.parse::<syn::Token![@]>()?;
        let token = input.parse()?;
        input.parse::<syn::Token![as]>()?;
        Ok(Self {
            token,
            ty: input.parse()?,
        })
    }
    fn text(&self) -> TokenStream {
        let (token, ty) = (&self.token, &self.ty);
        quote!(@#token as #ty)
    }
}
impl syn::parse::Parse for Provider {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        Self::parse_text(input)
    }
}

/// Recorded hasher state of one hashed layout or key set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hasher {
    Fx,
    Keyed,
    None,
}
impl Hasher {
    /// The `BuildHasher` state type, absent for unhashed providers.
    pub fn state(self) -> Option<Type> {
        match self {
            Self::Fx => Some(syn::parse_quote!(
                ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>
            )),
            Self::Keyed => Some(syn::parse_quote!(::cq_storage::HashState)),
            Self::None => None,
        }
    }
}

/// Canonical admission strategy of a bulk canonical layout (DL-ShardedAdmission).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    ShardedHash,
    SortedSubtract,
    None,
}

/// Hasher of task-local candidate deduplication: the canonical layout's own
/// recorded state, or the default Fx state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskHasher {
    Layout,
    Fx,
}

/// The spelling function a token declares. Each function builds the Rust type
/// from the instance's key row type, payload row type and hasher state;
/// `Verbatim` tokens (authored or device providers) carry their recorded
/// spelling unchanged and are never respelled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spelling {
    StdSet,
    StdMap,
    Rows,
    Sharded,
    Runs,
    Verbatim,
}
impl Spelling {
    /// Whether this spelling function takes a hasher state.
    pub fn hashed(self) -> Option<bool> {
        match self {
            Self::StdSet | Self::StdMap | Self::Sharded => Some(true),
            Self::Rows | Self::Runs => Some(false),
            Self::Verbatim => None,
        }
    }
    /// Spell an instance; `None` for verbatim tokens or a hasher state the
    /// function does not accept. `key` is the key row type (the full row for
    /// `rows`), `payload` the payload row type (ignored by sets and rows).
    pub fn spell(self, key: &Type, payload: &Type, hasher: Hasher) -> Option<Type> {
        let state = hasher.state();
        Some(match (self, hasher) {
            (Self::StdSet, Hasher::Keyed) => syn::parse_quote!(::cq_storage::HashSet<#key>),
            (Self::StdSet, Hasher::Fx) => {
                syn::parse_quote!(::std::collections::HashSet<#key, #state>)
            }
            (Self::StdMap, Hasher::Fx | Hasher::Keyed) => syn::parse_quote!(
                ::std::collections::HashMap<#key, ::std::vec::Vec<#payload>, #state>
            ),
            (Self::Sharded, Hasher::Fx | Hasher::Keyed) => syn::parse_quote!(
                ::cq_datalog_parallel_runtime::ShardedLayout<#key, #payload, #state>
            ),
            (Self::Runs, Hasher::None) => {
                syn::parse_quote!(::cq_datalog_parallel_runtime::RunLayout<#key, #payload>)
            }
            (Self::Rows, Hasher::None) => syn::parse_quote!(::std::vec::Vec<#key>),
            _ => return None,
        })
    }
}

/// One token declaration of the Storage `tokens` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub token: Ident,
    pub spelling: Spelling,
    /// Maintenance protocol of layouts with this token; `[]` for buffers.
    pub protocol: Option<LayoutProtocol>,
    pub admission: Admission,
    /// The provider accepts a hasher state (its instances record fx or keyed).
    pub hashed: bool,
    pub task_hasher: TaskHasher,
    /// CQ host reads every instance answers natively on its recorded key
    /// (the layout's key columns); `[]` for row buffers and opaque hosts.
    pub reads: Vec<HostRead>,
    /// Cursor contracts of the declared `exact` read.
    pub supports: Vec<HostSupport>,
}
crate::record!(Token {
    token: Ident,
    spelling: Spelling,
    protocol: Option<LayoutProtocol>,
    admission: Admission,
    hashed: bool,
    task_hasher: TaskHasher,
    reads: Vec<HostRead>,
    supports: Vec<HostSupport>
});

/// Base Storage tokens, declared by the Storage pass.
pub const HASH_SET: &str = "hash_set";
pub const HASH_MAP: &str = "hash_map";
pub const ROWS_VEC: &str = "rows_vec";

fn token(
    name: &str,
    spelling: Spelling,
    protocol: Option<LayoutProtocol>,
    admission: Admission,
    task_hasher: TaskHasher,
    reads: &[HostRead],
) -> Token {
    Token {
        token: Ident::new(name, proc_macro2::Span::call_site()),
        spelling,
        protocol,
        admission,
        hashed: spelling.hashed().unwrap_or(false),
        task_hasher,
        reads: reads.to_vec(),
        supports: Vec::new(),
    }
}
/// A payload-free key set of all-column keys (also negation key sets).
/// `HashSet<K, S>` answers `ExactMembership<K>`; it has no `IndexRead`.
pub fn hash_set() -> Token {
    token(
        HASH_SET,
        Spelling::StdSet,
        Some(LayoutProtocol::Set),
        Admission::None,
        TaskHasher::Fx,
        &[HostRead::Membership],
    )
}
/// Payload rows grouped by key. `HashMap<K, Vec<V>, S>` answers
/// `IndexRead`/`PointLookup`, `ExactMembership<K>` and `Candidates`; its
/// slice cursor is not `Copy`, so it declares no cursor contracts.
pub fn hash_map() -> Token {
    token(
        HASH_MAP,
        Spelling::StdMap,
        Some(LayoutProtocol::GroupedMap),
        Admission::None,
        TaskHasher::Fx,
        &[HostRead::Exact, HostRead::Membership, HostRead::Candidates],
    )
}
/// Delta/next row buffers: plain row sources.
pub fn rows_vec() -> Token {
    token(
        ROWS_VEC,
        Spelling::Rows,
        None,
        Admission::None,
        TaskHasher::Fx,
        &[],
    )
}
/// A declaration with explicit facts: authored, extension or device tokens.
#[allow(clippy::too_many_arguments)]
pub fn declared(
    name: &str,
    spelling: Spelling,
    protocol: Option<LayoutProtocol>,
    admission: Admission,
    hashed: bool,
    task_hasher: TaskHasher,
    reads: &[HostRead],
    supports: &[HostSupport],
) -> Token {
    Token {
        hashed,
        supports: supports.to_vec(),
        ..token(name, spelling, protocol, admission, task_hasher, reads)
    }
}

/// The CQ host declaration of an input bound to an instance of `declaration`
/// whose recorded key is `key` (its leading declared columns): one surface
/// with the token's reads and supports, or none for an empty key or a token
/// declaring no reads. A `borrowed` instance stores borrowed key columns and
/// answers only the token's membership read, through the query's borrowed key.
pub fn host_decl(
    declaration: &Token,
    key: &[Ident],
    borrowed: bool,
) -> Option<cq_syntax::HostDecl> {
    if borrowed {
        let membership = declaration.reads.contains(&HostRead::Membership);
        return (!key.is_empty() && membership)
            .then(|| syn::parse_quote!(host [#(#key),*] borrowed reads [membership]));
    }
    if key.is_empty() || declaration.reads.is_empty() {
        return None;
    }
    let reads = declaration.reads.iter().map(Text::text);
    let supports = (!declaration.supports.is_empty()).then(|| {
        let supports = declaration.supports.iter().map(Text::text);
        quote!(supports [#(#supports),*])
    });
    Some(syn::parse_quote!(host [#(#key),*] reads [#(#reads),*] #supports))
}

/// Add a declaration unless its token is already declared.
pub fn declare(tokens: &mut Vec<Token>, declaration: Token) {
    if !tokens.iter().any(|token| token.token == declaration.token) {
        tokens.push(declaration);
    }
}
/// The declaration of a recorded token id.
pub fn lookup<'a>(tokens: &'a [Token], token: &Ident) -> syn::Result<&'a Token> {
    tokens
        .iter()
        .find(|declaration| declaration.token == *token)
        .ok_or_else(|| syn::Error::new_spanned(token, "provider token is not declared"))
}
/// The token's spelling of an instance with this schema and hasher.
pub fn spell(declaration: &Token, key: &Type, payload: &Type, hasher: Hasher) -> syn::Result<Type> {
    declaration
        .spelling
        .spell(key, payload, hasher)
        .ok_or_else(|| {
            syn::Error::new_spanned(
                &declaration.token,
                "provider token has no spelling for this hasher state",
            )
        })
}

/// Check a recorded instance against its declaration: hasher state matches
/// `hashed`, and a spelled token's type is its spelling function's output.
pub fn check_instance<'a>(
    tokens: &'a [Token],
    provider: &Provider,
    key: &Type,
    payload: &Type,
    hasher: Hasher,
) -> syn::Result<&'a Token> {
    let declaration = lookup(tokens, &provider.token)?;
    crate::ensure(
        declaration.hashed == (hasher != Hasher::None),
        &provider.token,
        "provider hasher state differs from its token's hashed fact",
    )?;
    if declaration.spelling != Spelling::Verbatim {
        crate::ensure(
            spell(declaration, key, payload, hasher)? == provider.ty,
            &provider.ty,
            "provider spelling differs from its token's spelling of the recorded schema",
        )?;
    }
    Ok(declaration)
}

/// Well-formed table: unique ids; spelled tokens agree with their function;
/// host reads and supports are listed once, and supports need an exact read.
pub fn check_table(tokens: &[Token]) -> syn::Result<()> {
    for (index, declaration) in tokens.iter().enumerate() {
        crate::ensure(
            !tokens[..index]
                .iter()
                .any(|previous| previous.token == declaration.token),
            &declaration.token,
            "duplicate provider token declaration",
        )?;
        crate::ensure(
            declaration
                .spelling
                .hashed()
                .is_none_or(|hashed| hashed == declaration.hashed),
            &declaration.token,
            "provider token hashed fact differs from its spelling function",
        )?;
        crate::ensure(
            declaration.admission == Admission::None
                || declaration.protocol == Some(LayoutProtocol::Bulk),
            &declaration.token,
            "only bulk provider tokens declare a canonical admission strategy",
        )?;
        crate::ensure(
            declaration.task_hasher == TaskHasher::Fx || declaration.hashed,
            &declaration.token,
            "a layout task hasher requires a hashed provider token",
        )?;
        crate::ensure(
            distinct(&declaration.reads) && distinct(&declaration.supports),
            &declaration.token,
            "a provider token lists each host read and support once",
        )?;
        crate::ensure(
            declaration.supports.is_empty() || declaration.reads.contains(&HostRead::Exact),
            &declaration.token,
            "provider token cursor supports require an exact host read",
        )?;
    }
    Ok(())
}
fn distinct<T: PartialEq>(items: &[T]) -> bool {
    items
        .iter()
        .enumerate()
        .all(|(index, item)| !items[..index].contains(item))
}

macro_rules! keyword_text { ($ty:ident, $what:literal, $($name:ident => $variant:ident),* $(,)?) => {
    impl Text for $ty {
        fn parse_text(input: ParseStream<'_>) -> syn::Result<Self> {
            let name: Ident = input.parse()?;
            match name.to_string().as_str() {
                $(stringify!($name) => Ok(Self::$variant),)*
                _ => Err(syn::Error::new_spanned(name, concat!("unknown ", $what))),
            }
        }
        fn text(&self) -> TokenStream {
            match self { $(Self::$variant => quote!($name),)* }
        }
    }
}; }
keyword_text!(Hasher, "hasher state", fx => Fx, keyed => Keyed, none => None);
keyword_text!(Admission, "admission strategy", sharded_hash => ShardedHash, sorted_subtract => SortedSubtract, none => None);
keyword_text!(TaskHasher, "task hasher", layout => Layout, fx => Fx);
keyword_text!(Spelling, "spelling function", std_set => StdSet, std_map => StdMap, rows => Rows, sharded => Sharded, runs => Runs, verbatim => Verbatim);
keyword_text!(HostRead, "host read", exact => Exact, membership => Membership, candidates => Candidates, range => Range, ordered => Ordered);
keyword_text!(HostSupport, "host support", retain => Retain, expand => Expand);
