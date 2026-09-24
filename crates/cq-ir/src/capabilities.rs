//! Cursor operations a relation's representation admits, and their syntax.
//!
//! An operation is a claim with a law against the relation's abstraction
//! function: `seek @r[C](k) = σ_{C=k}`, `contains` its nonemptiness,
//! `ordered` a key-ordered `seek`, `iter` enumeration of the support.

use syn::{Ident, Token, token};

use crate::{Column, CommaList};

pub mod kw {
    syn::custom_keyword!(provides);
    syn::custom_keyword!(iter);
    syn::custom_keyword!(seek);
    syn::custom_keyword!(contains);
    syn::custom_keyword!(ordered);
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Provides {
    pub provides_token: kw::provides,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    #[parse(CommaList::parse_terminated)]
    pub capabilities: CommaList<Capability>,
}

/// One admitted cursor operation over a named relation.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Capability {
    #[parse(peek = kw::iter)]
    Iter { iter_token: kw::iter },
    #[parse(peek = kw::seek)]
    Seek {
        seek_token: kw::seek,
        descriptor: Descriptor,
    },
    #[parse(peek = kw::contains)]
    Contains {
        contains_token: kw::contains,
        descriptor: Descriptor,
    },
    #[parse(peek = kw::ordered)]
    Ordered {
        ordered_token: kw::ordered,
        descriptor: Descriptor,
    },
}

/// `@r[C]`: the IX index-requirement syntax, reused for capability keys.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Descriptor {
    pub at_token: Token![@],
    pub relation: Ident,
    pub columns: Columns,
}

#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Columns {
    #[syn(bracketed)]
    pub bracket: token::Bracket,
    #[syn(in = bracket)]
    #[parse(CommaList::parse_terminated)]
    pub columns: CommaList<Column>,
}

impl Columns {
    pub fn ordinals(&self) -> Vec<usize> {
        self.columns.iter().map(|c| c.index as usize).collect()
    }
}

/// A required or provided cursor operation, ordered by strength.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Operation {
    Iter,
    Contains(Vec<usize>),
    Seek(Vec<usize>),
    Ordered(Vec<usize>),
}

impl Operation {
    /// `ordered ⊒ seek ⊒ contains` on the same columns; `iter` is separate.
    pub fn satisfies(&self, required: &Operation) -> bool {
        match (self, required) {
            (Operation::Iter, Operation::Iter) => true,
            (
                Operation::Ordered(p),
                Operation::Ordered(r) | Operation::Seek(r) | Operation::Contains(r),
            )
            | (Operation::Seek(p), Operation::Seek(r) | Operation::Contains(r))
            | (Operation::Contains(p), Operation::Contains(r)) => p == r,
            _ => false,
        }
    }
}

pub fn operations(provides: &Provides) -> Vec<Operation> {
    provides
        .capabilities
        .iter()
        .map(|c| match c {
            Capability::Iter { .. } => Operation::Iter,
            Capability::Contains { descriptor, .. } => {
                Operation::Contains(descriptor.columns.ordinals())
            }
            Capability::Seek { descriptor, .. } => Operation::Seek(descriptor.columns.ordinals()),
            Capability::Ordered { descriptor, .. } => {
                Operation::Ordered(descriptor.columns.ordinals())
            }
        })
        .collect()
}
