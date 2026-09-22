//! Typed administrative callback state owned by this operation family.

use cq_ir::provider_protocol as protocol;
use protocol::{exact, kw};

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
