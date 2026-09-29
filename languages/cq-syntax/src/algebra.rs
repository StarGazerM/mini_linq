//! Operator details shared by relational and physical programs.
use crate::CommaList;
use syn::{Ident, Token, token};
pub mod kw {
    syn::custom_keyword!(keep);
    syn::custom_keyword!(project);
    syn::custom_keyword!(rename);
    syn::custom_keyword!(unit);
    syn::custom_keyword!(select);
    syn::custom_keyword!(with);
    syn::custom_keyword!(extend);
    syn::custom_keyword!(value);
}
/// The singleton zero-column relation used to seed a global logical clause.
///
/// ```text
/// Unit ::= "unit"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Unit {
    pub unit_token: kw::unit,
}

/// One source-attribute to query-variable rename.
///
/// ```text
/// AttributeRename ::= Ident "->" Ident
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct AttributeRename {
    pub source: Ident,
    pub minus_token: Token![-],
    pub gt_token: Token![>],
    pub target: Ident,
}

/// A complete one-to-one rename of one declared relation occurrence.
///
/// ```text
/// Rename ::= "rename" RelationName "{"
///              [AttributeRename ("," AttributeRename)* [","]]
///            "}"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Rename {
    pub rename_token: kw::rename,
    pub relation: Ident,
    #[syn(braced)]
    pub mapping_brace: token::Brace,
    #[syn(in = mapping_brace)]
    #[parse(CommaList::parse_terminated)]
    pub mapping: CommaList<AttributeRename>,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Select {
    pub select_token: kw::select,
    pub input: Ident,
    pub if_token: Token![if],
    #[syn(parenthesized)]
    pub condition_paren: token::Paren,
    #[syn(in = condition_paren)]
    pub condition: syn::Expr,
}

/// Compute one tuple of fresh attributes for each input tuple. Rust owns the
/// expression and its types; `_` permits local Rust inference for a binding.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Extend {
    pub extend_token: kw::extend,
    pub input: Ident,
    pub with_token: kw::with,
    #[syn(braced)]
    pub columns_brace: token::Brace,
    #[syn(in = columns_brace)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<crate::ColumnDecl>,
    pub value_token: kw::value,
    #[syn(parenthesized)]
    pub value_paren: token::Paren,
    #[syn(in = value_paren)]
    pub value: syn::Expr,
}

/// Set projection of an earlier result to a distinct subset of attributes.
///
/// The attribute braces denote a set. Ordered output tuple layout belongs to
/// the separate output metadata.
///
/// ```text
/// Project ::= "project" RelationId "keep" "{"
///               [Ident ("," Ident)* [","]]
///             "}"
/// ```
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Project {
    pub project_token: kw::project,
    pub input: Ident,
    pub keep_token: kw::keep,
    #[syn(braced)]
    pub attributes_brace: token::Brace,
    #[syn(in = attributes_brace)]
    #[parse(CommaList::parse_terminated)]
    pub attributes: CommaList<Ident>,
}
