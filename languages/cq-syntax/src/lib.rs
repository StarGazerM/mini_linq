//! Shared syntax, declarations, Rust expression helpers and predicate recognition.

use syn::ext::IdentExt;
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
    identifier.unraw().to_string()
}

pub mod predicate;
pub mod rust;

/// One named, typed column in a declared input relation.
///
/// ```text
/// ColumnDecl ::= Ident ":" Type
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct ColumnDecl {
    pub name: Ident,
    pub colon_token: Token![:],
    pub ty: Type,
}

/// One typed relation declaration supplied by the host program.
///
/// CQ atom arguments remain positional against this declaration order.
///
/// An `#[index(…)]` attribute is advice for structure selection: whichever
/// structures a later pass builds over this relation use the Rust type it
/// names, with the operations it claims. It is not part of the query's
/// meaning and names no columns — which columns get indexed is a planner
/// decision the query cannot see. CQ checks only that the attribute is a
/// well-formed `index(…)` list; its argument belongs to the pass that reads it.
///
/// ```text
/// RelationDecl ::= ["#" "[" "index" "(" … ")" "]"]
///                  "relation" Ident "(" [ColumnDecl ("," ColumnDecl)* [","]] ")" ";"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct RelationDecl {
    #[parse(syn::Attribute::parse_outer)]
    #[to_tokens(|tokens, attrs| quote::TokenStreamExt::append_all(tokens, attrs))]
    pub attrs: Vec<syn::Attribute>,
    pub relation_token: kw::relation,
    pub name: Ident,
    #[syn(parenthesized)]
    pub columns_paren: token::Paren,
    #[syn(in = columns_paren)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<ColumnDecl>,
    pub semi_token: Token![;],
}

impl RelationDecl {
    /// The relation arity derived from its declared columns.
    pub fn arity(&self) -> usize {
        self.columns.len()
    }

    /// The structure advice attached to this relation, if any. Shape is the
    /// CQ contract's concern; the argument is read by the pass that consults
    /// it.
    pub fn index_advice(&self) -> syn::Result<Option<&syn::Attribute>> {
        let mut advice = None;
        for attr in &self.attrs {
            if !attr.path().is_ident("index") {
                return Err(syn::Error::new_spanned(
                    attr,
                    "relation declarations accept only `#[index(…)]`",
                ));
            }
            attr.meta.require_list()?;
            if advice.is_some() {
                return Err(syn::Error::new_spanned(
                    attr,
                    format!("relation `{}` advises an index twice", self.name),
                ));
            }
            advice = Some(attr);
        }
        Ok(advice)
    }
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
        self.variables().count() == self.args.len()
            && self
                .variables()
                .map(symbol_name)
                .collect::<BTreeSet<_>>()
                .len()
                == self.args.len()
    }
}

/// An embedded Rust condition. Parsing does not establish reorderability.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Filter {
    pub if_token: Token![if],
    pub condition: syn::Expr,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Let {
    pub let_token: Token![let],
    #[parse(syn::Pat::parse_single)]
    pub pattern: syn::Pat,
    #[syn(optional, peek = Token![:])]
    pub annotation: Option<TypeAnnotation>,
    pub eq_token: Token![=],
    pub expression: syn::Expr,
}

impl Let {
    /// Written binding types; unannotated bindings retain Rust type inference.
    pub fn binding_types(&self) -> Vec<(Ident, syn::Type)> {
        let mut vars: Vec<_> = rust::bindings(&self.pattern)
            .into_iter()
            .map(|id| (id, syn::parse_quote!(_)))
            .collect();
        if let Some(annotation) = &self.annotation {
            if vars.len() == 1 && matches!(self.pattern, syn::Pat::Ident(_)) {
                vars[0].1 = annotation.ty.clone();
            }
        }
        vars
    }
}

/// The optional Rust type ascription on a CQ let binding.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct TypeAnnotation {
    pub colon_token: Token![:],
    pub ty: Type,
}

pub fn check_input_declarations<'a>(
    inputs: impl IntoIterator<Item = &'a RelationDecl>,
) -> syn::Result<BTreeMap<String, &'a RelationDecl>> {
    let mut schema = BTreeMap::new();
    for input in inputs {
        input.index_advice()?;
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

/// Parse declarations while the next token starts a relation declaration.
pub fn parse_declarations(input: syn::parse::ParseStream<'_>) -> syn::Result<Vec<RelationDecl>> {
    let mut declarations = Vec::new();
    while input.peek(syn::Token![#]) || input.peek(kw::relation) {
        declarations.push(input.parse()?);
    }
    Ok(declarations)
}
