//! Supplied nominal languages for the YA compiler route.
use syn::ext::IdentExt;
use syn::{Ident, Index, Token};

pub mod cq;
pub mod index_requirements;
pub mod iterator_pipeline;
pub mod relational_plan;
pub mod rust_access_plan;

pub mod kw {
    syn::custom_keyword!(agg);
    syn::custom_keyword!(indexes);
    syn::custom_keyword!(relation);
}

pub type SyntaxList<Item, Separator> = syn::punctuated::Punctuated<Item, Separator>;
pub type CommaList<Item> = SyntaxList<Item, Token![,]>;
pub type SemicolonList<Item> = SyntaxList<Item, Token![;]>;
pub type InputList = Vec<cq::RelationDecl>;
pub type Column = Index;

pub fn symbol_name(identifier: &Ident) -> String {
    identifier.unraw().to_string()
}
