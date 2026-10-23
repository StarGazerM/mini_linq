//! Collective reduction templates; the caller places these at the recorded
//! partition boundary, never at an implementation stripe boundary.

use cq_lang_sequence::Monoid;
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
}

/// Lower the machine instance of BE-Group at an already published canonical
/// valuation boundary. No sorting/canonicalization choices are made here.
pub fn group(
    group: &cq_lang_structures::syntax::algebra::Group,
    input_columns: &[String],
    output_columns: &[String],
    output_types: &std::collections::BTreeMap<String, syn::Type>,
    input_index: usize,
    kernel: &Ident,
    direct: Option<GroupInput>,
) -> syn::Result<(TokenStream, TokenStream)> {
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
    let prepare = format_ident!("prepare_{kernel}");
    let mut columns = group.keys.iter().map(symbol_name).collect::<Vec<_>>();
    let keys = columns.len();
    let mut arguments = columns.iter().cloned().map(Some).collect::<Vec<_>>();
    let mut descriptors = Vec::new();
    for aggregate in &group.aggregates {
        let kind = match aggregate.kind {
            AggregateKind::Count => 0u32,
            AggregateKind::Sum => 1,
            AggregateKind::Min => 2,
            AggregateKind::Max => 3,
        };
        let ty = if kind >= 2 {
            let syn::Type::Path(path) = &aggregate.ty else {
                return Err(syn::Error::new_spanned(
                    &aggregate.ty,
                    "GPU extrema require Option<T>",
                ));
            };
            let syn::PathArguments::AngleBracketed(args) =
                &path.path.segments.last().unwrap().arguments
            else {
                return Err(syn::Error::new_spanned(
                    &aggregate.ty,
                    "GPU extrema require Option<T>",
                ));
            };
            let Some(syn::GenericArgument::Type(ty)) = args.args.first() else {
                return Err(syn::Error::new_spanned(
                    &aggregate.ty,
                    "GPU extrema require Option<T>",
                ));
            };
            ty
        } else {
            &aggregate.ty
        };
        let spelling = quote!(#ty).to_string();
        let (min, max) = match spelling.as_str() {
            "i64" => (quote!(i64::MIN), quote!(i64::MAX)),
            "i32" | "i16" | "i8" => (quote!(<#ty>::MIN as i64), quote!(<#ty>::MAX as i64)),
            "u32" | "u16" | "u8" => (quote!(0i64), quote!(<#ty>::MAX as i64)),
            "usize" if kind == 0 => (quote!(0i64), quote!(i64::MAX)),
            _ => {
                return Err(syn::Error::new_spanned(
                    ty,
                    "GPU group requires an integer codec fitting i64",
                ));
            }
        };
        descriptors.push(
            quote!(::cq_gpu_sequence_runtime::group::Aggregate {kind:#kind,min:#min,max:#max}),
        );
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
            let offset = crate::emit::packed_offset(quote!(packed), quote!(position), quote!(lanes), quote!(#shift), quote!(#bits));
            writes.push(quote! {
                let offset = #offset;
                *values.add(position*#width+#column)=#base.wrapping_add(offset as i64);
            });
        } else {
            writes.push(quote!(*values.add(position*#width+#column)=1i64;));
        }
    }
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
        decoded.push(if optional {
            quote!(if valid[#slot]==0 {None} else {Some(::cq_gpu_runtime::decode_value(values[#slot], &__cq_dictionary)?)})
        } else {
            quote!(::cq_gpu_runtime::decode_value::<#ty>(values[#slot], &__cq_dictionary)?)
        });
    }
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
    Ok((
        quote! {
            #[kernel]
            #[launch_bounds(256)]
            #[launch_contract(domain=1,block=(256,1,1))]
            pub fn #kernel(values:*mut i64,length:usize,#input_parameters #(#params),*) {
                unsafe {
                    let mut position=::cuda_device::thread::index_1d().get() as usize;
                    while position<length {
                        let mut __live = true;
                        #load
                        #live_write
                        if __live { #(#writes)* }
                        position+=::cuda_device::thread::gridDim_x() as usize*256;
                    }
                }
            }
        },
        quote! {
            let __cq_group_values=unsafe {::cq_gpu_runtime::Buffer::<i64>::uninitialized(__cq_pool.clone(),#length.checked_mul(#width).ok_or("group allocation overflow")?)?};
            #live_buffer
            if #length!=0 {
                let prepared=__cq_module.#prepare(::cuda_core::LaunchConfig1D::new(#length.div_ceil(256).min(2272) as u32,256,0))?;
                __cq_module.#kernel(__cq_pool.stream(),&prepared,__cq_group_values.ptr(),#length,#input_arguments #(#args),*)?;
            }
            #status
            let __cq_result=unsafe {::cq_gpu_sequence_runtime::group::fold(
                __cq_pool.clone(),&__cq_group_values,#live_argument,#length,#keys,&[#(#descriptors),*],__cq_statuses,
            )?}.with_decoder(
                move |values:&[i64],valid:&[u8]| -> ::cq_gpu_sequence_runtime::Result<_> {Ok((#(#decoded,)*))}
            );
            #release_live
        },
    ))
}
