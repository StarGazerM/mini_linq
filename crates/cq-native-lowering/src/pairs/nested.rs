//! Rescan the RHS for every LHS item, without a witness table or row buffer.
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
    let __cq_pair_key = syn::Ident::new("__cq_pair_key", proc_macro2::Span::mixed_site());
    let __cq_pair_left = syn::Ident::new("__cq_pair_left", proc_macro2::Span::mixed_site());
    let __cq_pair_right = syn::Ident::new("__cq_pair_right", proc_macro2::Span::mixed_site());
    Ok(quote!({
        #work
        for #__cq_pair_left in (#ls) {
            #[allow(unused_variables)]
            let #l = &#__cq_pair_left;
            let #__cq_pair_key = (#lk);
            for #__cq_pair_right in (#rs) {
                #[allow(unused_variables)]
            let #r = &#__cq_pair_right;
                #compared
                if #__cq_pair_key == (#rk) { #body }
            }
        }
    }))
}
