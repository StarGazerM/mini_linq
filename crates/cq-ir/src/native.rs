//! Native access leaves and storage fragments within physical realization.
//!
//! These invocations occur inside the Rust program emitted from
//! RA or IndexRequirements; native expression leaves can expand after Access lowering. They
//! describe storage operations, not additional relational operators. The selected
//! implementation supplies the fragment and owes the operation's semantic law;
//! Rust checks its concrete types, expressions, statements and borrowing.
//!
//! ```text
//! Operation ::= type (RustType,*) | input (Expr) | len (Expr) | row (Expr)
//!              | field (Expr) (Column) | record (Expr,*) | key (Expr,*)
//!              | scan (Expr) | rows (Expr)
//!              | select (Expr) (Expr)
//!              | materialize Ident Ident (Expr)
//!              | produce Ident Ident via Ident { RustStatements }
//!              | pairs Ident in (Expr) by (Expr), Ident in (Expr) by (Expr) { RustStatements }
//!              | blocks (Expr) | bounds (Expr)
//!              | block_rows (Expr) (Expr)
//!              | build { AccessPlan }
//!              | contains (Expr) (Expr) | probe (Expr) (Expr)
//!              | may_match (Expr) (Expr) (Expr)
//!              | candidates Path { Expr } (Expr)
//! ```

use crate::{Column, CommaList};
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
    syn::custom_keyword!(build);
    syn::custom_keyword!(contains);
    syn::custom_keyword!(may_match);
    syn::custom_keyword!(probe);
    syn::custom_keyword!(candidates);
    syn::custom_keyword!(field);
    syn::custom_keyword!(record);
    syn::custom_keyword!(key);
    syn::custom_keyword!(produce);
    syn::custom_keyword!(via);
    syn::custom_keyword!(pairs);
    syn::custom_keyword!(by);
}

/// One traversal and its key in a native pair producer. The item binder is a
/// shared borrow of the traversal's item; no physical row layout is prescribed.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub struct PairInput {
    pub item: Ident,
    pub in_token: Token![in],
    #[syn(parenthesized)]
    pub source_paren: token::Paren,
    #[syn(in = source_paren)]
    pub source: Box<syn::Expr>,
    pub by_token: kw::by,
    #[syn(parenthesized)]
    pub key_paren: token::Paren,
    #[syn(in = key_paren)]
    pub key: Box<syn::Expr>,
}

/// Operations admitted by the supplied provider interface.
///
/// `Type` produces a Rust type; `Materialize` and `Produce` produce statements.
/// `Field` produces a Rust place expression; other variants produce expressions. Implementations may reject unsupported
/// variants. No successful parse asserts that a storage family supports blocks,
/// that membership is exact, or that bounds permit sound rejection.
#[derive(Clone, Debug, PartialEq, Eq, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Operation {
    /// Read a logical column using the selected row representation's syntax.
    #[parse(peek = kw::field)]
    Field {
        field_token: kw::field,
        #[syn(parenthesized)]
        row_paren: token::Paren,
        #[syn(in = row_paren)]
        row: Box<syn::Expr>,
        #[syn(parenthesized)]
        column_paren: token::Paren,
        #[syn(in = column_paren)]
        column: Column,
    },
    /// Construct a physical row from its logical field values.
    #[parse(peek = kw::record)]
    Record {
        record_token: kw::record,
        #[syn(parenthesized)]
        paren: token::Paren,
        #[syn(in = paren)]
        #[parse(CommaList::parse_terminated)]
        fields: CommaList<syn::Expr>,
    },
    /// Construct a key from borrowed logical fields. The key implementation
    /// chooses whether to retain those borrows or copy coordinates into a
    /// constructed resource representation; reading does not require Clone.
    #[parse(peek = kw::key)]
    Key {
        key_token: kw::key,
        #[syn(parenthesized)]
        paren: token::Paren,
        #[syn(in = paren)]
        #[parse(CommaList::parse_terminated)]
        fields: CommaList<syn::Expr>,
    },
    /// Construct a represented row set directly from calls to a local sink.
    /// The body may call `emit(row)` repeatedly; the provider owns distinctness.
    #[parse(peek = kw::produce)]
    Produce {
        produce_token: kw::produce,
        owner: Ident,
        value: Ident,
        via_token: kw::via,
        emit: Ident,
        body: syn::Block,
    },
    /// Visit each equal-key pair once, binding borrowed items in `body`.
    /// Traversal order, rescanning and resource construction belong to the
    /// selected algorithm. Equivalence requires stable sources and pure keys.
    #[parse(peek = kw::pairs)]
    Pairs {
        pairs_token: kw::pairs,
        left: PairInput,
        comma: Token![,],
        right: PairInput,
        body: syn::Block,
    },
    /// Input storage type for an ordered logical column schema. A selected
    /// representation must account for every stated type; width alone cannot
    /// establish that its physical records implement the relation schema.
    #[parse(peek = Token![type])]
    Type {
        type_token: Token![type],
        schema: syn::TypeTuple,
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
    /// Construct an access resource from the existing annotated query region.
    /// Its selected implementation determines the resource representation.
    #[parse(peek = kw::build)]
    Build {
        build_token: kw::build,
        #[syn(braced)]
        plan_brace: token::Brace,
        #[syn(in = plan_brace)]
        plan: Box<crate::rust_access_plan::Plan>,
    },
    /// Exact membership of a key in a borrowed resource. Keys may borrow rows.
    #[parse(peek = kw::contains)]
    Contains {
        contains_token: kw::contains,
        #[syn(parenthesized)]
        resource_paren: token::Paren,
        #[syn(in = resource_paren)]
        resource: Box<syn::Expr>,
        #[syn(parenthesized)]
        key_paren: token::Paren,
        #[syn(in = key_paren)]
        key: Box<syn::Expr>,
    },
    /// Conservative rejection using enclosing bounds and projected coordinates.
    #[parse(peek = kw::may_match)]
    MayMatch {
        may_match_token: kw::may_match,
        #[syn(parenthesized)]
        resource_paren: token::Paren,
        #[syn(in = resource_paren)]
        resource: Box<syn::Expr>,
        #[syn(parenthesized)]
        bounds_paren: token::Paren,
        #[syn(in = bounds_paren)]
        bounds: Box<syn::Expr>,
        #[syn(parenthesized)]
        columns_paren: token::Paren,
        #[syn(in = columns_paren)]
        columns: Box<syn::Expr>,
    },
    /// Enumerate precisely the witnesses for a key in a borrowed resource.
    #[parse(peek = kw::probe)]
    Probe {
        probe_token: kw::probe,
        #[syn(parenthesized)]
        resource_paren: token::Paren,
        #[syn(in = resource_paren)]
        resource: Box<syn::Expr>,
        #[syn(parenthesized)]
        key_paren: token::Paren,
        #[syn(in = key_paren)]
        key: Box<syn::Expr>,
    },
    /// Enumerate handles covering every row needed by the enclosing exact guard.
    ///
    /// A scan may ignore the predicate. Pruning may omit a block only if the
    /// supplied predicate, applied to its enclosing bounds, is false. The
    /// enclosing access region must establish that every omitted current row
    /// fails its exact residual predicate. This is a conditional access law,
    /// not a promise that candidate enumeration itself denotes the full view.
    #[parse(peek = kw::candidates)]
    Candidates {
        candidates_token: kw::candidates,
        provider: syn::Path,
        #[syn(braced)]
        value_brace: token::Brace,
        #[syn(in = value_brace)]
        value: Box<syn::Expr>,
        #[syn(parenthesized)]
        predicate_paren: token::Paren,
        #[syn(in = predicate_paren)]
        predicate: Box<syn::Expr>,
    },
}
