//! Caller-owned candidate publication for checked GPU Sequence programs.

use cq_lang_sequence::{Analysis, Program};
use proc_macro2::TokenStream;
use quote::quote;

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
    columns: Vec<TokenStream>,
    pub bits: TokenStream,
}

impl Packing {
    pub fn new(types: Vec<syn::Type>) -> syn::Result<Self> {
        let columns: Vec<_> = types.iter().map(crate::runs::scalar).collect();
        let bits = quote!((0u32 #(+ #columns.bits())*));
        Ok(Self { columns, bits })
    }

    /// Bind the arena and make its pool the execution pool: a hole runs on its
    /// arena's stream (a DL-WaveLanes lane, or the program stream), reading
    /// inputs that share the arena pool's memory.
    pub fn setup(&self, arena: &syn::Expr) -> TokenStream {
        let columns = &self.columns;
        quote! {
            let __cq_arena = &mut (#arena);
            if !__cq_arena.pool().shares_memory(&__cq_pool) {
                return Err("GPU arena and inputs must share a pool's memory".into());
            }
            let __cq_pool = __cq_arena.pool().clone();
            let __cq_arena_packing = ::gpu_datalog_runtime::Packing::with_columns(
                &[#(#columns),*])?;
        }
    }

    /// Pack one answer row; each value is a row slot or a capture kernel scalar.
    pub fn write(&self, values: &[TokenStream]) -> TokenStream {
        let bits = &self.bits;
        let parts = values.iter().zip(&self.columns)
            .map(|(value, column)| {
                quote! {
                    let width = #column.bits();
                    __shift -= width;
                    let logical_lane = (__shift / 64) as usize;
                    let lane = if __lanes <= 2 { logical_lane } else { __lanes - 1 - logical_lane };
                    let local = __shift % 64;
                    let mask = u64::MAX >> (64 - width);
                    let sign = if #column.is_signed() { 1u64 << (width - 1) } else { 0 };
                    let value = ((#value) as u64 & mask) ^ sign;
                    *__packed.add(lane) |= value << local;
                    if local != 0 && local + width > 64 {
                        let next = if __lanes <= 2 { logical_lane + 1 } else { __lanes - 2 - logical_lane };
                        *__packed.add(next) |= value >> (64 - local);
                    }
                }
            }).collect::<Vec<_>>();
        quote! {
            let __lanes = (#bits).div_ceil(64).max(1) as usize;
            let mut __shift = #bits;
            let __packed = __output_keys.add(__destination as usize * __lanes);
            for lane in 0..__lanes { *__packed.add(lane) = 0; }
            #(#parts)*
        }
    }
}
