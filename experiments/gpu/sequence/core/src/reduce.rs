//! Collective reduction templates; the caller places these at the recorded
//! partition boundary, never at an implementation stripe boundary.
//!
//! DESIGN translator-gpu-reduce
//! Owns collective templates, not reduction placement (translator-decides-nothing, T-Gpu).
//! Source: doc/PLANNING-REDESIGN.md Architecture; doc/formal/CONFORMANCE.md GPU-FoldCompletion.
//! Consume the recorded partition boundary and completion/partial-combine facts.
//! DO NOT fold once per implementation stripe, invent extra reductions, or pick a grouping strategy.

use cq_lang_sequence::Monoid;
use cq_lang_sequence::gpu::realization::{Auxiliary, BlockGeometry};
use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

pub struct Reduction {
    pub declare: TokenStream,
    pub update: TokenStream,
    pub finish: TokenStream,
}

pub fn build(
    monoid: &Monoid,
    input_slot: Option<usize>,
    output_slot: usize,
    shared_offset: usize,
    accumulator: &Ident,
) -> syn::Result<Reduction> {
    let value = match monoid {
        Monoid::Count | Monoid::Or => quote!(1i64),
        Monoid::Sum(column) => {
            let slot = input_slot.ok_or_else(|| {
                syn::Error::new_spanned(column, "T-Gpu: sum lacks a checked input column slot")
            })?;
            quote!(__row[#slot] as i64)
        }
    };
    let update = if matches!(monoid, Monoid::Or) {
        quote!(if __live { #accumulator = 1; })
    } else {
        quote! {
            if __live {
                match #accumulator.checked_add(#value) {
                    Some(__sum) => #accumulator = __sum,
                    None => ::cq_gpu_sequence_runtime::device::flag_overflow(__meta.add(2)),
                }
            }
        }
    };
    let combine = if matches!(monoid, Monoid::Or) {
        quote!(*__values.add(__tid) |= *__values.add(__tid + __stride);)
    } else {
        quote! {
            match (*__values.add(__tid)).checked_add(*__values.add(__tid + __stride)) {
                Some(__sum) => *__values.add(__tid) = __sum,
                None => ::cq_gpu_sequence_runtime::device::flag_overflow(__meta.add(2)),
            }
        }
    };
    Ok(Reduction {
        declare: quote!(let mut #accumulator = 0i64;),
        update,
        finish: quote! {
            {
                let __values = __shared.add(#shared_offset).cast::<i64>();
                *__values.add(__tid) = #accumulator;
                ::cuda_device::thread::sync_threads();
                let mut __stride = 128usize;
                while __stride != 0 {
                    if __tid < __stride { #combine }
                    ::cuda_device::thread::sync_threads();
                    __stride /= 2;
                }
                __row[#output_slot] = *__values;
                __live = __tid == 0;
                // Every lane has consumed the result before scratch reuse.
                ::cuda_device::thread::sync_threads();
            }
        },
    })
}

pub struct GroupInput {
    pub parameters: Vec<TokenStream>,
    pub arguments: Vec<TokenStream>,
    pub load: TokenStream,
    pub columns: std::collections::BTreeMap<String, TokenStream>,
    pub length: TokenStream,
    /// A recorded tile source (P6 C): the fold iterates its vectors and the
    /// surviving lanes instead of a grid-stride position loop.
    pub tiles: Option<crate::tiles::TileLoop>,
}

/// Lower the machine instance of BE-Group at an already published canonical
/// valuation boundary. No sorting/canonicalization choices are made here.
pub fn group(
    group: &cq_lang_structures::syntax::algebra::Group,
    provider: Option<&syn::Type>,
    input_columns: &[String],
    output_columns: &[String],
    output_types: &std::collections::BTreeMap<String, syn::Type>,
    input_index: usize,
    kernel: &Ident,
    direct: Option<GroupInput>,
    codecs: &crate::runs::Codecs,
    captures: &cq_lang_structures::syntax::Captures,
    geometry: &BlockGeometry,
    rows: &crate::rows::Rows,
) -> syn::Result<(TokenStream, TokenStream)> {
    let threads = proc_macro2::Literal::u32_unsuffixed(geometry.block_threads);
    let shared = proc_macro2::Literal::u32_unsuffixed(geometry.shared_bytes);
    use cq_lang_structures::{physical::symbol_name, syntax::algebra::AggregateKind};
    use quote::format_ident;
    let owner = format_ident!("__cq_canonical_{input_index}");
    let fields = format_ident!("__cq_fields_{input_index}");
    let length = direct
        .as_ref()
        .map(|input| input.length.clone())
        .unwrap_or_else(|| {
            let id = format_ident!("__cq_store_length_{input_index}");
            quote!(#id)
        });
    if direct.as_ref().is_some_and(|input| input.tiles.is_some()) {
        return Err(syn::Error::new_spanned(group, "T-Gpu: a tile source feeds only a keyless fused fold"));
    }
    if let Some(key) = group
        .keys
        .iter()
        .find(|key| captures.contains(&symbol_name(key)))
    {
        return Err(syn::Error::new_spanned(
            key,
            "T-Gpu: a capture group key has no GPU realization",
        ));
    }
    let mut columns = group.keys.iter().map(symbol_name).collect::<Vec<_>>();
    let keys = columns.len();
    let fold = match (keys, provider) {
        (0, None) => quote!(::cq_gpu_sequence_runtime::group::fold),
        (0, Some(_)) => {
            return Err(syn::Error::new_spanned(
                group,
                "GPU-Group: global fold cannot carry a keyed accumulator provider",
            ));
        }
        (_, Some(provider)) => quote!(<#provider>::fold),
        (_, None) => {
            return Err(syn::Error::new_spanned(
                group,
                "GPU-Group: keyed fold lacks a recorded accumulator provider",
            ));
        }
    };
    let mut arguments = columns.iter().cloned().map(Some).collect::<Vec<_>>();
    let mut descriptors = Vec::new();
    for aggregate in &group.aggregates {
        let kind = match aggregate.kind {
            AggregateKind::Count => 0u32,
            AggregateKind::Sum => 1,
            AggregateKind::Min => 2,
            AggregateKind::Max => 3,
        };
        let ty = &aggregate.ty;
        let optional = kind >= 2;
        descriptors.push(quote!(
            ::cq_gpu_sequence_runtime::group::aggregate::<#ty, #optional>(#kind)?
        ));
        columns.push(symbol_name(&aggregate.name));
        arguments.push(if kind == 0 {
            None
        } else {
            let expression = aggregate
                .expression
                .as_ref()
                .expect("checked aggregate argument");
            Some(symbol_name(
                cq_lang_structures::syntax::rust::variable(expression).ok_or_else(|| {
                    syn::Error::new_spanned(
                        expression,
                        "GPU aggregate argument must be a normalized variable",
                    )
                })?,
            ))
        });
    }
    let width = columns.len();
    let mut params = direct
        .as_ref()
        .map(|input| input.parameters.clone())
        .unwrap_or_default();
    let mut args = direct
        .as_ref()
        .map(|input| input.arguments.clone())
        .unwrap_or_default();
    let mut writes = Vec::new();
    for (column, argument) in arguments.iter().enumerate() {
        if let Some(argument) = argument {
            // IF-Capture: a capture argument is one host value, passed as a kernel scalar.
            if let Some((parameter, value)) = capture_argument(captures, argument, column) {
                params.push(quote!(#parameter:i64));
                args.push(value);
                writes.push(quote!(*values.add(position*#width+#column)=#parameter;));
                continue;
            }
            if let Some(input) = &direct {
                let value = input.columns.get(argument).ok_or_else(|| {
                    syn::Error::new_spanned(group, "GPU resident group argument is absent")
                })?;
                writes.push(quote!(*values.add(position*#width+#column)=#value as i64;));
                continue;
            }
            let field = input_columns
                .iter()
                .position(|name| name == argument)
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        group,
                        "GPU group input lacks a canonical valuation column",
                    )
                })?;
            let base = format_ident!("base_{column}");
            let shift = format_ident!("shift_{column}");
            let bits = format_ident!("bits_{column}");
            params.extend([quote!(#base:i64), quote!(#shift:u32), quote!(#bits:u32)]);
            args.extend([
                quote!(#fields[#field].base),
                quote!(#fields[#field].shift),
                quote!(#fields[#field].bits),
            ]);
            let offset = crate::emit::packed_offset(
                quote!(packed),
                quote!(position),
                quote!(lanes),
                quote!(#shift),
                quote!(#bits),
            );
            // DECODE V: P6 grouped input reconstructs packed FOR fields before relational grouping and aggregation.
            writes.push(quote! {
                let offset = #offset;
                *values.add(position*#width+#column)=#base.wrapping_add(offset as i64);
            });
        } else {
            writes.push(quote!(*values.add(position*#width+#column)=1i64;));
        }
    }
    let decoded = decoders(group, &columns, output_columns, output_types, codecs)?;
    let filtered = direct.is_some();
    let (load, input_parameters, input_arguments, status, live_argument) =
        if let Some(input) = direct {
            (
                input.load,
                quote!(live:*mut u32,),
                quote!(__cq_group_live.cu_deviceptr() as *mut u32,),
                quote!(),
                quote!(Some(&__cq_group_live)),
            )
        } else {
            (
                quote!(),
                quote!(packed:*const u64,lanes:usize,),
                quote!(#owner.keys().ptr(),#owner.lanes(),),
                quote!(__cq_statuses.push(#owner.metadata().clone());),
                quote!(None),
            )
        };
    let live_write = filtered.then(|| quote!(*live.add(position)=__live as u32;));
    let live_buffer = filtered.then(|| quote! {
        let __cq_group_live=unsafe {::cuda_core::DeviceBuffer::<u32>::uninitialized_async(__cq_pool.stream(),#length)?};
    });
    let release_live = filtered.then(|| {
        quote!(unsafe {
            __cq_group_live.drop_async(__cq_pool.stream())?;
        })
    });
    let abi = crate::emit::CheckedKernel::lower(kernel, quote! {
        #[kernel]
        #[launch_bounds(#threads)]
        #[launch_contract(domain=1,block=(#threads,1,1))]
    }, quote!(values:*mut i64,length:usize,#input_parameters #(#params),*), quote! {
                unsafe {
                    let mut position=::cuda_device::thread::index_1d().get() as usize;
                    while position<length {
                        let mut __live = true;
                        #load
                        #live_write
                        if __live { #(#writes)* }
                        position+=::cuda_device::thread::gridDim_x() as usize*#threads;
                    }
                }
    }, rows, cq_lang_sequence::gpu::realization::Launch::InvocationOccupancy, geometry.shared_bytes)?;
    let shape = abi.prepare(quote!(::cuda_core::LaunchConfig1D::new(1,#threads,#shared)));
    let prepare = abi.prepare(quote!(::cuda_core::LaunchConfig1D::new(grid,#threads,#shared)));
    let call = abi.call(quote!(__cq_pool.stream()), quote!(&prepared), quote!(grid),
        quote!(__cq_group_values.ptr(),#length,#input_arguments #(#args),*))?;
    Ok((
        abi.definition(),
        quote! {
            let __cq_group_values=unsafe {::cq_gpu_runtime::Buffer::<i64>::uninitialized(__cq_pool.clone(),#length.checked_mul(#width).ok_or("group allocation overflow")?)?};
            #live_buffer
            if #length!=0 {
                let shape = #shape?;
                let grid=::cq_gpu_runtime::launch_grid_for_kernel(shape.function(),#length.div_ceil(#threads),#threads,#shared)?;
                let prepared = #prepare?;
                #call?;
            }
            #status
            let __cq_result=unsafe {#fold(
                __cq_pool.clone(),&__cq_group_values,#live_argument,#length,#keys,&[#(#descriptors),*],__cq_statuses,&__cq_auxiliary,
            )?}.with_decoder(
                move |values:&[i64],valid:&[u8]| -> ::cq_gpu_sequence_runtime::Result<_> {Ok((#(#decoded,)*))}
            );
            #release_live
        },
    ))
}

/// A capture aggregate argument's kernel scalar and its host value, read once
/// through the capture binder bound in the host setup (IF-Capture).
fn capture_argument(
    captures: &cq_lang_structures::syntax::Captures,
    argument: &str,
    column: usize,
) -> Option<(Ident, TokenStream)> {
    let binder = captures
        .binders()
        .find(|binder| cq_lang_structures::physical::symbol_name(binder) == argument)?;
    let parameter = quote::format_ident!("__cq_capture_argument_{column}");
    Some((
        parameter,
        quote!((::core::clone::Clone::clone(#binder) as i64)),
    ))
}

/// Decode each grouped output column from its machine value and validity.
fn decoders(
    group: &cq_lang_structures::syntax::algebra::Group,
    columns: &[String],
    output_columns: &[String],
    output_types: &std::collections::BTreeMap<String, syn::Type>,
    codecs: &crate::runs::Codecs,
) -> syn::Result<Vec<TokenStream>> {
    use cq_lang_structures::{physical::symbol_name, syntax::algebra::AggregateKind};
    let mut decoded = Vec::new();
    for column in output_columns {
        let slot = columns
            .iter()
            .position(|name| name == column)
            .ok_or_else(|| syn::Error::new_spanned(group, "GPU grouped output column is absent"))?;
        let ty = &output_types[column];
        let optional = group.aggregates.iter().any(|aggregate| {
            symbol_name(&aggregate.name) == *column
                && matches!(aggregate.kind, AggregateKind::Min | AggregateKind::Max)
        });
        // DECODE P: grouped answer decoder resolves symbols, nullable extrema and scalars only when the host requests logical rows.
        decoded.push(if let Some(codec) = codecs.column(column) {
            quote!(<#codec as ::gpu_datalog_runtime::SymbolCodec<#ty>>::decode(
                u32::try_from(values[#slot]).map_err(|_| "GPU symbol code exceeds u32")?
            ))
        } else if optional {
            quote!(if valid[#slot]==0 {None} else {Some(::cq_gpu_runtime::decode_value(values[#slot], &__cq_dictionary)?)})
        } else {
            quote!(::cq_gpu_runtime::decode_value::<#ty>(values[#slot], &__cq_dictionary)?)
        });
    }
    Ok(decoded)
}

/// Formal rule GPU-GroupGridFold: a keyless group folds its canonical
/// valuations in one fused pass: every occupancy-grid block (recorded block
/// geometry) reduces its grid-stride rows into typed per-aggregate partials
/// (with presence for extrema), and one stream-ordered block per aggregate
/// (recorded combine geometry) combines them into the single global row,
/// keeping the empty-input identity and checked overflow.
pub fn global(
    group: &cq_lang_structures::syntax::algebra::Group,
    input_columns: &[String],
    output_columns: &[String],
    output_types: &std::collections::BTreeMap<String, syn::Type>,
    input_index: usize,
    kernel: &Ident,
    direct: Option<GroupInput>,
    codecs: &crate::runs::Codecs,
    captures: &cq_lang_structures::syntax::Captures,
    geometry: &BlockGeometry,
    combine: &BlockGeometry,
    rows: &crate::rows::Rows,
) -> syn::Result<(TokenStream, TokenStream)> {
    let threads = proc_macro2::Literal::u32_unsuffixed(geometry.block_threads);
    let shared = proc_macro2::Literal::u32_unsuffixed(geometry.shared_bytes);
    let half = proc_macro2::Literal::usize_suffixed(geometry.block_threads as usize / 2);
    let presence = proc_macro2::Literal::usize_unsuffixed(geometry.block_threads as usize * 8);
    let combine_geometry = geometry_tokens(combine);
    use cq_lang_structures::{physical::symbol_name, syntax::algebra::AggregateKind};
    use quote::format_ident;
    if !group.keys.is_empty() {
        return Err(syn::Error::new_spanned(
            group,
            "GPU-GroupGridFold: keyed groups use the recorded accumulator",
        ));
    }
    let owner = format_ident!("__cq_canonical_{input_index}");
    let fields = format_ident!("__cq_fields_{input_index}");
    let length = direct
        .as_ref()
        .map(|input| input.length.clone())
        .unwrap_or_else(|| {
            let id = format_ident!("__cq_store_length_{input_index}");
            quote!(#id)
        });
    let mut params = direct
        .as_ref()
        .map(|input| input.parameters.clone())
        .unwrap_or_default();
    let mut args = direct
        .as_ref()
        .map(|input| input.arguments.clone())
        .unwrap_or_default();
    let mut columns = Vec::new();
    let mut descriptors = Vec::new();
    let mut declare = Vec::new();
    let mut update = Vec::new();
    let mut reduce = Vec::new();
    let width = group.aggregates.len();
    for (column, aggregate) in group.aggregates.iter().enumerate() {
        let (kind, identity) = match aggregate.kind {
            AggregateKind::Count => (0u32, quote!(0i64)),
            AggregateKind::Sum => (1, quote!(0i64)),
            AggregateKind::Min => (2, quote!(i64::MAX)),
            AggregateKind::Max => (3, quote!(i64::MIN)),
        };
        let ty = &aggregate.ty;
        let optional = kind >= 2;
        descriptors
            .push(quote!(::cq_gpu_sequence_runtime::group::aggregate::<#ty, #optional>(#kind)?));
        columns.push(symbol_name(&aggregate.name));
        let value = format_ident!("__cq_value_{column}");
        let present = format_ident!("__cq_present_{column}");
        declare.push(quote!(let mut #value = #identity; let mut #present = false;));
        let argument = if kind == 0 {
            quote!(1i64)
        } else {
            let expression = aggregate
                .expression
                .as_ref()
                .expect("checked aggregate argument");
            let name = symbol_name(
                cq_lang_structures::syntax::rust::variable(expression).ok_or_else(|| {
                    syn::Error::new_spanned(
                        expression,
                        "GPU aggregate argument must be a normalized variable",
                    )
                })?,
            );
            if let Some((parameter, value)) = capture_argument(captures, &name, column) {
                params.push(quote!(#parameter:i64));
                args.push(value);
                quote!(#parameter)
            } else if let Some(input) = &direct {
                let value = input.columns.get(&name).ok_or_else(|| {
                    syn::Error::new_spanned(group, "GPU resident group argument is absent")
                })?;
                quote!((#value as i64))
            } else {
                let field = input_columns
                    .iter()
                    .position(|column| *column == name)
                    .ok_or_else(|| {
                        syn::Error::new_spanned(
                            group,
                            "GPU group input lacks a canonical valuation column",
                        )
                    })?;
                let base = format_ident!("base_{column}");
                let shift = format_ident!("shift_{column}");
                let bits = format_ident!("bits_{column}");
                params.extend([quote!(#base:i64), quote!(#shift:u32), quote!(#bits:u32)]);
                args.extend([
                    quote!(#fields[#field].base),
                    quote!(#fields[#field].shift),
                    quote!(#fields[#field].bits),
                ]);
                let offset = crate::emit::packed_offset(
                    quote!(packed),
                    quote!(position),
                    quote!(lanes),
                    quote!(#shift),
                    quote!(#bits),
                );
                // DECODE V: P6 keyless grid fold reconstructs packed FOR fields before typed per-block partials.
                quote!({ let offset = #offset; #base.wrapping_add(offset as i64) })
            }
        };
        let combine = |into: TokenStream,
                       from: TokenStream,
                       from_present: TokenStream,
                       present: TokenStream| match kind {
            2 => {
                quote!(if #from_present && (!#present || #from < #into) { #into = #from; } #present |= #from_present;)
            }
            3 => {
                quote!(if #from_present && (!#present || #from > #into) { #into = #from; } #present |= #from_present;)
            }
            _ => quote! {
                match (#into).checked_add(#from) {
                    Some(sum) => #into = sum,
                    None => ::cq_gpu_sequence_runtime::device::flag_overflow(error),
                }
            },
        };
        let accumulate = combine(
            quote!(#value),
            quote!(__argument),
            quote!(true),
            quote!(#present),
        );
        update.push(quote!({ let __argument = #argument; #accumulate }));
        let shared_combine = combine(
            quote!(*values.add(__tid)),
            quote!(*values.add(__tid + stride)),
            quote!((*present.add(__tid + stride) != 0)),
            quote!(__present),
        );
        reduce.push(quote! {
            *values.add(__tid) = #value;
            *present.add(__tid) = #present as u32;
            ::cuda_device::thread::sync_threads();
            let mut stride = #half;
            while stride != 0 {
                if __tid < stride {
                    let mut __present = *present.add(__tid) != 0;
                    #shared_combine
                    *present.add(__tid) = __present as u32;
                }
                ::cuda_device::thread::sync_threads();
                stride /= 2;
            }
            if __tid == 0 {
                let block = ::cuda_device::thread::blockIdx_x() as usize;
                *partials.add(block * #width + #column) = *values;
                *partial_present.add(block * #width + #column) = *present;
            }
            ::cuda_device::thread::sync_threads();
        });
    }
    let decoded = decoders(group, &columns, output_columns, output_types, codecs)?;
    let tiles = direct.as_ref().and_then(|input| input.tiles.clone());
    let (load, input_parameters, input_arguments, status) = if let Some(input) = direct {
        (input.load, quote!(), quote!(), quote!())
    } else {
        (
            quote!(),
            quote!(packed:*const u64,lanes:usize,),
            quote!(#owner.keys().ptr(),#owner.lanes(),),
            quote!(__cq_statuses.push(#owner.metadata().clone());),
        )
    };
    // Formal rule GPU-TilePredicate: a tile source's block owns whole vectors;
    // each lane folds its surviving rows, so no position publication exists.
    let (rows_loop, parts) = match &tiles {
        Some(tiles) => {
            if tiles.lanes != geometry.block_threads {
                return Err(syn::Error::new_spanned(group, "GPU-TilePredicate: tile lanes differ from the fold geometry"));
            }
            let crate::tiles::TileLoop { prologue, vectors, rows_per_lane, vector_rows, .. } = tiles;
            let rows_per_lane = proc_macro2::Literal::u32_suffixed(*rows_per_lane);
            let lanes = proc_macro2::Literal::u32_suffixed(tiles.lanes);
            let vector_rows_value = proc_macro2::Literal::u64_suffixed(*vector_rows);
            let vector_rows = proc_macro2::Literal::usize_suffixed(*vector_rows as usize);
            (
                quote! {
                    let _ = length;
                    let __cq_tile_lane = __tid as u32;
                    let __cq_tile_vectors = #vectors;
                    let mut __cq_tile_v = ::cuda_device::thread::blockIdx_x() as u64;
                    while __cq_tile_v < __cq_tile_vectors {
                        #prologue
                        let mut __cq_tile_k = 0u32;
                        while __cq_tile_mask != 0 && __cq_tile_k < #rows_per_lane {
                            if __cq_tile_mask & (1u32 << __cq_tile_k) != 0 {
                                let __cq_tile_local = __cq_tile_k * #lanes + __cq_tile_lane;
                                let position = (__cq_tile_v * #vector_rows_value + __cq_tile_local as u64) as usize;
                                let _ = position;
                                let mut __live = true;
                                #load
                                if __live { #(#update)* }
                            }
                            __cq_tile_k += 1;
                        }
                        __cq_tile_v += ::cuda_device::thread::gridDim_x() as u64;
                    }
                },
                quote!(#length.div_ceil(#vector_rows)),
            )
        }
        None => (
            quote! {
                let mut position = ::cuda_device::thread::index_1d().get() as usize;
                while position < length {
                    let mut __live = true;
                    #load
                    if __live { #(#update)* }
                    position += ::cuda_device::thread::gridDim_x() as usize * #threads;
                }
            },
            quote!(#length.div_ceil(#threads)),
        ),
    };
    let abi = crate::emit::CheckedKernel::lower(kernel, quote! {
        #[kernel]
        #[launch_bounds(#threads)]
        #[launch_contract(domain=1,block=(#threads,1,1),dynamic_shared_range=(#shared,#shared))]
    }, quote!(partials:*mut i64,partial_present:*mut u32,error:*mut u32,length:usize,#input_parameters #(#params),*), quote! {
                unsafe {
                    let __tid = ::cuda_device::thread::threadIdx_x() as usize;
                    #(#declare)*
                    #rows_loop
                    let shared = ::cuda_device::DynamicSharedArray::<u8, 16>::get_raw();
                    let values = shared.cast::<i64>();
                    let present = shared.add(#presence).cast::<u32>();
                    #(#reduce)*
                }
    }, rows, cq_lang_sequence::gpu::realization::Launch::InvocationOccupancy, geometry.shared_bytes)?;
    let shape = abi.prepare(quote!(::cuda_core::LaunchConfig1D::new(1,#threads,#shared)));
    let prepare = abi.prepare(quote!(::cuda_core::LaunchConfig1D::new(grid,#threads,#shared)));
    let call = abi.call(quote!(__cq_pool.stream()), quote!(&prepared), quote!(grid),
        quote!(__cq_global.partials(),__cq_global.present(),__cq_global.error(),#length,#input_arguments #(#args),*))?;
    Ok((
        abi.definition(),
        quote! {
            let shape = #shape?;
            let grid = ::cq_gpu_runtime::launch_grid_for_kernel(shape.function(),#parts,#threads,#shared)?;
            let __cq_global = ::cq_gpu_sequence_runtime::group::GlobalFold::new(
                __cq_pool.clone(), grid as usize, &[#(#descriptors),*])?;
            let prepared = #prepare?;
            #call?;
            #status
            let __cq_result = unsafe { __cq_global.finish(__cq_statuses, #combine_geometry)? }.with_decoder(
                move |values:&[i64],valid:&[u8]| -> ::cq_gpu_sequence_runtime::Result<_> {Ok((#(#decoded,)*))}
            );
        },
    ))
}

/// The runtime value of a recorded kernel geometry.
pub(crate) fn geometry_tokens(geometry: &BlockGeometry) -> TokenStream {
    let threads = proc_macro2::Literal::u32_unsuffixed(geometry.block_threads);
    let shared = proc_macro2::Literal::u32_unsuffixed(geometry.shared_bytes);
    quote!(::cq_gpu_sequence_runtime::Geometry { block_threads: #threads, shared_bytes: #shared })
}

/// The runtime `Auxiliary` value of the recorded auxiliary geometry.
pub(crate) fn auxiliary_tokens(auxiliary: &Auxiliary) -> TokenStream {
    let Auxiliary {
        selection,
        offsets,
        range,
        canon,
        group,
        group_reduce,
        bounds,
    } = auxiliary;
    let [
        selection,
        offsets,
        range,
        canon,
        group,
        group_reduce,
        bounds,
    ] = [
        selection,
        offsets,
        range,
        canon,
        group,
        group_reduce,
        bounds,
    ]
    .map(geometry_tokens);
    quote!(::cq_gpu_sequence_runtime::Auxiliary {
        selection: #selection,
        offsets: #offsets,
        range: #range,
        canon: #canon,
        group: #group,
        group_reduce: #group_reduce,
        bounds: #bounds,
    })
}
