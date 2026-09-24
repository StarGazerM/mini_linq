//! MiniLinq's nominal query languages, from CQ down to the Access region.
//!
//! CQ, [`relational_plan::Module`], [`index_requirements::Module`], and
//! [`stream::Module`] each have one public parser and a local language
//! contract. RA gives every set operator a named DAG result; IX names the
//! attribute-keyed access each binary operator demands; Stream realizes every
//! result as a typed indexed-stream program whose modes and capabilities are
//! the first representation facts in the pipeline. Below Stream,
//! [`rust_access_plan::Plan`] is the erased, flattened region whose leaves are
//! Rust expressions over the runtime's traits, and the pull strategy lowers it
//! to an explicit [`iterator_pipeline::Pipeline`]. [`state_protocol`] is the
//! one runtime-call shape execution endpoints use for distinct and sink state.

use syn::ext::IdentExt;
use syn::{Ident, Index, Token};

pub mod cq;
pub mod index_requirements;
pub mod iterator_pipeline;
pub mod relational_plan;
pub mod rust_access_plan;
pub mod state_protocol;
pub mod stream;

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

/// The typed relation declarations supplied as inputs by the host program.
pub type InputList = Vec<cq::RelationDecl>;

/// A zero-based relation-column number written as a Rust integer literal.
pub type Column = Index;

/// Return the MiniLinq symbol represented by a Rust identifier.
///
/// In particular, `x` and the Rust raw spelling `r#x` return the same name.
pub fn symbol_name(identifier: &Ident) -> String {
    identifier.unraw().to_string()
}
