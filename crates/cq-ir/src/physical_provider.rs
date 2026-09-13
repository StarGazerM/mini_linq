//! Contextual provider fragments within physical realization.
//!
//! These invocations occur inside the Rust context emitted from
//! IndexRequirements; native expression leaves can expand after Access lowering. They
//! describe storage operations, not additional relational operators. The selected
//! implementation supplies the fragment and owes the operation's semantic law;
//! Rust checks its concrete types, expressions, statements and borrowing.
//!
//! ```text
//! Operation ::= type Column | input (Expr) | len (Expr) | row (Expr)
//!              | scan (Expr) | rows (Expr)
//!              | select (Expr) (Expr)
//!              | materialize Ident Ident (Expr) (Expr)
//!              | blocks (Expr) | bounds (Expr)
//!              | block_rows (Expr) (Expr)
//! ```

use crate::Column;
use syn::{Ident, Token, token};

pub mod kw {
    syn::custom_keyword!(input);
    syn::custom_keyword!(len);
    syn::custom_keyword!(row);
    syn::custom_keyword!(scan);
    syn::custom_keyword!(rows);
    syn::custom_keyword!(select);
    syn::custom_keyword!(materialize);
    syn::custom_keyword!(blocks);
    syn::custom_keyword!(bounds);
    syn::custom_keyword!(block_rows);
}

/// Operations admitted by the supplied provider interface.
///
/// `Type` produces a Rust type, `Materialize` a statement sequence, and every
/// other variant a Rust expression. Implementations may reject unsupported
/// variants. No successful parse asserts that a storage family supports blocks,
/// that membership is exact, or that bounds permit sound rejection.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Operation {
    /// Input storage type for the stated row width.
    #[parse(peek = Token![type])]
    Type {
        type_token: Token![type],
        arity: Column,
    },
    /// Represent all input rows, preserving their physical layout.
    #[parse(peek = kw::input)]
    Input {
        input_token: kw::input,
        #[syn(parenthesized)]
        value_paren: token::Paren,
        #[syn(in = value_paren)]
        value: Box<syn::Expr>,
    },
    /// Cardinality of the represented set.
    #[parse(peek = kw::len)]
    Len {
        len_token: kw::len,
        #[syn(parenthesized)]
        value_paren: token::Paren,
        #[syn(in = value_paren)]
        value: Box<syn::Expr>,
    },
    /// Decode a valid handle without changing its identity or row layout.
    #[parse(peek = kw::row)]
    Row {
        row_token: kw::row,
        #[syn(parenthesized)]
        value_paren: token::Paren,
        #[syn(in = value_paren)]
        value: Box<syn::Expr>,
    },
    /// Enumerate exactly the current selected rows.
    #[parse(peek = kw::scan)]
    Scan {
        scan_token: kw::scan,
        #[syn(parenthesized)]
        value_paren: token::Paren,
        #[syn(in = value_paren)]
        value: Box<syn::Expr>,
    },
    /// Enumerate exactly the valid handles of the current selected rows.
    #[parse(peek = kw::rows)]
    Rows {
        rows_token: kw::rows,
        #[syn(parenthesized)]
        value_paren: token::Paren,
        #[syn(in = value_paren)]
        value: Box<syn::Expr>,
    },
    /// Represent exactly the supplied current handles, preserving prior values.
    #[parse(peek = kw::select)]
    Select {
        select_token: kw::select,
        #[syn(parenthesized)]
        value_paren: token::Paren,
        #[syn(in = value_paren)]
        value: Box<syn::Expr>,
        #[syn(parenthesized)]
        rows_paren: token::Paren,
        #[syn(in = rows_paren)]
        rows: Box<syn::Expr>,
    },
    /// Bind storage and its represented value from an exact distinct row set.
    #[parse(peek = kw::materialize)]
    Materialize {
        materialize_token: kw::materialize,
        owner: Ident,
        value: Ident,
        #[syn(parenthesized)]
        rows_paren: token::Paren,
        #[syn(in = rows_paren)]
        rows: Box<syn::Expr>,
        #[syn(parenthesized)]
        block_size_paren: token::Paren,
        #[syn(in = block_size_paren)]
        block_size: Box<syn::Expr>,
    },
    /// Enumerate current nonempty descriptors without opening payload.
    #[parse(peek = kw::blocks)]
    Blocks {
        blocks_token: kw::blocks,
        #[syn(parenthesized)]
        value_paren: token::Paren,
        #[syn(in = value_paren)]
        value: Box<syn::Expr>,
    },
    /// Read enclosing coordinate bounds without opening payload.
    #[parse(peek = kw::bounds)]
    Bounds {
        bounds_token: kw::bounds,
        #[syn(parenthesized)]
        descriptor_paren: token::Paren,
        #[syn(in = descriptor_paren)]
        descriptor: Box<syn::Expr>,
    },
    /// Enumerate current handles in this descriptor, opening only its payload.
    #[parse(peek = kw::block_rows)]
    BlockRows {
        block_rows_token: kw::block_rows,
        #[syn(parenthesized)]
        descriptor_paren: token::Paren,
        #[syn(in = descriptor_paren)]
        descriptor: Box<syn::Expr>,
        #[syn(parenthesized)]
        value_paren: token::Paren,
        #[syn(in = value_paren)]
        value: Box<syn::Expr>,
    },
}
