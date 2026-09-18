//! Sort both keyed inputs and merge equal runs, preserving pair multiplicity.
//! Sorting is explicit: no ordering claim is inferred from a provider's name.
use super::{Options, operands};
use cq_ir::native::Operation;
use proc_macro2::TokenStream;
use quote::quote;

pub fn compile(source: &Operation, options: &Options) -> syn::Result<TokenStream> {
    let (left, right, body) = operands(source)?;
    let (l, ls, lk) = (&left.item, &left.source, &left.key);
    let (r, rs, rk) = (&right.item, &right.source, &right.key);
    let observer = syn::Ident::new("__cq_work", proc_macro2::Span::mixed_site());
    let work = options
        .work
        .as_ref()
        .map(|value| quote!(let #observer = (#value);));
    let compared = options
        .work
        .as_ref()
        .map(|_| quote!(#observer.add(|w| w.exact_probes += 1);));
    let __cq_i = syn::Ident::new("__cq_i", proc_macro2::Span::mixed_site());
    let __cq_ie = syn::Ident::new("__cq_ie", proc_macro2::Span::mixed_site());
    let __cq_j = syn::Ident::new("__cq_j", proc_macro2::Span::mixed_site());
    let __cq_je = syn::Ident::new("__cq_je", proc_macro2::Span::mixed_site());
    let __cq_pair_left = syn::Ident::new("__cq_pair_left", proc_macro2::Span::mixed_site());
    let __cq_pair_right = syn::Ident::new("__cq_pair_right", proc_macro2::Span::mixed_site());
    let __cq_pair_row = syn::Ident::new("__cq_pair_row", proc_macro2::Span::mixed_site());
    Ok(quote!({
        #work
        let mut #__cq_pair_left = ::std::vec::Vec::new();
        let mut #__cq_pair_right = ::std::vec::Vec::new();
        for #__cq_pair_row in (#ls) {
            #[allow(unused_variables)]
            let #l = &#__cq_pair_row;
            #__cq_pair_left.push(((#lk), #__cq_pair_row));
        }
        for #__cq_pair_row in (#rs) {
            #[allow(unused_variables)]
            let #r = &#__cq_pair_row;
            #__cq_pair_right.push(((#rk), #__cq_pair_row));
        }
        #__cq_pair_left.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        #__cq_pair_right.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let (mut #__cq_i, mut #__cq_j) = (0usize, 0usize);
        while #__cq_i < #__cq_pair_left.len() && #__cq_j < #__cq_pair_right.len() {
            #compared
            match #__cq_pair_left[#__cq_i].0.cmp(&#__cq_pair_right[#__cq_j].0) {
                ::core::cmp::Ordering::Less => #__cq_i += 1,
                ::core::cmp::Ordering::Greater => #__cq_j += 1,
                ::core::cmp::Ordering::Equal => {
                    let mut #__cq_ie = #__cq_i + 1;
                    while #__cq_ie < #__cq_pair_left.len()
                        && #__cq_pair_left[#__cq_ie].0 == #__cq_pair_left[#__cq_i].0 { #__cq_ie += 1; }
                    let mut #__cq_je = #__cq_j + 1;
                    while #__cq_je < #__cq_pair_right.len()
                        && #__cq_pair_right[#__cq_je].0 == #__cq_pair_right[#__cq_j].0 { #__cq_je += 1; }
                    for (_, #l) in &#__cq_pair_left[#__cq_i..#__cq_ie] {
                        for (_, #r) in &#__cq_pair_right[#__cq_j..#__cq_je] { #body }
                    }
                    #__cq_i = #__cq_ie;
                    #__cq_j = #__cq_je;
                }
            }
        }
    }))
}
