//! Exact semijoin refinement for indexed tuple sets.
//!
//! Keep a left row iff its shared key has a right witness. With no shared
//! attributes this is a right nonemptiness guard. Right payloads are never
//! enumerated, so witness multiplicity cannot multiply the retained rows.
use super::query::{Observation, scan};
use cq_ir::rust_access_plan as access;
use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

pub(super) fn lower(
    left: &Ident,
    left_variables: &[Ident],
    right: &Ident,
    key: &[Ident],
    index: Option<(&Ident, bool)>,
    observation: Observation,
) -> syn::Result<(TokenStream, Vec<access::Clause>)> {
    let mut bindings = quote!(let __left = &#left;);
    let mut clauses = vec![scan(left_variables, quote!(__left))];
    let exists = match index {
        None => {
            bindings.extend(quote!(let __right = &#right;));
            quote!(!__right.is_empty())
        }
        Some((index, payload)) => {
            bindings.extend(quote!(let __index = &#index;));
            let fields = key.iter().map(|v| quote!(::core::clone::Clone::clone(#v)));
            let value = quote!((#(#fields,)*));
            if payload {
                quote!(__index.contains_key(&#value))
            } else {
                quote!(__index.contains(&#value))
            }
        }
    };
    let condition = match observation {
        Observation::Silent => exists,
        Observation::Count => {
            bindings.extend(quote!(let __probes_ref = &__probes;));
            quote!({ __probes_ref.set(__probes_ref.get()+1); #exists })
        }
    };
    clauses.push(syn::parse2(quote!(key(#(#key),*) => if (#condition)))?);
    Ok((bindings, clauses))
}
