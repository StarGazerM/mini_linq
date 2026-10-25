//! Task realizations of the checked key-partition constructors.
use cq_lang_sequence::Term;
use cq_lang_sequence::structures::symbol_name;
use cq_sequence_pull::{Emitter, Value};
use quote::quote;

pub(crate) fn partition(
    emitter: &Emitter<'_>,
    input: &Value,
    keys: &[syn::Ident],
    parts: usize,
    grain: usize,
    stored: Option<&syn::Ident>,
) -> syn::Result<proc_macro2::TokenStream> {
    if parts == 1 {
        let rows = &input.rows;
        return Ok(quote!(::core::iter::once(#rows)));
    }
    let result = input.result.as_ref().ok_or_else(|| {
        syn::Error::new(
            proc_macro2::Span::call_site(),
            "Tasks key partition lacks row provenance",
        )
    })?;
    let pattern = emitter.context().pattern(result);
    let keys = keys
        .iter()
        .map(|key| emitter.context().access(result, &symbol_name(key)))
        .collect::<Vec<_>>();
    let key = cq_print::group(&keys);
    let chunks = if let Some(stored) = stored {
        quote!(#stored.par_chunks(#grain).map(|__cq_chunk| __cq_chunk.iter().cloned()))
    } else {
        let rows = &input.rows;
        quote!((#rows).collect::<::std::vec::Vec<_>>().into_par_iter().chunks(#grain))
    };
    Ok(quote!({
        use ::rayon::prelude::*;
        ::cq_sequence_tasks::combine::route_by(#chunks, #parts, |#pattern, __cq_count| {
            ::cq_storage::sharded::route_index(&#key, __cq_count)
        }).into_iter().map(::std::vec::Vec::into_iter)
    }))
}

pub(crate) fn canon(
    emitter: &mut Emitter<'_>,
    input: &Value,
) -> syn::Result<proc_macro2::TokenStream> {
    let rows = &input.rows;
    let mut local = input.clone();
    local.rows = quote!(__cq_part.into_iter());
    local.depth = 0;
    let canonical = emitter.canon(local)?.rows;
    let canonical = quote!((#canonical).collect::<::std::vec::Vec<_>>());
    Ok(quote!({
        let __cq_parts = (#rows).map(|__cq_part| __cq_part.collect::<::std::vec::Vec<_>>())
            .collect::<::std::vec::Vec<_>>();
        ::cq_sequence_tasks::combine::map_parts(__cq_parts, |__cq_part| {
            #canonical
        }).into_iter().map(::std::vec::Vec::into_iter)
    }))
}

pub(crate) fn group(
    emitter: &mut Emitter<'_>,
    input: &Value,
    name: &syn::Ident,
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
        ::cq_sequence_tasks::combine::map_parts(__cq_parts, |__cq_part| {
            #grouped
        }).into_iter().map(::std::vec::Vec::into_iter)
    }))
}

pub(crate) fn partition_keys<'a>(
    term: &'a Term,
    program: &'a cq_lang_sequence::Program,
) -> Option<&'a [syn::Ident]> {
    match term {
        Term::PartitionK { keys, .. } => Some(keys),
        Term::Ref(name) => program
            .sequence
            .definitions
            .iter()
            .find(|definition| definition.name == *name)
            .and_then(|definition| partition_keys(&definition.term, program)),
        Term::Store { inner, .. }
        | Term::Canon(inner)
        | Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Group(_, inner)
        | Term::Lookup { inner, .. } => partition_keys(inner, program),
        Term::Choose(arms) => {
            let keys = arms
                .first()
                .and_then(|(_, arm)| partition_keys(arm, program))?;
            arms.iter()
                .all(|(_, arm)| partition_keys(arm, program) == Some(keys))
                .then_some(keys)
        }
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
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Canon(inner)
        | Term::Store { inner, .. }
        | Term::Tile { inner, .. }
        | Term::Lookup { inner, .. } => ancestors(inner, names),
        Term::Choose(arms) => {
            for (_, arm) in arms {
                ancestors(arm, names);
            }
        }
        Term::Union(arms) => {
            for arm in arms {
                ancestors(arm, names);
            }
        }
        Term::Rows(_) | Term::Ref(_) => {}
    }
}

/// Fold each scheduler task directly into a finite map, then merge maps on host.
/// The checked Tile premise proves complete valuations distinct across tasks.
pub(crate) fn partial(
    emitter: &mut Emitter<'_>,
    input: &Value,
    name: &syn::Ident,
    tile: Option<super::TaskTile>,
    workers: usize,
) -> syn::Result<proc_macro2::TokenStream> {
    use cq_lang_sequence::structures::{physical::Operator, syntax::algebra::AggregateKind};
    let result = symbol_name(name);
    let Operator::Group(group) = &emitter.context().physical[&result].operator else {
        return Err(syn::Error::new_spanned(
            name,
            "Tasks partial map lacks Group provenance",
        ));
    };
    let types = group
        .aggregates
        .iter()
        .map(|aggregate| aggregate.ty.clone())
        .collect::<Vec<_>>();
    let identities = group
        .aggregates
        .iter()
        .map(|aggregate| match aggregate.kind {
            AggregateKind::Count => quote!(0usize),
            AggregateKind::Sum => {
                let ty = &aggregate.ty;
                quote!(<#ty as ::core::default::Default>::default())
            }
            AggregateKind::Min | AggregateKind::Max => quote!(::core::option::Option::None),
        })
        .collect::<Vec<_>>();
    let merges = group
        .aggregates
        .iter()
        .enumerate()
        .map(|(position, aggregate)| {
            let field = syn::Index::from(position);
            match aggregate.kind {
                AggregateKind::Count | AggregateKind::Sum => {
                    quote!(__cq_state.#field += __cq_partial.#field;)
                }
                AggregateKind::Min | AggregateKind::Max => {
                    let compare = if aggregate.kind == AggregateKind::Min {
                        quote!(__cq_value < *__cq_previous)
                    } else {
                        quote!(__cq_value > *__cq_previous)
                    };
                    quote! {
                        if let ::core::option::Option::Some(__cq_value) = __cq_partial.#field {
                            if __cq_state.#field.as_ref().is_none_or(|__cq_previous| #compare) {
                                __cq_state.#field = ::core::option::Option::Some(__cq_value);
                            }
                        }
                    }
                }
            }
        })
        .collect::<Vec<_>>();
    let row = emitter.context().carrier(&result, |column| {
        if let Some(position) = group.keys.iter().position(|key| symbol_name(key) == column) {
            let field = syn::Index::from(position);
            quote!(__cq_keys.#field)
        } else {
            let field = syn::Index::from(
                group
                    .aggregates
                    .iter()
                    .position(|aggregate| symbol_name(&aggregate.name) == column)
                    .expect("checked group output"),
            );
            quote!(__cq_state.#field)
        }
    });
    let rows = &input.rows;
    let mut local = input.clone();
    local.depth = 0;
    let (binding, prepare, parts) = if let Some(super::TaskTile {
        binding,
        prepare,
        parts,
    }) = tile
    {
        local.rows = quote!((#rows).flatten());
        (binding, prepare, parts)
    } else {
        let binding = syn::parse_quote!(__cq_group_part);
        local.rows = quote!(::core::iter::IntoIterator::into_iter(#binding));
        (
            binding,
            quote!(let __cq_group_input = (#rows)
                .map(|part| part.collect::<::std::vec::Vec<_>>()).collect::<::std::vec::Vec<_>>();),
            quote!(__cq_group_input.into_par_iter()),
        )
    };
    let table = quote::format_ident!("__cq_task_groups", span = proc_macro2::Span::mixed_site());
    let updates = emitter.group_accumulate(name, local, quote!(#table))?;
    let updates = super::task_scope(
        emitter,
        quote!({
            let () = #updates;
            #table
        }),
    );
    let merged = quote!({
        use ::rayon::prelude::*;
        #prepare
        let __cq_task_context = ::cq_storage::profile_select! {
            plain { () }
            counters { ::cq_storage::counters::TaskContext::capture() }
        };
        let __cq_tiles = #parts;
        let __cq_tiles_per_task = __cq_tiles.len().div_ceil(#workers).max(1);
        let __cq_partials = __cq_tiles.fold_chunks(
            __cq_tiles_per_task,
            ::std::collections::HashMap::<_, (#(#types,)*)>::new,
            |mut #table, #binding| #updates,
        ).collect::<::std::vec::Vec<_>>();
        let mut #table = ::std::collections::HashMap::<_, (#(#types,)*)>::new();
        for __cq_map in __cq_partials {
            for (__cq_keys, __cq_partial) in __cq_map {
                let __cq_state = #table.entry(__cq_keys).or_insert_with(|| (#(#identities,)*));
                #(#merges)*
            }
        }
        #table.into_iter().map(|(__cq_keys, __cq_state)| #row)
    });
    let merged = if let Some(index) = emitter.context().profile_index(&result) {
        let profile = emitter.context().profile_handle();
        quote!(#profile.scope(#index, || #merged))
    } else {
        merged
    };
    Ok(emitter.context().profile_output(&result, merged))
}

pub(crate) fn global(
    emitter: &mut Emitter<'_>,
    input: &Value,
    name: &syn::Ident,
    tile: Option<super::TaskTile>,
) -> syn::Result<proc_macro2::TokenStream> {
    use cq_lang_sequence::structures::{physical::Operator, syntax::algebra::AggregateKind};
    let result = symbol_name(name);
    let Operator::Group(group) = &emitter.context().physical[&result].operator else {
        return Err(syn::Error::new_spanned(
            name,
            "Tasks partial fold lacks group provenance",
        ));
    };
    let initial = emitter.context().carrier(&result, |column| {
        let aggregate = group
            .aggregates
            .iter()
            .find(|aggregate| symbol_name(&aggregate.name) == column)
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
    let merges = emitter
        .context()
        .carried(&result)
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let field = syn::Index::from(index);
            let aggregate = group
                .aggregates
                .iter()
                .find(|aggregate| symbol_name(&aggregate.name) == *column)
                .expect("checked global aggregate column");
            match aggregate.kind {
                AggregateKind::Count | AggregateKind::Sum => {
                    quote!(__cq_state.#field += __cq_partial.#field;)
                }
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
        })
        .collect::<Vec<_>>();
    let rows = &input.rows;
    let mut local = input.clone();
    local.depth = 0;
    let (parts, binding, prepare) = if let Some(super::TaskTile {
        binding,
        prepare,
        parts,
    }) = tile
    {
        local.rows = quote!((#rows).flatten());
        (parts, quote!(let #binding = __cq_part;), prepare)
    } else {
        local.rows = quote!(__cq_part.into_iter());
        (
            quote!((#rows).map(|__cq_part| __cq_part.collect::<::std::vec::Vec<_>>())
            .collect::<::std::vec::Vec<_>>().into_par_iter()),
            quote!(),
            quote!(),
        )
    };
    let grouped = emitter.group_partial(name, local)?.rows;
    let grouped = quote!({
        #binding
        (#grouped).next().expect("global aggregate always has an identity row")
    });
    let grouped = super::task_scope(emitter, grouped);
    let merge = quote!({
        use ::rayon::prelude::*;
        #prepare
        let __cq_task_context = ::cq_storage::profile_select! {
            plain { () }
            counters { ::cq_storage::counters::TaskContext::capture() }
        };
        let __cq_partials = (#parts).map(|__cq_part| #grouped).collect::<::std::vec::Vec<_>>();
        let mut __cq_state = #initial;
        for __cq_partial in __cq_partials {
            #(#merges)*
        }
        __cq_state
    });
    let merge = if let Some(index) = emitter.context().profile_index(&result) {
        let profile = emitter.context().profile_handle();
        quote!(#profile.scope(#index, || #merge))
    } else {
        merge
    };
    Ok(emitter
        .context()
        .profile_output(&result, quote!(::core::iter::once(#merge))))
}
