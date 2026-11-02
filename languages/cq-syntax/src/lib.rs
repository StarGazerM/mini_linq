//! Shared syntax, declarations and Rust free-variable/pattern analysis.
//!
//! Rust expressions are opaque, pure and total by premise, and depend only on
//! their free variables. The compiler preserves their syntax and never infers
//! relational meaning from them. Explicit `where` predicates compare column
//! representations under their recorded equality and ordering laws; rustc
//! enforces host trait bounds. Types remain host-language syntax.
//! Finite pure generators denote set-valued unnesting: order and duplicate
//! multiplicity are unobservable. Block interfaces delimit lexical scopes.
//!
use syn::{Ident, Index, Token, Type, token};

pub mod kw {
    syn::custom_keyword!(agg);
    syn::custom_keyword!(indexes);
    syn::custom_keyword!(relation);
}

/// A list that retains the separators written between its items.
///
/// It supports the familiar list operations used by the homework: iteration,
/// `len`, `is_empty`, `push`, and construction with `collect`.
pub type SyntaxList<Item, Separator> = syn::punctuated::Punctuated<Item, Separator>;

/// A syntax-preserving comma-separated list.
pub type CommaList<Item> = SyntaxList<Item, Token![,]>;

/// A syntax-preserving semicolon-separated list.
pub type SemicolonList<Item> = SyntaxList<Item, Token![;]>;

/// A zero-based relation-column number written as a Rust integer literal.
pub type Column = Index;

/// Return the MiniLinq symbol represented by a Rust identifier.
///
/// In particular, `x` and the Rust raw spelling `r#x` return the same name.
pub fn symbol_name(identifier: &Ident) -> String {
    let mut name = identifier.to_string();
    if name.starts_with("r#") {
        name.drain(..2);
    }
    name
}

pub mod keyed;
pub use keyed::SupplyKind;
pub mod constraint;
pub use constraint::Constraint;
pub mod capture;
pub mod rust;
pub use capture::{Capture, Captures};

pub mod host;
pub use host::{HostDecl, HostRead, HostSupport};

/// One named, typed column in a declared input relation.
///
/// ```text
/// ColumnDecl ::= Ident ":" Type ["in" ["[" DomainId "]"] "(" Expr ")"]
/// ```
///
/// The optional `in (D)` is the column's authored code domain: a query-closed
/// host value implementing `cq_storage::CodeDomain` whose codes the column
/// stores (WF-Domain, A-Domain). The type stays opaque; only the domain fact
/// decides operand translation.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct ColumnDecl {
    pub name: Ident,
    pub colon_token: Token![:],
    pub ty: Type,
    #[syn(optional, peek = Token![in])]
    pub domain: Option<CodeDomain>,
}

/// Declaration-table identity of an opaque code-domain spelling.
///
/// The bracketed integer is an explicit IR fact, not Rust expression syntax.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DomainId(pub u32);

impl syn::parse::Parse for DomainId {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let content;
        syn::bracketed!(content in input);
        let value: syn::LitInt = content.parse()?;
        let id = Self(value.base10_parse()?);
        if !content.is_empty() {
            return Err(content.error("a domain identity is one integer"));
        }
        Ok(id)
    }
}

impl quote::ToTokens for DomainId {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let value = syn::Index::from(self.0 as usize);
        token::Bracket::default().surround(tokens, |tokens| {
            quote::ToTokens::to_tokens(&value, tokens);
        });
    }
}

/// `in [id] (D)`: a column domain; source declarations may omit the id.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct CodeDomain {
    pub in_token: Token![in],
    /// Assigned once at the declaration boundary; retained by every later IR.
    #[syn(optional, peek = token::Bracket)]
    pub id: Option<DomainId>,
    #[syn(parenthesized)]
    pub paren_token: token::Paren,
    #[syn(in = paren_token)]
    pub expression: syn::Expr,
}

/// One typed relation declaration supplied by the host program.
///
/// CQ atom arguments remain positional against this declaration order.
///
/// Declaration order is the host layout. A host is a plain row source unless
/// it declares keyed reads ([`HostDecl`], IF-HostDecl); only declared reads
/// are answered natively.
///
/// ```text
/// RelationDecl ::= "relation" Ident "(" [ColumnDecl ("," ColumnDecl)* [","]] ")"
///                  ["=" "(" Expr ")"] HostDecl* ";"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct RelationDecl {
    pub relation_token: kw::relation,
    pub name: Ident,
    #[syn(parenthesized)]
    pub columns_paren: token::Paren,
    #[syn(in = columns_paren)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<ColumnDecl>,
    #[syn(optional, peek = Token![=])]
    pub value: Option<LeafValue>,
    #[parse(host::parse_hosts)]
    #[to_tokens(|tokens, hosts| quote::TokenStreamExt::append_all(tokens, hosts))]
    pub hosts: Vec<HostDecl>,
    pub semi_token: Token![;],
}

impl RelationDecl {
    /// The relation arity derived from its declared columns.
    pub fn arity(&self) -> usize {
        self.columns.len()
    }

    /// A result schema describes types, not a runtime input value.
    pub fn check_result(&self) -> syn::Result<()> {
        if self.value.is_some() {
            return Err(syn::Error::new_spanned(
                self,
                "result declarations cannot bind a host value",
            ));
        }
        if let Some(host) = self.hosts.first() {
            return Err(syn::Error::new_spanned(
                host,
                "result declarations cannot declare host reads",
            ));
        }
        Ok(())
    }
}

/// Intern authored domain spellings once in this declaration table.
///
/// Rust payloads remain opaque: only their token spelling is used as an
/// interning key, never their expression structure or meaning. Printed ids
/// are already recorded facts and are preserved without reinterpreting Rust.
pub fn intern_domains(declarations: &mut [RelationDecl]) {
    if declarations
        .iter()
        .flat_map(|declaration| &declaration.columns)
        .filter_map(|column| column.domain.as_ref())
        .all(|domain| domain.id.is_some())
    {
        return;
    }
    let mut identities = BTreeMap::new();
    let mut next = 0;
    for domain in declarations
        .iter()
        .flat_map(|declaration| &declaration.columns)
        .filter_map(|column| column.domain.as_ref())
    {
        if let Some(id) = domain.id {
            identities.insert(quote::ToTokens::to_token_stream(&domain.expression).to_string(), id);
            next = next.max(id.0 + 1);
        }
    }
    for domain in declarations
        .iter_mut()
        .flat_map(|declaration| &mut declaration.columns)
        .filter_map(|column| column.domain.as_mut())
    {
        if domain.id.is_none() {
            let spelling = quote::ToTokens::to_token_stream(&domain.expression).to_string();
            domain.id = Some(*identities.entry(spelling).or_insert_with(|| {
                let id = DomainId(next);
                next += 1;
                id
            }));
        }
    }
}

/// A leaf expression is evaluated once in the enclosing Rust environment.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct LeafValue {
    pub eq_token: Token![=],
    #[syn(parenthesized)]
    pub paren_token: token::Paren,
    #[syn(in = paren_token)]
    pub expression: syn::Expr,
}

use std::collections::{BTreeMap, BTreeSet};
/// A relation name applied to a possibly empty, optionally trailing-comma list.
///
/// ```text
/// Atom ::= Ident "(" [RustExpr ("," RustExpr)* [","]] ")"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Atom {
    pub relation: Ident,
    #[syn(parenthesized)]
    pub paren_token: token::Paren,
    #[syn(in = paren_token)]
    #[parse(CommaList::parse_terminated)]
    pub args: CommaList<syn::Expr>,
}

impl Atom {
    /// Bare variable arguments only. Expression dependencies are a separate
    /// question; callers must never use this iterator as an expression analysis.
    pub fn variables(&self) -> impl Iterator<Item = &Ident> {
        self.args.iter().filter_map(rust::variable)
    }
    pub fn is_normalized(&self) -> bool {
        let mut variables = BTreeSet::new();
        self.args.iter().all(|argument| {
            rust::variable(argument).map_or_else(
                || equality::literal(argument),
                |variable| variables.insert(symbol_name(variable)),
            )
        })
    }
}

/// An opaque, pure, total Rust condition placed by its free variables.
/// With `capture`, the condition is a query capture's `bool` (IF-Capture).
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::ToTokens)]
pub struct Filter {
    pub if_token: Token![if],
    pub capture: Option<capture::kw::capture>,
    pub condition: syn::Expr,
}

pub(crate) fn reject_guard_attributes(input: syn::parse::ParseStream<'_>) -> syn::Result<()> {
    if !input.peek(Token![#]) {
        return Ok(());
    }
    input.call(syn::Attribute::parse_outer)?;
    Err(input.error("attributes are not supported on guards or relational constraints"))
}

impl syn::parse::Parse for Filter {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        reject_guard_attributes(input)?;
        Ok(Self {
            if_token: input.parse()?,
            capture: capture::parse_reference(input)?,
            condition: input.parse()?,
        })
    }
}

impl Filter {
    /// The Rust condition a printer evaluates: a capture reference reads the
    /// host-once `bool` through its binder reference (IF-Capture).
    pub fn test(&self) -> syn::Expr {
        capture::realize_test(self.capture.as_ref(), &self.condition)
    }
}

/// Finite pure `IntoIterator` generation. Iterator order and duplicates are
/// unobservable; evaluation is an opaque step, like a scalar `let` binding.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct For {
    pub for_token: Token![for],
    #[parse(syn::Pat::parse_single)]
    pub pattern: syn::Pat,
    pub in_token: Token![in],
    pub expression: syn::Expr,
}

impl For {
    /// Scalar-shaped syntax for binding/scope analysis, not scalar semantics.
    pub fn binding(&self) -> Let {
        Let {
            let_token: Default::default(),
            pattern: self.pattern.clone(),
            annotation: None,
            eq_token: Default::default(),
            capture: None,
            expression: self.expression.clone(),
        }
    }
}

/// A scalar binding. With `capture`, the value is a copy of the named query
/// capture (IF-Capture); the expression is then that capture's name.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Let {
    pub let_token: Token![let],
    #[parse(syn::Pat::parse_single)]
    pub pattern: syn::Pat,
    #[syn(optional, peek = Token![:])]
    pub annotation: Option<TypeAnnotation>,
    pub eq_token: Token![=],
    #[parse(capture::parse_reference)]
    pub capture: Option<capture::kw::capture>,
    pub expression: syn::Expr,
}

impl Let {
    /// The Rust value a printer binds: a capture reference copies the
    /// host-once value out of its binder reference (IF-Capture).
    pub fn value(&self) -> syn::Expr {
        capture::realize_value(self.capture.as_ref(), &self.expression)
    }

    /// Carry a whole-variable annotation without interpreting Rust types.
    /// Destructuring field types and binding modes are inferred by rustc.
    pub fn binding_types(&self) -> Vec<(Ident, syn::Type)> {
        rust::bindings(&self.pattern)
            .into_iter()
            .map(|id| {
                let ty = match (&self.pattern, &self.annotation) {
                    (syn::Pat::Ident(pattern), Some(annotation))
                        if pattern.by_ref.is_none() && pattern.ident == id =>
                    {
                        annotation.ty.clone()
                    }
                    _ => syn::parse_quote!(_),
                };
                (id, ty)
            })
            .collect()
    }
}

/// The optional Rust type ascription on a CQ let binding.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct TypeAnnotation {
    pub colon_token: Token![:],
    pub ty: Type,
}

/// Formal rule IF-Input: Check declaration names and arity; preserve layout order.
pub fn check_input_declarations<'a>(
    inputs: impl IntoIterator<Item = &'a RelationDecl>,
) -> syn::Result<BTreeMap<String, &'a RelationDecl>> {
    let mut schema = BTreeMap::new();
    for input in inputs {
        if input.columns.is_empty() {
            return Err(syn::Error::new_spanned(
                input,
                format!("input relation `{}` must have positive arity", input.name),
            ));
        }

        let mut column_names = BTreeSet::new();
        for column in &input.columns {
            let name = symbol_name(&column.name);
            if !column_names.insert(name.clone()) {
                return Err(syn::Error::new_spanned(
                    &column.name,
                    format!(
                        "input relation `{}` declares column `{name}` more than once",
                        input.name
                    ),
                ));
            }
        }
        input.check_hosts()?;
        if schema.insert(symbol_name(&input.name), input).is_some() {
            return Err(syn::Error::new_spanned(
                &input.name,
                format!("input relation `{}` is declared more than once", input.name),
            ));
        }
    }
    Ok(schema)
}

pub mod algebra;
pub mod occurrence;
pub use occurrence::Occurrence;

/// Parse declarations while the next token starts a relation declaration.
pub fn parse_declarations(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<RelationDecl>> {
    let mut declarations = Vec::new();
    while input.peek(kw::relation) {
        declarations.push(input.parse()?);
    }
    Ok(declarations)
}

pub mod equality;
pub use equality::{Equality, EqualityValue};
