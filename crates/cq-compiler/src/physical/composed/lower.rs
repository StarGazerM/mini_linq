//! Names and field mappings connect provider and algorithm expansion holes.
use super::{
    PhysicalOptions,
    layout::{View, positions},
    resources::Resource,
    wiring::Wiring,
};
use cq_ir::{
    physical_context as contextual, relational_plan as ra, rust_access_plan as access, symbol_name,
};
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
/// CP-Bind: serialize only the definition's free physical row bindings.
pub(super) fn context(
    definition: &ra::Definition,
    operands: &[(&syn::Ident, &View)],
    wiring: &Wiring,
) -> syn::Result<contextual::Context> {
    let mut seen = std::collections::BTreeSet::new();
    let bindings = operands
        .iter()
        .filter(|(id, _)| seen.insert(symbol_name(id)))
        .map(|(id, view)| binding(id, view, wiring));
    syn::parse2(quote! { #definition { #(#bindings)* } })
}
fn binding(id: &syn::Ident, view: &View, wiring: &Wiring) -> TokenStream {
    let columns = view.layout.iter().map(|name| logical_ident(name));
    let provider = wiring.provider(&view.provider);
    let value = &view.name;
    quote!(#id(#(#columns),*) => #provider { #value };)
}

pub(super) fn logical_ident(name: &str) -> syn::Ident {
    syn::parse_str(name)
        .unwrap_or_else(|_| syn::Ident::new_raw(name, proc_macro2::Span::call_site()))
}

pub(super) fn semijoin(
    definition: &ra::Definition,
    left: &View,
    target: &View,
    resource: &Resource,
    wiring: &Wiring,
) -> syn::Result<TokenStream> {
    let ra::Operator::Semijoin(op) = &definition.operator else {
        unreachable!()
    };
    let result = &definition.result;
    let left_id = &op.left;
    let projected = &resource.projected;
    let columns = resource.columns.iter().map(|name| logical_ident(name));
    let left_binding = binding(left_id, left, wiring);
    let value = &resource.name;
    let invocation: contextual::Context = syn::parse2(quote! {
        #result = semijoin #left_id with #projected;
        { #left_binding #projected(#(#columns),*) { #value }; }
    })?;
    contextual::contract::check_semijoin(&invocation)?;
    let name = &target.name;
    let recipe = wiring.recipe(&resource.recipe, Some(&resource.keys));
    Ok(quote! { let #name = #recipe! { #invocation }; })
}

pub(super) fn join(
    definition: &ra::Definition,
    left: &View,
    right: &View,
    target: &Materialized<'_>,
    recipe: &syn::Path,
    wiring: &Wiring,
) -> syn::Result<TokenStream> {
    let ra::Operator::NaturalJoin(op) = &definition.operator else {
        unreachable!()
    };
    let context = context(definition, &[(&op.left, left), (&op.right, right)], wiring)?;
    let recipe = wiring.recipe(recipe, None);
    contextual::contract::check_join(&context)?;
    Ok(materialize(
        target,
        quote! { #recipe! { #context } },
        wiring,
    ))
}
fn materialize(target: &Materialized<'_>, rows: TokenStream, wiring: &Wiring) -> TokenStream {
    let owner = &target.owner;
    let name = &target.view.name;
    let provider = wiring.provider(&target.view.provider);
    quote! { #provider! { materialize #owner #name (#rows) (__ml_block_rows) } }
}
