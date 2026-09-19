//! Typed administrative callback state owned by this operation family.

use cq_ir::{CommaList, provider_protocol as protocol, rust_access_plan};
use protocol::{exact, kw};
use syn::token;

/// An implementation can play both a driver role and an exact-access role.
/// Each arm delegates to the shared grammar; it adds no task syntax.
#[derive(Clone, Debug, syn_derive::Parse, syn_derive::ToTokens)]
pub enum ProviderTask {
    #[parse(peek = kw::input)]
    Input(protocol::Task),
    #[parse(peek = kw::index)]
    Index(protocol::Task),
    #[parse(peek = kw::rows)]
    Rows(exact::Task),
    #[parse(peek = kw::matches)]
    Matches(exact::Task),
    #[parse(peek = kw::output)]
    Output(exact::Task),
}

pub type ProviderRequest = protocol::Request<ProviderTask>;

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
