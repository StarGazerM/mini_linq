//! Exercise 2: compile exact semijoin without enumerating right witnesses.
use super::query::Observation;
use cq_ir::rust_access_plan as access;
use proc_macro2::TokenStream;
use syn::Ident;

pub(super) fn lower(
    left: &Ident,
    left_variables: &[Ident],
    right: &Ident,
    key: &[Ident],
    index: Option<(&Ident, bool)>,
    observation: Observation,
) -> syn::Result<(TokenStream, Vec<access::Clause>)> {
    let _ = (left_variables, right, key, index, observation);
    // Build bindings and Access clauses. query::scan is supplied.
    // index=None means an empty shared key; Some((id,true)) has payload buckets.
    Err(syn::Error::new_spanned(left, "semijoin lowering exercise"))
}
