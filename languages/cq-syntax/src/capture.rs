//! Query-closed Rust values bound once on the host.
//!
//! CQ normalization records each query-closed `let`/`if` here (formal rule
//! A-Capture); every later language carries the list unchanged. A capture is
//! never a row column: its binders are host names, `if` captures gate the
//! whole answer, and expressions keep their written syntax.
//!
//! ```text
//! Captures ::= ["capture" "{" (Let ";" | "if" RustExpr ";" | DomainCapture ";")* "}"]
//! ```
use crate::{CodeDomain, Filter, Let, rust, symbol_name};
use quote::ToTokens;
use std::borrow::Cow;
use std::collections::BTreeSet;
use syn::{Ident, Token, token};

pub mod kw {
    syn::custom_keyword!(capture);
    syn::custom_keyword!(domain);
    syn::custom_keyword!(exact);
    syn::custom_keyword!(lower);
    syn::custom_keyword!(upper);
}

/// IF-Capture: `capture c` in a binding or guard value position
/// names query capture `c`; the binder copies (a guard reads) its host value.
pub fn parse_reference(input: syn::parse::ParseStream<'_>) -> syn::Result<Option<kw::capture>> {
    if input.peek(kw::capture) && input.peek2(Ident) {
        let marker: kw::capture = input.parse()?;
        let fork = input.fork();
        if fork.parse::<Ident>().is_ok() && !fork.peek(token::Paren) && !fork.peek(Token![.]) {
            return Ok(Some(marker));
        }
        return Err(input.error("a capture reference names one capture binder"));
    }
    Ok(None)
}

/// Printer realization of a capture-referencing binding value: a copy of the
/// host value through its binder reference.
pub fn realize_value(reference: Option<&kw::capture>, expression: &syn::Expr) -> syn::Expr {
    match reference {
        Some(_) => syn::parse_quote!(::core::clone::Clone::clone(#expression)),
        None => expression.clone(),
    }
}

/// Printer realization of a capture-referencing guard: the host `bool`.
pub fn realize_test(reference: Option<&kw::capture>, condition: &syn::Expr) -> syn::Expr {
    match reference {
        Some(_) => syn::parse_quote!(*#condition),
        None => condition.clone(),
    }
}

/// WF-Capture: a capture reference names a `let` capture binder.
pub fn check_reference(
    reference: Option<&kw::capture>,
    expression: &syn::Expr,
    captures: &Captures,
) -> syn::Result<()> {
    if reference.is_some()
        && !rust::variable(expression).is_some_and(|name| captures.contains(&symbol_name(name)))
    {
        return Err(syn::Error::new_spanned(
            expression,
            "a capture reference names a capture binder",
        ));
    }
    Ok(())
}

/// A-Domain's recorded operation, never inferred from a Rust call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomainFunction {
    Exact,
    Lower,
    Upper,
}

impl syn::parse::Parse for DomainFunction {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(kw::exact) {
            input.parse::<kw::exact>()?;
            Ok(Self::Exact)
        } else if input.peek(kw::lower) {
            input.parse::<kw::lower>()?;
            Ok(Self::Lower)
        } else {
            input.parse::<kw::upper>()?;
            Ok(Self::Upper)
        }
    }
}

impl ToTokens for DomainFunction {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::Exact => kw::exact::default().to_tokens(tokens),
            Self::Lower => kw::lower::default().to_tokens(tokens),
            Self::Upper => kw::upper::default().to_tokens(tokens),
        }
    }
}

/// Explicit IR syntax preserves A-Domain provenance across serialization.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct DomainCapture {
    pub domain_token: kw::domain,
    pub name: Ident,
    pub colon_token: Token![:],
    pub ty: syn::Type,
    pub domain: CodeDomain,
    pub eq_token: Token![=],
    pub function: DomainFunction,
    #[syn(parenthesized)]
    pub paren_token: token::Paren,
    #[syn(in = paren_token)]
    pub operand: syn::Expr,
}

impl DomainCapture {
    /// Realize the recorded operation without recognizing Rust syntax.
    pub fn binding(&self) -> Let {
        let Self {
            name,
            ty,
            domain,
            function,
            operand,
            ..
        } = self;
        let domain = &domain.expression;
        syn::parse_quote!(let #name: #ty = ::cq_storage::domain::#function(&(#domain), &(#operand)))
    }
}

/// One host-once value: a binding or an answer gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Capture {
    Let(Box<Let>),
    Guard(Filter),
    Domain(Box<DomainCapture>),
}

impl Capture {
    fn reference(&self) -> Option<&kw::capture> {
        match self {
            Self::Let(binding) => binding.capture.as_ref(),
            Self::Guard(guard) => guard.capture.as_ref(),
            Self::Domain(_) => None,
        }
    }

    pub fn expressions(&self) -> impl Iterator<Item = &syn::Expr> {
        let expressions = match self {
            Self::Let(binding) => [Some(&binding.expression), None],
            Self::Guard(guard) => [Some(&guard.condition), None],
            Self::Domain(capture) => [Some(&capture.domain.expression), Some(&capture.operand)],
        };
        expressions.into_iter().flatten()
    }

    /// A binding ready for a Rust printer; the typed record remains unchanged.
    pub fn binding(&self) -> Option<Cow<'_, Let>> {
        match self {
            Self::Let(binding) => Some(Cow::Borrowed(binding)),
            Self::Domain(capture) => Some(Cow::Owned(capture.binding())),
            Self::Guard(_) => None,
        }
    }

    pub fn binding_types(&self) -> Vec<(Ident, syn::Type)> {
        match self {
            Self::Let(binding) => binding.binding_types(),
            Self::Domain(capture) => vec![(capture.name.clone(), capture.ty.clone())],
            Self::Guard(_) => Vec::new(),
        }
    }

    pub fn binders(&self) -> Vec<Ident> {
        match self {
            Self::Let(binding) => rust::bindings(&binding.pattern),
            Self::Guard(_) => Vec::new(),
            Self::Domain(capture) => vec![capture.name.clone()],
        }
    }
}

impl syn::parse::Parse for Capture {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if input.peek(kw::domain) {
            Ok(Self::Domain(input.parse()?))
        } else if input.peek(Token![let]) {
            Ok(Self::Let(input.parse()?))
        } else {
            Ok(Self::Guard(input.parse()?))
        }
    }
}

impl ToTokens for Capture {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        match self {
            Self::Let(binding) => binding.to_tokens(tokens),
            Self::Guard(guard) => guard.to_tokens(tokens),
            Self::Domain(capture) => capture.to_tokens(tokens),
        }
    }
}

/// The query's captures in dependency order; empty prints nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Captures {
    pub items: Vec<Capture>,
}

impl Captures {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Host names bound by `let` captures.
    pub fn binders(&self) -> impl Iterator<Item = Ident> + '_ {
        self.items.iter().flat_map(Capture::binders)
    }

    pub fn names(&self) -> BTreeSet<String> {
        self.binders().map(|id| symbol_name(&id)).collect()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.binders().any(|id| symbol_name(&id) == name)
    }

    /// Formal rule WF-Capture: binders are distinct, disjoint from the query's
    /// logical names, and every expression reads no logical name.
    pub fn check(&self, logical: &BTreeSet<String>) -> syn::Result<()> {
        let mut seen = BTreeSet::new();
        for item in &self.items {
            if item.reference().is_some()
                || item.expressions().any(|expression| {
                    !rust::free_variables(expression)
                        .dependencies(logical)
                        .is_empty()
                })
            {
                return Err(syn::Error::new_spanned(
                    item,
                    "a capture reads no query variable and is written Rust",
                ));
            }
            for binder in item.binders() {
                let name = symbol_name(&binder);
                if logical.contains(&name) || !seen.insert(name) {
                    return Err(syn::Error::new_spanned(
                        binder,
                        "capture binders are distinct host names, not query variables",
                    ));
                }
            }
        }
        Ok(())
    }
}

impl syn::parse::Parse for Captures {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        if !input.peek(kw::capture) {
            return Ok(Self::default());
        }
        input.parse::<kw::capture>()?;
        let content;
        syn::braced!(content in input);
        let mut items = Vec::new();
        while !content.is_empty() {
            items.push(content.parse()?);
            content.parse::<Token![;]>()?;
        }
        Ok(Self { items })
    }
}

impl ToTokens for Captures {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        if self.items.is_empty() {
            return;
        }
        kw::capture::default().to_tokens(tokens);
        token::Brace::default().surround(tokens, |tokens| {
            for item in &self.items {
                item.to_tokens(tokens);
                <Token![;]>::default().to_tokens(tokens);
            }
        });
    }
}
