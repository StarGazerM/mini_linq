//! Block-directory, payload and mask expansions.
//! Handles carry storage identity separately from payload addresses, including
//! for zero-column records whose addresses cannot identify their allocation.
//! Recipes must preserve the handles produced by `rows`/`block_rows`: filtering,
//! reordering and deduplication are valid; recombining handle fields is not.
//! Selection uses their coordinates and checks identity/current membership.
use crate::physical::options::ProviderOptions;
use cq_ir::physical_provider::Operation;
use proc_macro2::TokenStream;
use quote::quote;

/// Expand a typed block operation with an independently selected mask.
///
/// The result is a Rust type for `Type`, statements for `Materialize`, and an
/// expression otherwise. The owning physical context supplies argument meanings.
pub fn compile_block_provider(
    operation: &Operation,
    options: &ProviderOptions,
) -> syn::Result<TokenStream> {
    let storage = &options.storage;
    let output = match operation {
        Operation::Type { arity, .. } => {
            quote!(#storage::BlockStore<#arity>)
        }
        Operation::Input { value, .. } => {
            quote!({
                let __cq_store = (#value);
                (__cq_store, <#storage::BlockMask as #storage::MakeSelection<_>>::all(__cq_store))
            })
        }
        Operation::Len { value, .. } => {
            quote!(#storage::BlockSelection::len(&(#value).1))
        }
        Operation::Row { value, .. } => {
            quote!((#value).3)
        }
        Operation::Materialize {
            owner,
            value,
            rows,
            block_size,
            ..
        } => {
            quote! {
                let #owner = {
                    // Infer the row width without allowing an inherent
                    // constructor to shadow the selected trait operation.
                    fn __cq_pack<const N: ::core::primitive::usize>(
                        rows: ::std::collections::HashSet<[::core::primitive::i64; N]>,
                        size: ::core::num::NonZeroUsize,
                    ) -> #storage::BlockStore<N>
                    where
                        #storage::BlockStore<N>: #storage::BuildBlocks<Row = [::core::primitive::i64; N]>,
                    {
                        <#storage::BlockStore<N> as #storage::BuildBlocks>::pack_unique(rows, size)
                    }
                    __cq_pack((#rows), (#block_size))
                };
                let #value = (&#owner, <#storage::BlockMask as #storage::MakeSelection<_>>::all(&#owner));
            }
        }
        Operation::Scan { value, .. } => block_scan(storage, value, options.work.as_ref(), true),
        Operation::Rows { value, .. } => block_scan(storage, value, options.work.as_ref(), false),
        Operation::Blocks { value, .. } => {
            let binding = work_binding(options.work.as_ref());
            let inspection = options
                .work
                .as_ref()
                .map(|_| quote!(.inspect(move |_| __cq_work.add(|w| w.blocks_considered += 1))));
            quote!({
                let __cq_source = (#value);
                #binding
                #storage::BlockRead::blocks(__cq_source.0)
                    .filter(move |b| #storage::BlockSelection::block_nonempty(
                        &__cq_source.1, #storage::BlockDescriptor::id(b)))
                    #inspection
            })
        }
        Operation::Bounds { descriptor, .. } => {
            quote!(#storage::CoordinateBounds::bounds(&(#descriptor)))
        }
        Operation::BlockRows {
            descriptor, value, ..
        } => block_rows(storage, descriptor, value, options.work.as_ref()),
        Operation::Select { value, rows, .. } => {
            quote!({
                let __cq_source = (#value);
                let __cq_selected = (#rows);
                let mut __cq_mask = <#storage::BlockMask as #storage::MakeSelection<_>>::empty(__cq_source.0);
                for (__cq_store, __cq_block, __cq_row, _) in __cq_selected {
                    assert!(__cq_store == #storage::BlockRead::identity(__cq_source.0),
                        "selected row belongs to another block store");
                    assert!(#storage::BlockSelection::contains(&__cq_source.1, __cq_block, __cq_row),
                        "selected coordinates are absent from the current mask");
                    #storage::BlockSelection::insert(&mut __cq_mask, __cq_block, __cq_row);
                }
                (__cq_source.0, __cq_mask)
            })
        }
    };
    Ok(output)
}

fn block_scan(
    storage: &syn::Path,
    value: &syn::Expr,
    work: Option<&syn::Expr>,
    payload_only: bool,
) -> TokenStream {
    let binding = work_binding(work);
    let local_work = work.map(|_| syn::parse_quote!(__cq_work));
    let rows = block_rows(
        storage,
        &syn::parse_quote!(__cq_block),
        &syn::parse_quote!(__cq_source),
        local_work.as_ref(),
    );
    let projection = payload_only.then(|| quote!(.map(|(_, _, _, row)| row)));
    quote!({
        let __cq_source = (#value);
        #binding
        #storage::BlockRead::blocks(__cq_source.0)
            .filter(move |b| #storage::BlockSelection::block_nonempty(
                &__cq_source.1, #storage::BlockDescriptor::id(b)))
            .flat_map(move |__cq_block| #rows)
            #projection
    })
}

fn block_rows(
    storage: &syn::Path,
    descriptor: &syn::Expr,
    value: &syn::Expr,
    work: Option<&syn::Expr>,
) -> TokenStream {
    let binding = work_binding(work);
    let opened = work.map(|_| quote!(let mut __cq_opened = false;));
    let update = work.map(|_| {
        quote! {
            __cq_work.add(|w| {
                if !__cq_opened {
                    __cq_opened = true;
                    w.blocks_opened += 1;
                }
                w.rows_visited += 1;
            });
        }
    });
    quote!({
        let __cq_block = (#descriptor);
        let __cq_source = (#value);
        #binding
        let __cq_store = #storage::BlockRead::identity(#storage::BlockDescriptor::store(&__cq_block));
        assert!(__cq_store == #storage::BlockRead::identity(__cq_source.0),
            "block descriptor belongs to another block store");
        let __cq_block_id = #storage::BlockDescriptor::id(&__cq_block);
        #opened
        #storage::BlockDescriptor::rows(__cq_block)
            .filter(move |(row, _)| #storage::BlockSelection::contains(&__cq_source.1, __cq_block_id, *row))
            .map(move |(__cq_row, __cq_payload)| {
                #update
                (__cq_store, __cq_block_id, __cq_row, __cq_payload)
            })
    })
}

fn work_binding(work: Option<&syn::Expr>) -> Option<TokenStream> {
    work.map(|work| quote!(let __cq_work = (#work);))
}
