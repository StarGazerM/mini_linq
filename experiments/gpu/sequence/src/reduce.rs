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
