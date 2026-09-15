//! Independent source-to-target relations, owned by their compilation boundary.

pub(crate) mod canonical;
pub(crate) mod indexes;
pub(crate) mod ordered;
pub(crate) mod physical;
pub(crate) mod pipeline;
pub(crate) mod planned_join;
pub(crate) mod pull;
