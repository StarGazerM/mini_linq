//! Build the RHS hash buckets before probing with LHS rows.
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
    let built = options
        .work
        .as_ref()
        .map(|_| quote!(#observer.add(|w| w.index_rows += 1);));
    let probed = options
        .work
        .as_ref()
        .map(|_| quote!(#observer.add(|w| w.exact_probes += 1);));
    let __cq_pair_bucket = syn::Ident::new("__cq_pair_bucket", proc_macro2::Span::mixed_site());
    let __cq_pair_index = syn::Ident::new("__cq_pair_index", proc_macro2::Span::mixed_site());
    let __cq_pair_key = syn::Ident::new("__cq_pair_key", proc_macro2::Span::mixed_site());
    let __cq_pair_row = syn::Ident::new("__cq_pair_row", proc_macro2::Span::mixed_site());
    Ok(quote!({
        #work
        let mut #__cq_pair_index = ::std::collections::HashMap::<_, ::std::vec::Vec<_>>::new();
        for #__cq_pair_row in (#rs) {
            #[allow(unused_variables)]
            let #r = &#__cq_pair_row;
            let #__cq_pair_key = (#rk);
            #built
            #__cq_pair_index.entry(#__cq_pair_key).or_default().push(#__cq_pair_row);
        }
        for #__cq_pair_row in (#ls) {
            #[allow(unused_variables)]
            let #l = &#__cq_pair_row;
            let #__cq_pair_key = (#lk);
            #probed
            if let Some(#__cq_pair_bucket) = #__cq_pair_index.get(&#__cq_pair_key) {
                for #r in #__cq_pair_bucket { #body }
            }
        }
    }))
}
