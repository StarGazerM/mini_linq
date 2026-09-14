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
    let provider = wiring.provider(&input.provider);
    let next = &options.access;
    let plan: access::Plan = syn::parse2(quote! {
        out(__row) => ({ let __projected: [::std::primitive::i64; #width] = [#(__row[#columns]),*]; __projected }) :-
        Rows(__row) => for (__row,) in (#provider! { scan (&#name) }.map(|row| (row,)));
    })?;
    access::contract::check(&plan)?;
    Ok(materialize(target, quote!(#next! { #plan }), wiring))
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
    let scan = wiring.access(scan, None);
    let keys = wiring.access(&resource.provider, Some(&resource.keys));
    let (left, name, resource, next) =
        (&left.name, &target.name, &resource.name, &options.iterator);
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
    let witness = wiring.access(witnesses, None);
    let (l, r, next) = (&left.name, &right.name, &options.access);
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
        Rows(__row) => for (__row,) in (#rp! { scan (__join_right) }.map(|row| (row,)));
    })?;
    let plan: access::Plan = syn::parse2(quote! {
        out(__left_row,__right_row) => ({ let __joined: [::std::primitive::i64; #width] = [#(#fields),*]; __joined }) :-
        Left(__left_row) => for (__left_row,) in (#lp! { scan (__join_left) }.map(|row| (row,))),
        Right(__right_row) => for (__right_row,) in (
            #witness! { probe (__join_resource_ref) ([#(__left_row[#lk]),*]) }.map(|row| (row,))
        );
    })?;
    access::contract::check(&build)?;
    access::contract::check(&plan)?;
    Ok(materialize(
        target,
        quote!({
            let __join_left = &#l;
            let __join_right = &#r;
            let __join_resource = #witness! { build { #build } };
            let __join_resource_ref = &__join_resource;
            #next! { #plan }
        }),
        wiring,
    ))
}
fn materialize(target: &Materialized<'_>, rows: TokenStream, wiring: &Wiring) -> TokenStream {
    let (owner, name) = (&target.owner, &target.view.name);
    let provider = wiring.provider(&target.view.provider);
    quote! { #provider! { materialize #owner #name (#rows) (__ml_block_rows) } }
}
