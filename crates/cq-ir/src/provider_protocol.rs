//! Macro protocols for physical realization and execution-state providers.
//!
//! Driver messages are continuations within a pass, not another retained query
//! language. State requests expand directly to Rust expressions.
//! External operation families may use `Request<T>` with their own typed task.
//! The common driver handles only Input, Index, and Lower. The separate `state`
//! protocol supplies expression operations to selected execution endpoints.

use proc_macro2::TokenStream;
use syn::{Expr, Ident, Pat, Path, Stmt, Token, token};

use crate::{Column, CommaList, SemicolonList, physical, rust_access_plan, symbol_name};

pub mod kw {
    syn::custom_keyword!(reply_to);
    syn::custom_keyword!(plan);
    syn::custom_keyword!(environment);
    syn::custom_keyword!(request);
    syn::custom_keyword!(input);
    syn::custom_keyword!(from);
    syn::custom_keyword!(index);
    syn::custom_keyword!(lower);
    syn::custom_keyword!(resume);
    syn::custom_keyword!(emitted);
    syn::custom_keyword!(result);
    syn::custom_keyword!(stored);
    syn::custom_keyword!(indexed);
    syn::custom_keyword!(region);
    syn::custom_keyword!(at);
    syn::custom_keyword!(operations);
    syn::custom_keyword!(pending);
    syn::custom_keyword!(continuations);
    syn::custom_keyword!(rows);
    syn::custom_keyword!(fields);
    syn::custom_keyword!(matches);
    syn::custom_keyword!(values);
    syn::custom_keyword!(output);
    syn::custom_keyword!(whole);
    syn::custom_keyword!(access);
    syn::custom_keyword!(witness);
    syn::custom_keyword!(row);
    syn::custom_keyword!(source);
    syn::custom_keyword!(item);
}

#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Request<T: syn::parse::Parse + quote::ToTokens = Task> {
    pub reply_to_token: kw::reply_to,
    #[syn(parenthesized)]
    pub reply_to_paren: token::Paren,
    #[syn(in = reply_to_paren)]
    pub reply_to: Path,
    #[syn(braced)]
    pub state_brace: token::Brace,
    #[syn(in = state_brace)]
    pub state: TokenStream,
    pub plan_token: kw::plan,
    #[syn(braced)]
    pub plan_brace: token::Brace,
    #[syn(in = plan_brace)]
    pub plan: physical::Module,
    pub environment_token: kw::environment,
    #[syn(braced)]
    pub environment_brace: token::Brace,
    #[syn(in = environment_brace)]
    pub environment: Environment,
    pub request_token: kw::request,
    #[syn(braced)]
    pub request_brace: token::Brace,
    #[syn(in = request_brace)]
    pub request: T,
}

#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Task {
    #[parse(peek = kw::input)]
    Input {
        input_token: kw::input,
        name: Ident,
        from_token: kw::from,
        #[syn(parenthesized)]
        source_paren: token::Paren,
        #[syn(in = source_paren)]
        source: Expr,
    },
    #[parse(peek = kw::index)]
    Index { index_token: kw::index, name: Ident },
    #[parse(peek = kw::lower)]
    Lower { lower_token: kw::lower, name: Ident },
}

impl Task {
    pub fn name(&self) -> &Ident {
        match self {
            Self::Input { name, .. } | Self::Index { name, .. } | Self::Lower { name, .. } => name,
        }
    }
}

#[derive(Clone, Debug, Default, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Environment {
    #[syn(repeat)]
    pub bindings: Vec<Binding>,
}

impl Environment {
    pub fn relation(&self, name: &Ident) -> Option<&Binding> {
        self.bindings.iter().find(|binding| {
            !matches!(binding, Binding::Indexed { .. })
                && symbol_name(binding.name()) == symbol_name(name)
        })
    }
    pub fn index(&self, name: &Ident) -> Option<&Binding> {
        self.bindings.iter().find(|binding| {
            matches!(binding, Binding::Indexed { .. })
                && symbol_name(binding.name()) == symbol_name(name)
        })
    }
}

#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Binding {
    #[parse(peek = kw::stored)]
    Stored {
        stored_token: kw::stored,
        name: Ident,
        at_token: kw::at,
        #[syn(parenthesized)]
        value_paren: token::Paren,
        #[syn(in = value_paren)]
        value: Expr,
        operations_token: kw::operations,
        operations: Path,
    },
    #[parse(peek = kw::indexed)]
    Indexed {
        indexed_token: kw::indexed,
        name: Ident,
        at_token: kw::at,
        #[syn(parenthesized)]
        value_paren: token::Paren,
        #[syn(in = value_paren)]
        value: Expr,
        operations_token: kw::operations,
        operations: Path,
    },
    #[parse(peek = kw::region)]
    Region {
        region_token: kw::region,
        name: Ident,
        #[syn(braced)]
        access_brace: token::Brace,
        #[syn(in = access_brace)]
        access: rust_access_plan::Plan,
    },
}

impl Binding {
    pub fn name(&self) -> &Ident {
        match self {
            Self::Stored { name, .. } | Self::Indexed { name, .. } | Self::Region { name, .. } => {
                name
            }
        }
    }
}

#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Reply {
    pub resume_token: kw::resume,
    #[syn(braced)]
    pub resume_brace: token::Brace,
    #[syn(in = resume_brace)]
    pub resume: TokenStream,
    pub emitted_token: kw::emitted,
    #[syn(braced)]
    pub emitted_brace: token::Brace,
    #[syn(in = emitted_brace)]
    #[parse(syn::Block::parse_within)]
    #[to_tokens(|tokens, statements| quote::TokenStreamExt::append_all(tokens, statements))]
    pub emitted: Vec<Stmt>,
    pub result_token: kw::result,
    #[syn(braced)]
    pub result_brace: token::Brace,
    #[syn(in = result_brace)]
    pub result: Binding,
}

/// The common driver's serialized continuation, opaque to provider macros.
#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
pub struct State {
    pub plan_token: kw::plan,
    #[syn(braced)]
    pub plan_brace: token::Brace,
    #[syn(in = plan_brace)]
    pub plan: physical::Module,
    pub environment_token: kw::environment,
    #[syn(braced)]
    pub environment_brace: token::Brace,
    #[syn(in = environment_brace)]
    pub environment: Environment,
    pub emitted_token: kw::emitted,
    #[syn(braced)]
    pub emitted_brace: token::Brace,
    #[syn(in = emitted_brace)]
    #[parse(syn::Block::parse_within)]
    #[to_tokens(|tokens, statements| quote::TokenStreamExt::append_all(tokens, statements))]
    pub emitted: Vec<Stmt>,
    pub pending_token: kw::pending,
    #[syn(braced)]
    pub pending_brace: token::Brace,
    #[syn(in = pending_brace)]
    pub pending: Task,
    pub continuations_token: kw::continuations,
    #[syn(parenthesized)]
    pub continuations_paren: token::Paren,
    #[syn(in = continuations_paren)]
    pub resume_path: Path,
    #[syn(in = continuations_paren)]
    pub comma_token: Token![,],
    #[syn(in = continuations_paren)]
    pub access_path: Path,
}

/// An optional exact indexed binary-join family. Generic scheduling does not
/// inspect or require these operation messages.
pub mod exact {
    use super::*;

    #[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
    pub struct Value {
        #[syn(parenthesized)]
        pub paren: token::Paren,
        #[syn(in = paren)]
        pub value: Expr,
    }

    #[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
    pub struct Field {
        pub column: Column,
        pub eq_token: Token![=],
        #[syn(parenthesized)]
        pub value_paren: token::Paren,
        #[syn(in = value_paren)]
        pub value: Expr,
    }

    #[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
    pub struct Whole {
        pub source: Ident,
        pub eq_token: Token![=],
        #[syn(parenthesized)]
        pub value_paren: token::Paren,
        #[syn(in = value_paren)]
        pub value: Expr,
    }

    #[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
    pub struct Witness {
        pub row_token: kw::row,
        #[syn(parenthesized)]
        pub row_paren: token::Paren,
        #[syn(in = row_paren)]
        pub row: Expr,
        pub fields_token: kw::fields,
        #[syn(braced)]
        pub fields_brace: token::Brace,
        #[syn(in = fields_brace)]
        #[parse(SemicolonList::parse_terminated)]
        pub fields: SemicolonList<Field>,
    }

    #[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
    pub enum Task {
        #[parse(peek = kw::rows)]
        Rows { rows_token: kw::rows, name: Ident },
        #[parse(peek = kw::matches)]
        Matches {
            matches_token: kw::matches,
            name: Ident,
            values_token: kw::values,
            #[syn(bracketed)]
            values_bracket: token::Bracket,
            #[syn(in = values_bracket)]
            #[parse(CommaList::parse_terminated)]
            values: CommaList<Value>,
        },
        #[parse(peek = kw::output)]
        Output {
            output_token: kw::output,
            name: Ident,
            fields_token: kw::fields,
            #[syn(braced)]
            fields_brace: token::Brace,
            #[syn(in = fields_brace)]
            #[parse(SemicolonList::parse_terminated)]
            fields: SemicolonList<Field>,
            whole_token: kw::whole,
            #[syn(braced)]
            whole_brace: token::Brace,
            #[syn(in = whole_brace)]
            #[parse(SemicolonList::parse_terminated)]
            whole: SemicolonList<Whole>,
        },
    }

    /// A provider describes one iterable and how to decode its item. It does
    /// not introduce an Access binding or decide how the iterable is consumed.
    /// The selected RA rule composes these leaves into dependent bindings.
    #[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
    pub struct IteratorReply {
        pub resume_token: kw::resume,
        #[syn(braced)]
        pub resume_brace: token::Brace,
        #[syn(in = resume_brace)]
        pub resume: TokenStream,
        pub source_token: kw::source,
        #[syn(parenthesized)]
        pub source_paren: token::Paren,
        #[syn(in = source_paren)]
        pub source: Expr,
        pub item_token: kw::item,
        #[syn(parenthesized)]
        pub item_paren: token::Paren,
        #[syn(in = item_paren)]
        #[parse(Pat::parse_multi_with_leading_vert)]
        pub pattern: Pat,
        pub witness_token: kw::witness,
        #[syn(braced)]
        pub witness_brace: token::Brace,
        #[syn(in = witness_brace)]
        pub witness: Witness,
    }

    /// Output construction returns expressions only. Binding composition and
    /// execution remain the responsibility of the selected lowering.
    #[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
    pub struct WitnessReply {
        pub resume_token: kw::resume,
        #[syn(braced)]
        pub resume_brace: token::Brace,
        #[syn(in = resume_brace)]
        pub resume: TokenStream,
        pub witness_token: kw::witness,
        #[syn(braced)]
        pub witness_brace: token::Brace,
        #[syn(in = witness_brace)]
        pub witness: Witness,
    }
}

/// State operations used by selected execution endpoints.
pub mod state;
