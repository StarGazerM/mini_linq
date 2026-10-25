//! Task realizations of the checked key-partition constructors.
use cq_lang_sequence::Term;
use cq_lang_sequence::structures::symbol_name;
use cq_sequence_pull::{Emitter, Value};
use quote::quote;

pub(crate) fn partition(
    emitter: &Emitter<'_>, input: &Value, keys: &[syn::Ident], parts: usize, grain: usize,
) -> syn::Result<proc_macro2::TokenStream> {
    let result = input.result.as_ref().ok_or_else(|| syn::Error::new(
        proc_macro2::Span::call_site(), "Tasks key partition lacks row provenance"))?;
    let pattern = emitter.context().pattern(result);
    let keys = keys.iter().map(|key| emitter.context().access(result, &symbol_name(key))).collect::<Vec<_>>();
    let key = cq_print::group(&keys);
    let rows = &input.rows;
    Ok(quote!({
        use ::rayon::prelude::*;
        let __cq_chunks = (#rows).collect::<::std::vec::Vec<_>>()
            .into_par_iter().chunks(#grain).collect::<::std::vec::Vec<_>>();
        ::mini_linq::__tasks::combine::route_by(__cq_chunks, #parts, |#pattern, __cq_count| {
            ::cq_storage::sharded::route_index(&#key, __cq_count)
        }).into_iter().map(::std::vec::Vec::into_iter)
    }))
}

pub(crate) fn canon(emitter: &mut Emitter<'_>, input: &Value) -> syn::Result<proc_macro2::TokenStream> {
    let rows = &input.rows;
    let mut local = input.clone();
    local.rows = quote!(__cq_part.into_iter());
    local.depth = 0;
    let canonical = emitter.canon(local)?.rows;
    let canonical = quote!((#canonical).collect::<::std::vec::Vec<_>>());
    Ok(quote!({
        let __cq_parts = (#rows).map(|__cq_part| __cq_part.collect::<::std::vec::Vec<_>>())
            .collect::<::std::vec::Vec<_>>();
        ::mini_linq::__tasks::combine::map_parts(__cq_parts, |__cq_part| {
            #canonical
        }).into_iter().map(::std::vec::Vec::into_iter)
    }))
}

pub(crate) fn group(
    emitter: &mut Emitter<'_>, input: &Value, name: &syn::Ident,
) -> syn::Result<proc_macro2::TokenStream> {
    let rows = &input.rows;
    let mut local = input.clone();
    local.rows = quote!(__cq_part.into_iter());
    local.depth = 0;
    let grouped = emitter.group(name, local)?.rows;
    let grouped = quote!((#grouped).collect::<::std::vec::Vec<_>>());
    Ok(quote!({
        let __cq_parts = (#rows).map(|__cq_part| __cq_part.collect::<::std::vec::Vec<_>>())
            .collect::<::std::vec::Vec<_>>();
        ::mini_linq::__tasks::combine::map_parts(__cq_parts, |__cq_part| {
            #grouped
        }).into_iter().map(::std::vec::Vec::into_iter)
    }))
}

pub(crate) fn partition_keys<'a>(
    term: &'a Term, program: &'a cq_lang_sequence::Program,
) -> Option<&'a [syn::Ident]> {
    match term {
        Term::PartitionK { keys, .. } => Some(keys),
        Term::Ref(name) => program.sequence.definitions.iter()
            .find(|definition| definition.name == *name)
            .and_then(|definition| partition_keys(&definition.term, program)),
        Term::Store { inner, .. } | Term::Canon(inner)
        | Term::Map(_, inner) | Term::Compact(_, inner) | Term::Group(_, inner)
        | Term::Lookup { inner, .. } => partition_keys(inner, program),
        _ => None,
    }
}

pub(crate) fn ancestors(term: &Term, names: &mut std::collections::BTreeSet<String>) {
    match term {
        Term::PartitionK { inner, .. } => {
            if let Term::Ref(name) = inner.as_ref() {
                names.insert(symbol_name(name));
            }
            ancestors(inner, names);
        }
        Term::Map(_, inner) | Term::Compact(_, inner) | Term::Flatten(inner)
        | Term::Fold(_, inner) | Term::Group(_, inner) | Term::Canon(inner)
        | Term::Store { inner, .. } | Term::Tile { inner, .. }
        | Term::Lookup { inner, .. } => ancestors(inner, names),
        Term::Choose(arms) => {
            for (_, arm) in arms { ancestors(arm, names); }
        }
        Term::Union(arms) => {
            for arm in arms { ancestors(arm, names); }
        }
        Term::Rows(_) | Term::Ref(_) => {}
    }
}

pub(crate) fn global(
    emitter: &mut Emitter<'_>,
    input: &Value,
    name: &syn::Ident,
    tile: Option<(syn::Ident, proc_macro2::TokenStream, usize)>,
) -> syn::Result<proc_macro2::TokenStream> {
    use cq_lang_sequence::structures::{physical::Operator, syntax::algebra::AggregateKind};
    let result = symbol_name(name);
    let Operator::Group(group) = &emitter.context().physical[&result].operator else {
        return Err(syn::Error::new_spanned(name, "Tasks partial fold lacks group provenance"));
    };
    let initial = emitter.context().carrier(&result, |column| {
        let aggregate = group.aggregates.iter().find(|aggregate| symbol_name(&aggregate.name) == column)
            .expect("checked global aggregate column");
        match aggregate.kind {
            AggregateKind::Count => quote!(0usize),
            AggregateKind::Sum => {
                let ty = &aggregate.ty;
                quote!(<#ty as ::core::default::Default>::default())
            }
            AggregateKind::Min | AggregateKind::Max => quote!(::core::option::Option::None),
        }
    });
    let merges = emitter.context().carried(&result).iter().enumerate().map(|(index, column)| {
        let field = syn::Index::from(index);
        let aggregate = group.aggregates.iter().find(|aggregate| symbol_name(&aggregate.name) == *column)
            .expect("checked global aggregate column");
        match aggregate.kind {
            AggregateKind::Count | AggregateKind::Sum =>
                quote!(__cq_state.#field += __cq_partial.#field;),
            AggregateKind::Min | AggregateKind::Max => {
                let comparison = if aggregate.kind == AggregateKind::Min {
                    quote!(__cq_value < *__cq_previous)
                } else {
                    quote!(__cq_value > *__cq_previous)
                };
                quote! {
                    if let ::core::option::Option::Some(__cq_value) = __cq_partial.#field {
                        if __cq_state.#field.as_ref().is_none_or(|__cq_previous| #comparison) {
                            __cq_state.#field = ::core::option::Option::Some(__cq_value);
                        }
                    }
                }
            }
        }
    }).collect::<Vec<_>>();
    let rows = &input.rows;
    let mut local = input.clone();
    local.depth = 0;
    let (parts, binding) = if let Some((binding, source, grain)) = tile {
        local.rows = quote!((#rows).flatten());
        (quote!({
            use ::rayon::prelude::*;
            (#source).collect::<::std::vec::Vec<_>>().into_par_iter()
                .chunks(#grain).collect::<::std::vec::Vec<_>>()
        }), quote!(let #binding = &__cq_part;))
    } else {
        local.rows = quote!(__cq_part.into_iter());
        (quote!((#rows).map(|__cq_part| __cq_part.collect::<::std::vec::Vec<_>>())
            .collect::<::std::vec::Vec<_>>()), quote!())
    };
    let grouped = emitter.group_partial(name, local)?.rows;
    let grouped = quote!({
        #binding
        (#grouped).next().expect("global aggregate always has an identity row")
    });
    let merge = quote!(
        ::mini_linq::__tasks::combine::par_group_global_with(
            #parts,
            |__cq_part| #grouped,
            || #initial,
            |__cq_state, __cq_partial| { #(#merges)* },
        )
    );
    let merge = if let Some(index) = emitter.context().profile_index(&result) {
        let profile = emitter.context().profile_handle();
        quote!(#profile.scope(#index, || #merge))
    } else {
        merge
    };
    Ok(emitter.context().profile_output(&result, quote!(::core::iter::once(#merge))))
}
