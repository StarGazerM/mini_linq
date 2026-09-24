//! Expression requests for explicitly selected distinct-state and set-sink providers.
//!
//! `operator` identifies the local operator so a caller policy can dispatch per
//! occurrence. Each expansion is a Rust expression. Distinct uses `new` and
//! `insert(state, &row) -> bool`: true exactly on the first equal value, without
//! consuming the row. A set sink uses `new`, `insert(state, row)` and
//! `finish(state)`: insertion consumes a value and finish returns its exact set.
//! Providers own representation, equality strategy and any retained copies.
//! State starts empty; operations inspect no future input and preserve value
//! equality. Selecting a path is an obligation to satisfy these laws, not proof.

use syn::{Expr, Ident, Token, token};

pub mod kw {
    syn::custom_keyword!(operator);
    syn::custom_keyword!(new);
    syn::custom_keyword!(insert);
    syn::custom_keyword!(finish);
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
        state: Expr,
        #[syn(in = paren)]
        comma: Token![,],
        #[syn(in = paren)]
        value: Expr,
    },
    #[parse(peek = kw::finish)]
    Finish {
        finish_token: kw::finish,
        #[syn(parenthesized)]
        paren: token::Paren,
        #[syn(in = paren)]
        state: Expr,
    },
}
