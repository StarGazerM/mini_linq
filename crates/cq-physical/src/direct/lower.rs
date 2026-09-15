//! RA operators become Access plans; native leaves own physical data access.
use super::{
    PhysicalOptions,
    layout::{View, positions, separator},
    resources::Resource,
    wiring::Wiring,
};
use cq_ir::rust_access_plan as access;
use proc_macro2::TokenStream;
use quote::quote;

pub(super) struct Materialized<'a> {
    pub view: &'a View,
    pub owner: syn::Ident,
}
pub(super) fn unit(target: &Materialized<'_>, wiring: &Wiring) -> TokenStream {
    materialize(
        target,
        quote!(::std::collections::HashSet::<[::std::primitive::i64; 0]>::from([[]])),
        wiring,
    )
}
pub(super) fn project(
    input: &View,
    target: &Materialized<'_>,
    options: &PhysicalOptions,
    wiring: &Wiring,
) -> syn::Result<TokenStream> {
    let columns = positions(&input.layout, &target.view.layout);
    let width = columns.len();
    let name = &input.name;
    let input_binding = quote::format_ident!("{}_input", target.owner);
    let provider = wiring.provider(&input.provider);
    let next = &options.access;
    let plan: access::Plan = syn::parse2(quote! {
        out(__row) => ({ let __projected: [::std::primitive::i64; #width] = [#(__row[#columns]),*]; __projected }) :-
        Rows(__row) => for (__row,) in (#provider! { scan (#input_binding) }.map(|row| (row,)));
    })?;
    access::contract::check(&plan)?;
    let construction = materialize(target, quote!(#next! { #plan }), wiring);
    Ok(quote! { let #input_binding = &#name; #construction })
}

pub(super) fn semijoin(
    left: &View,
    target: &View,
    resource: &Resource,
    scan: &syn::Path,
    options: &PhysicalOptions,
    wiring: &Wiring,
) -> syn::Result<TokenStream> {
    let columns = positions(&left.layout, &resource.columns);
    let provider = wiring.provider(&left.provider);
    let scan = wiring.access(scan);
    let keys = wiring.access(&resource.provider);
    let (left, name, resource, next) = (&left.name, &target.name, &resource.name, &options.access);
    let key = if columns.is_empty() {
        quote!([])
    } else {
        quote!({
            let __cq_row = #provider! { row (__cq_handle) };
            [#(__cq_row[#columns]),*]
        })
    };
    let predicate = quote!(#keys! { contains (__cq_resource) (#key) });
    let plan: access::Plan = syn::parse2(quote! {
        out(__cq_handle) => (__cq_handle) :-
        Candidates(__cq_handle) => for (__cq_handle,) in (
            #scan! {
                candidates #provider { __cq_left }
                (move |bounds| #keys! { may_match (__cq_resource) (bounds) (&[#(#columns),*]) })
            }.map(|handle| (handle,))
        ),
        if #predicate => if (#predicate);
    })?;
    access::contract::check(&plan)?;
    Ok(quote! {
        let #name = {
            let __cq_left = &#left;
            let __cq_resource = &#resource;
            #provider! { select (__cq_left) (#next! { #plan }) }
        };
    })
}

pub(super) fn join(
    left: &View,
    right: &View,
    target: &Materialized<'_>,
    witnesses: &syn::Path,
    options: &PhysicalOptions,
    wiring: &Wiring,
) -> syn::Result<TokenStream> {
    let key = separator(left, right);
    let (lk, rk) = (
        positions(&left.layout, &key),
        positions(&right.layout, &key),
    );
    let (lp, rp) = (
        wiring.provider(&left.provider),
        wiring.provider(&right.provider),
    );
    let witness = wiring.access(witnesses);
    let (l, r, next) = (&left.name, &right.name, &options.access);
    let left_binding = quote::format_ident!("{}_left", target.owner);
    let right_binding = quote::format_ident!("{}_right", target.owner);
    let resource_binding = quote::format_ident!("{}_witness", target.owner);
    let resource_reference = quote::format_ident!("{}_witness_ref", target.owner);
    let key_width = key.len();
    let width = target.view.layout.len();
    let fields = target.view.layout.iter().map(|column| {
        if let Some(position) = left.layout.iter().position(|name| name == column) {
            quote!(__left_row[#position])
        } else {
            let position = right.layout.iter().position(|name| name == column).unwrap();
            quote!(__right_row[#position])
        }
    });
    let build: access::Plan = syn::parse2(quote! {
        out(__row) => ({ let __key: [::std::primitive::i64; #key_width] = [#(__row[#rk]),*]; (__key, __row) }) :-
        Rows(__row) => for (__row,) in (#rp! { scan (#right_binding) }.map(|row| (row,)));
    })?;
    let plan: access::Plan = syn::parse2(quote! {
        out(__left_row,__right_row) => ({ let __joined: [::std::primitive::i64; #width] = [#(#fields),*]; __joined }) :-
        Left(__left_row) => for (__left_row,) in (#lp! { scan (#left_binding) }.map(|row| (row,))),
        Right(__right_row) => for (__right_row,) in (
            #witness! { probe (#resource_reference) ([#(__left_row[#lk]),*]) }.map(|row| (row,))
        );
    })?;
    access::contract::check(&build)?;
    access::contract::check(&plan)?;
    let construction = materialize(target, quote!(#next! { #plan }), wiring);
    Ok(quote! {
        let #left_binding = &#l;
        let #right_binding = &#r;
        let #resource_binding = #witness! { build { #build } };
        let #resource_reference = &#resource_binding;
        #construction
    })
}
fn materialize(target: &Materialized<'_>, rows: TokenStream, wiring: &Wiring) -> TokenStream {
    let (owner, name) = (&target.owner, &target.view.name);
    let provider = wiring.provider(&target.view.provider);
    quote! { #provider! { materialize #owner #name (#rows) } }
}
