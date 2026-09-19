//! Iterable and field interpretation of the concrete borrowed representations.
//!
//! These leaves do not compose query bindings or choose an execution model.

use cq_ir::{
    provider_protocol::{self as protocol, exact as family},
    symbol_name,
};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Expr, Ident};

use crate::{
    layout, storage,
    syntax::{ProviderRequest, ProviderTask},
};

fn fields(names: &[Ident], values: &[Expr]) -> TokenStream {
    let ordinals = (0..names.len()).map(syn::Index::from);
    quote! { #(#ordinals = (#values);)* }
}

pub fn slice_rows(request: &ProviderRequest) -> syn::Result<TokenStream> {
    let ProviderTask::Rows(family::Task::Rows { name, .. }) = &request.request else {
        unreachable!()
    };
    let columns = layout::logical_columns(&request.plan, name)?;
    let [column] = columns.as_slice() else {
        return Err(syn::Error::new_spanned(
            name,
            "slice_rows requires one logical column",
        ));
    };
    let (source, _) = layout::stored(&request.environment, name)?;
    let callback = &request.reply_to;
    let state = &request.state;
    Ok(quote! { #callback! {
        resume { #state }
        source ((#source).iter())
        item (#column)
        witness { row (#column) fields { 0 = (#column); } }
    } })
}

pub fn map_rows(request: &ProviderRequest) -> syn::Result<TokenStream> {
    let ProviderTask::Rows(family::Task::Rows { name, .. }) = &request.request else {
        unreachable!()
    };
    let physical = layout::physical_columns(&request.plan, name)?;
    let [column, payload] = physical.as_slice() else {
        return Err(syn::Error::new_spanned(
            name,
            "map rows require two logical columns",
        ));
    };
    let columns = layout::logical_columns(&request.plan, name)?;
    let values: Vec<Expr> = columns.iter().map(|c| syn::parse_quote!(#c)).collect();
    let output_fields = fields(&columns, &values);
    let (source, _) = layout::stored(&request.environment, name)?;
    let callback = &request.reply_to;
    let state = &request.state;
    Ok(quote! { #callback! {
        resume { #state }
        source (
            (#source).iter().flat_map(|(#column, __cq_values)|
                __cq_values.iter().map(move |#payload| (#(#values,)*)))
        )
        item ((#(#columns,)*))
        witness { row ((#(#values,)*)) fields { #output_fields } }
    } })
}

pub fn map_matches(request: &ProviderRequest) -> syn::Result<TokenStream> {
    let ProviderTask::Matches(family::Task::Matches { name, values, .. }) = &request.request else {
        unreachable!()
    };
    let index = storage::map_index(request, name)?;
    if values.len() != 1 {
        return Err(syn::Error::new_spanned(
            &request.request,
            "map lookup requires one borrowed column expression",
        ));
    }
    let probe = &values[0].value;
    let relation = &index.requirement.relation;
    let physical = layout::physical_columns(&request.plan, relation)?;
    let [column, payload] = physical.as_slice() else {
        unreachable!()
    };
    let columns = layout::heading(&request.plan, relation)?;
    let decoded: Vec<Expr> = columns
        .iter()
        .map(|c| {
            if symbol_name(c) == symbol_name(column) {
                probe.clone()
            } else {
                syn::parse_quote!(#payload)
            }
        })
        .collect();
    let output_fields = fields(&columns, &decoded);
    let Some(protocol::Binding::Indexed { value: source, .. }) = request.environment.index(name)
    else {
        return Err(syn::Error::new_spanned(
            name,
            "Matches requires the exact previously established IX index binding",
        ));
    };
    let callback = &request.reply_to;
    let state = &request.state;
    Ok(quote! { #callback! {
        resume { #state }
        source ((#source).get(#probe).into_iter().flatten())
        item (#payload)
        witness { row ((#(#decoded,)*)) fields { #output_fields } }
    } })
}

pub fn borrowed_fields(tokens: TokenStream) -> syn::Result<TokenStream> {
    let request: protocol::Request<family::Task> = syn::parse2(tokens)?;
    let family::Task::Output {
        name,
        fields: requested,
        ..
    } = &request.request
    else {
        return Err(syn::Error::new_spanned(
            &request.request,
            "borrowed_fields constructs only the selected output carrier; it is not reusable storage",
        ));
    };
    let heading = layout::heading(&request.plan, name)?;
    let mut values = vec![None; heading.len()];
    for field in requested {
        let Some(slot) = values.get_mut(field.column.index as usize) else {
            return Err(syn::Error::new_spanned(
                field,
                "output field exceeds its logical heading",
            ));
        };
        if slot.replace(field.value.clone()).is_some() {
            return Err(syn::Error::new_spanned(
                field,
                "duplicate logical output field",
            ));
        }
    }
    let values = values
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| {
            syn::Error::new_spanned(
                name,
                "output construction requires each logical field exactly once",
            )
        })?;
    let output_fields = fields(&heading, &values);
    let callback = &request.reply_to;
    let state = &request.state;
    Ok(quote! { #callback! {
        resume { #state }
        witness { row ((#(#values,)*)) fields { #output_fields } }
    } })
}
