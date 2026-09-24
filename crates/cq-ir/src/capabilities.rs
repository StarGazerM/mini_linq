//! Cursor operations a relation's representation admits, and their syntax.
//!
//! An operation is a claim with a law against the relation's abstraction
//! function: `seek @r[C](k) = σ_{C=k}`, `contains` its nonemptiness,
//! `ordered` enumeration in key order from any bound, `iter` enumeration of
//! the support. Each is one runtime trait (`IndexRead`, `ExactMembership`,
//! `OrderedRead`), so claims are exact: a representation provides an
//! operation iff it implements that trait, and no claim implies another.

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

/// `provides { seek, contains }`: the operations a built structure claims on
/// its own key columns. Bare names, since the key is the entry's.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Claims {
    pub provides_token: kw::provides,
    #[syn(braced)]
    pub brace: token::Brace,
    #[syn(in = brace)]
    #[parse(CommaList::parse_terminated)]
    pub claims: CommaList<Claim>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Claim {
    #[parse(peek = kw::seek)]
    Seek(kw::seek),
    #[parse(peek = kw::contains)]
    Contains(kw::contains),
    #[parse(peek = kw::ordered)]
    Ordered(kw::ordered),
}

impl Claim {
    pub fn name(self) -> &'static str {
        match self {
            Claim::Seek(_) => "seek",
            Claim::Contains(_) => "contains",
            Claim::Ordered(_) => "ordered",
        }
    }

    pub fn operation(self, columns: Vec<usize>) -> Operation {
        match self {
            Claim::Seek(_) => Operation::Seek(columns),
            Claim::Contains(_) => Operation::Contains(columns),
            Claim::Ordered(_) => Operation::Ordered(columns),
        }
    }
}

/// A required or provided cursor operation.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Operation {
    Iter,
    Contains(Vec<usize>),
    Seek(Vec<usize>),
    Ordered(Vec<usize>),
}

impl Operation {
    /// Whether performing this access proves the demand `required`: an exact
    /// `seek` or an `ordered` seek on the same columns witnesses
    /// membership. This is about accesses, not claims — a structure
    /// provides exactly what it claims.
    pub fn discharges(&self, required: &Operation) -> bool {
        match (self, required) {
            (
                Operation::Ordered(p) | Operation::Seek(p) | Operation::Contains(p),
                Operation::Contains(r),
            )
            | (Operation::Ordered(p) | Operation::Seek(p), Operation::Seek(r)) => p == r,
            (Operation::Ordered(p), Operation::Ordered(r)) => p == r,
            (Operation::Iter, Operation::Iter) => true,
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
