//! Expression requests for an explicitly selected distinct-state provider.
//!
//! `operator` identifies the local operator so a caller policy can dispatch per
//! occurrence. Each expansion is a Rust expression. Distinct uses `new` and
//! `insert(state, &row) -> bool`: true exactly on the first equal value, without
//! consuming the row — the insertion law of a canonical index. Providers own
//! representation, equality strategy and any retained copies.
//! State starts empty; operations inspect no future input and preserve value
//! equality. Selecting a path is an obligation to satisfy these laws, not proof.

use syn::{Expr, Ident, Token, token};

pub mod kw {
    syn::custom_keyword!(operator);
    syn::custom_keyword!(new);
    syn::custom_keyword!(insert);
}

#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
pub struct Request {
    pub operator_token: kw::operator,
    pub operator: Ident,
    pub semi_token: Token![;],
    pub task: Task,
}

#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
pub enum Task {
    #[parse(peek = kw::new)]
    New { new_token: kw::new },
    #[parse(peek = kw::insert)]
    Insert {
        insert_token: kw::insert,
        #[syn(parenthesized)]
        paren: token::Paren,
        #[syn(in = paren)]
        state: Box<Expr>,
        #[syn(in = paren)]
        comma: Token![,],
        #[syn(in = paren)]
        value: Box<Expr>,
    },
}
