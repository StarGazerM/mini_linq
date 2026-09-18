//! RA operators become Access plans; native leaves own physical data access.
use super::{
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
    let provider = wiring.provider(&target.view.provider);
    materialize(
        target,
        quote!(::core::iter::once(#provider! { record () })),
        wiring,
    )
}
pub(super) fn project(
    input: &View,
    target: &Materialized<'_>,
    next: &syn::Path,
    wiring: &Wiring,
) -> syn::Result<TokenStream> {
    let columns = positions(&input.layout, &target.view.layout);
    let name = &input.name;
    let input_binding = quote::format_ident!("{}_input0", target.view.name);
    let provider = wiring.provider(&input.provider);
    let output = wiring.provider(&target.view.provider);
    let fields = columns.iter().map(|column| {
        let field = super::expressions::field(input, quote!(__row), *column, wiring);
        quote!(::core::clone::Clone::clone(&(#field)))
    });
    let plan: access::Plan = syn::parse2(quote! {
        out(__row) => (#output! { record (#(#fields,)*) }) :-
        Rows(__row) => for (__row,) in (#provider! { scan (#input_binding) }.map(|row| (row,)));
    })?;
    access::contract::check(&plan)?;
    let construction = materialize(target, quote!(#next! { #plan }), wiring);
    Ok(quote! { let #input_binding = &#name; #construction })
}

pub(super) fn select(
    input: &View,
    target: &View,
    condition: &syn::Expr,
    next: &syn::Path,
    wiring: &Wiring,
) -> syn::Result<TokenStream> {
    let provider = wiring.provider(&input.provider);
    let (input_name, name, next) = (&input.name, &target.name, next);
    let condition = super::expressions::condition(condition, input, quote!(__cq_row), wiring);
    let predicate = quote!({ let __cq_row = #provider! { row (__cq_handle) }; #condition });
    let plan: access::Plan = syn::parse2(quote! {
        out(__cq_handle) => (__cq_handle) :-
        Rows(__cq_handle) => for (__cq_handle,) in (
            #provider! { rows (__cq_input) }.map(|handle| (handle,))
        ),
        if #predicate => if (#predicate);
    })?;
    access::contract::check(&plan)?;
    Ok(quote! {
        let #name = {
            let __cq_input = &#input_name;
            #provider! { select (__cq_input) (#next! { #plan }) }
        };
    })
}

pub(super) fn extend(
    input: &View,
    target: &Materialized<'_>,
    operator: &cq_ir::relational_plan::Extend,
    next: &syn::Path,
    wiring: &Wiring,
) -> syn::Result<TokenStream> {
    let provider = wiring.provider(&input.provider);
    let output = wiring.provider(&target.view.provider);
    let name = &input.name;
    let input_binding = quote::format_ident!("{}_input0", target.view.name);
    let expression = super::expressions::expression(&operator.value, input, quote!(__row), wiring);
    let types = operator
        .columns
        .iter()
        .map(|column| crate::rust_bindings::qualify_type(&column.ty));
    let new_names = operator
        .columns
        .iter()
        .map(|column| cq_ir::symbol_name(&column.name))
        .collect::<Vec<_>>();
    let fields = target.view.layout.iter().map(|name| {
        if let Some(position) = input.layout.iter().position(|column| column == name) {
            let field = super::expressions::field(input, quote!(__row), position, wiring);
            quote!(::core::clone::Clone::clone(&(#field)))
        } else {
            let index =
                syn::Index::from(new_names.iter().position(|column| column == name).unwrap());
            quote!(__cq_value.#index)
        }
    });
    let plan: access::Plan = syn::parse2(quote! {
        out(__row) => ({
            let __cq_value: (#(#types,)*) = #expression;
            #output! { record (#(#fields,)*) }
        }) :-
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
    next: &syn::Path,
    wiring: &Wiring,
) -> syn::Result<TokenStream> {
    let columns = positions(&left.layout, &resource.columns);
    let provider = wiring.provider(&left.provider);
    let scan = wiring.access(scan);
    let keys = wiring.access(&resource.provider);
    let fields = columns.iter().map(|column| {
        let field = super::expressions::field(left, quote!(__cq_row), *column, wiring);
        quote!(&(#field))
    });
    let key = if columns.is_empty() {
        quote!(#keys! { key () })
    } else {
        quote!({
            let __cq_row = #provider! { row (__cq_handle) };
            #keys! { key (#(#fields,)*) }
        })
    };
    let (left, name, resource, next) = (&left.name, &target.name, &resource.name, next);
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
    algorithm: &syn::Path,
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
    let output = wiring.provider(&target.view.provider);
    let algorithm = wiring.access(algorithm);
    let (l, r) = (&left.name, &right.name);
    let left_binding = quote::format_ident!("{}_input0", target.view.name);
    let right_binding = quote::format_ident!("{}_input1", target.view.name);
    let (owner, name) = (&target.owner, &target.view.name);
    let left_key = lk
        .iter()
        .map(|&i| syn::Index::from(i))
        .map(|i| quote!(&(#lp! { field (__left_row) (#i) })));
    let right_key = rk
        .iter()
        .map(|&i| syn::Index::from(i))
        .map(|i| quote!(&(#rp! { field (__right_row) (#i) })));
    let fields = target.view.layout.iter().map(|column| {
        if let Some(i) = left.layout.iter().position(|name| name == column) {
            let i = syn::Index::from(i);
            quote!(::core::clone::Clone::clone(&(#lp! { field (__left_row) (#i) })))
        } else {
            let i = right.layout.iter().position(|name| name == column).unwrap();
            let i = syn::Index::from(i);
            quote!(::core::clone::Clone::clone(&(#rp! { field (__right_row) (#i) })))
        }
    });
    let construction: cq_ir::native::Operation = syn::parse2(quote! {
        produce #owner #name via __emit {
            #algorithm! {
                pairs __left_row in (#lp! { scan (#left_binding) }) by ((#(#left_key,)*)),
                      __right_row in (#rp! { scan (#right_binding) }) by ((#(#right_key,)*)) {
                    __emit(#output! { record (#(#fields,)*) });
                }
            };
        }
    })?;
    Ok(quote! {
        let #left_binding = &#l;
        let #right_binding = &#r;
        #output! { #construction }
    })
}

fn materialize(target: &Materialized<'_>, rows: TokenStream, wiring: &Wiring) -> TokenStream {
    let (owner, name) = (&target.owner, &target.view.name);
    let provider = wiring.provider(&target.view.provider);
    quote! { #provider! { materialize #owner #name (#rows) } }
}
