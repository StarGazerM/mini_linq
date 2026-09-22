//! Private continuation state for exact operator lowering.

use cq_ir::{CommaList, provider_protocol as protocol, rust_access_plan};
use protocol::{exact, kw};
use syn::token;

#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
pub struct AccessState {
    pub request_token: kw::request,
    #[syn(braced)]
    pub request_brace: token::Brace,
    #[syn(in = request_brace)]
    pub request: protocol::Request,
    pub access_token: kw::access,
    #[syn(braced)]
    pub access_brace: token::Brace,
    #[syn(in = access_brace)]
    #[parse(CommaList::parse_terminated)]
    pub clauses: CommaList<rust_access_plan::Clause>,
    pub witness_token: kw::witness,
    #[syn(braced)]
    pub witness_brace: token::Brace,
    #[syn(in = witness_brace)]
    pub witness: exact::Witness,
}
