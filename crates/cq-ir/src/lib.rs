//! MiniLinq's nominal query languages, from CQ down to the Structures module.
//!
//! CQ, [`relational_plan::Module`], [`index_requirements::Module`],
//! [`physical::Module`] and [`structures::Module`] each have one public
//! parser and a local language contract. RA gives every set operator a named
//! DAG result; IX names the attribute-keyed access each binary operator
//! demands; Physical fixes one physical operator per result and the mode
//! (pipe, let, alias) its reads allow; Structures binds every keyed read to
//! a host key fact or a Rust type built over the result. Below Structures a
//! printer emits Rust; [`state_protocol`] is the one runtime-call shape
//! execution endpoints use for distinct and sink state.
//! [`rust_access_plan::Plan`] and [`iterator_pipeline::Pipeline`] remain as
//! hand-staged entry points into the endpoints.

use syn::ext::IdentExt;
use syn::{Ident, Index, Token};

pub mod cq;
pub mod index_requirements;
pub mod iterator_pipeline;
pub mod physical;
pub mod policy;
pub mod relational_plan;
pub mod rust_access_plan;
pub mod state_protocol;
pub mod structures;

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
