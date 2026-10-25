//! Caller-owned candidate publication for checked GPU Sequence programs.

use cq_lang_sequence::{Analysis, Program};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Translate a raw candidate terminal into caller-owned arena storage.
///
/// The arena expression is evaluated once and mutably borrowed. Intermediate
/// publications retain their recorded protocols; terminal canon, folds, groups,
/// and host-only publications require a separate supported publication boundary.
// Formal rule GPU-Arena: Publish fixed declared head-column encodings into a retained caller-owned lease.
pub fn translate_arena(
    program: &Program,
    analysis: &Analysis,
    arena: &syn::Expr,
) -> syn::Result<syn::Expr> {
    crate::emit::translate_impl(program, analysis, Some(arena))
}

pub(crate) struct Packing {
    pub types: Vec<syn::Type>,
    widths: Vec<u32>,
    signed: Vec<bool>,
    columns: Vec<syn::Ident>,
    pub bits: u32,
}

impl Packing {
    pub fn new(types: Vec<syn::Type>) -> syn::Result<Self> {
        let mut widths = Vec::with_capacity(types.len());
        let mut signed = Vec::with_capacity(types.len());
        let mut columns = Vec::with_capacity(types.len());
        let mut bits = 0u32;
        for ty in &types {
            let name = match ty {
                syn::Type::Path(path) if path.qself.is_none() => path.path.get_ident().map(ToString::to_string),
                _ => None,
            };
            let (width, sign, column) = match name.as_deref() {
                Some("u8") => (8, false, "U8"),
                Some("i8") => (8, true, "I8"),
                Some("u16") => (16, false, "U16"),
                Some("i16") => (16, true, "I16"),
                Some("u32") => (32, false, "U32"),
                Some("i32") => (32, true, "I32"),
                Some("i64") => (64, true, "I64"),
                _ => return Err(syn::Error::new_spanned(ty,
                    "GPU arena output requires declared i8/i16/i32/i64 or u8/u16/u32 scalar columns")),
            };
            bits = bits.checked_add(width).ok_or_else(|| {
                syn::Error::new_spanned(ty, "GPU arena head packing width exceeds u32")
            })?;
            widths.push(width);
            signed.push(sign);
            columns.push(format_ident!("{column}"));
        }
        Ok(Self { types, widths, signed, columns, bits })
    }

    pub fn setup(&self, arena: &syn::Expr) -> TokenStream {
        let columns = &self.columns;
        quote! {
            let __cq_arena = &mut (#arena);
            if !::std::sync::Arc::ptr_eq(__cq_arena.pool(), &__cq_pool) {
                return Err("GPU arena and inputs must share a pool and stream".into());
            }
            let __cq_arena_packing = ::gpu_datalog_runtime::Packing::with_columns(
                &[#(::gpu_datalog_runtime::Column::#columns),*])?;
        }
    }

    pub fn write(&self, slots: &[usize]) -> TokenStream {
        let lanes = self.bits.div_ceil(64).max(1) as usize;
        let physical_lane = |lane: usize| if lanes <= 2 { lane } else { lanes - 1 - lane };
        let mut shift = self.bits;
        let parts = slots.iter().zip(&self.types).zip(&self.widths).zip(&self.signed)
            .map(|(((slot, ty), width), signed)| {
                shift -= width;
                let lane = physical_lane((shift / 64) as usize);
                let local = shift % 64;
                let mask = u64::MAX >> (64 - width);
                let sign = if *signed { 1u64 << (width - 1) } else { 0 };
                let spill = if local != 0 && local + width > 64 {
                    let next = physical_lane((shift / 64) as usize + 1);
                    let right = 64 - local;
                    quote!(*__packed.add(#next) |= value >> #right;)
                } else { TokenStream::new() };
                quote! {
                    let value = ((__row[#slot] as #ty as u64) & #mask) ^ #sign;
                    *__packed.add(#lane) |= value << #local;
                    #spill
                }
            }).collect::<Vec<_>>();
        quote! {
            let __packed = __output_keys.add(__destination as usize * #lanes);
            for lane in 0..#lanes { *__packed.add(lane) = 0; }
            #(#parts)*
        }
    }
}
