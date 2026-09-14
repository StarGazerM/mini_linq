//! MiniLinq's nominal query languages and Rust-shaped physical representations.
//!
//! CQ, [`relational_plan::Module`], and IndexRequirements each have one public
//! parser and local language contract. The relational plan gives every set
//! operator a named DAG result before indexes or Rust storage are chosen.
//! [`physical_provider`] declares native Access fragments.
//! [`rust_access_plan::Plan`] then associates every logical occurrence with
//! its concrete Rust access or output expression. The pull strategy lowers
//! those annotations to an explicit
//! [`iterator_pipeline::Pipeline`]; the next pass returns the actual ordinary
//! Rust expression for one lazy result-set iterator. There is no parallel
//! object-builder or storage-descriptor hierarchy.

use syn::ext::IdentExt;
use syn::{Ident, Index, Token};

pub mod cq;
pub mod index_requirements;
pub mod iterator_pipeline;
pub mod physical_provider;
pub mod relational_plan;
pub mod rust_access_plan;

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
