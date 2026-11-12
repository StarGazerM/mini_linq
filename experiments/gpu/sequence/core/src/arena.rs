//! Caller-owned candidate publication for checked GPU Sequence programs.

use cq_lang_sequence::gpu::publication::ArenaRows;
use cq_lang_sequence::{Analysis, Program};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};

/// Caller-owned arena and optional validated runtime output layout.
pub struct ArenaEmission {
    pub arena: syn::Expr,
    pub layout: Option<syn::Expr>,
}

/// Translate a raw candidate terminal into caller-owned arena storage.
///
/// The arena expression is evaluated once and mutably borrowed. Intermediate
/// publications retain their recorded protocols; terminal canon, folds, groups,
/// and host-only publications require a separate supported publication boundary.
// Formal rule GPU-Arena: Publish the recorded raw-candidate arena encoding into a retained caller-owned lease.
pub fn translate_arena(
    physical: &cq_lang_structures::physical::Program,
    structures: &cq_lang_structures::Program,
    program: &Program,
    analysis: &Analysis,
    arena: &syn::Expr,
) -> syn::Result<syn::Expr> {
    translate_emission(physical, structures, program, analysis, &ArenaEmission {
        arena: arena.clone(),
        layout: None,
    })
}

/// Emit the checked Sequence publication using a caller-owned output layout.
pub fn translate_emission(
    physical: &cq_lang_structures::physical::Program,
    structures: &cq_lang_structures::Program,
    program: &Program,
    analysis: &Analysis,
    emission: &ArenaEmission,
) -> syn::Result<syn::Expr> {
    crate::emit::translate_impl(physical, structures, program, analysis, Some(emission))
}

pub(crate) struct Packing {
    fields: Vec<String>,
    columns: Vec<TokenStream>,
    pub bits: TokenStream,
    dynamic: bool,
}

impl Packing {
    /// Print the recorded GPU-Arena encoding; each field's width and
    /// signedness is its recorded carrier's `Scalar::COLUMN`.
    pub fn new(record: &ArenaRows, dynamic: bool) -> Self {
        let fields = record
            .columns
            .iter()
            .map(|field| field.column.clone())
            .collect();
        let columns: Vec<_> = record
            .columns
            .iter()
            .map(|field| crate::runs::scalar(&field.carrier))
            .collect();
        let bits = quote!((0u32 #(+ #columns.bits())*));
        Self {
            fields,
            columns,
            bits,
            dynamic,
        }
    }

    /// The answer columns the packing writes, in recorded field order.
    pub fn fields(&self) -> &[String] {
        &self.fields
    }

    /// Bind the arena and make its pool the execution pool: a hole runs on its
    /// arena's stream (a DL-WaveLanes lane, or the program stream), reading
    /// inputs that share the arena pool's memory.
    pub fn setup(&self, emission: &ArenaEmission) -> TokenStream {
        let arena = &emission.arena;
        if let Some(layout) = &emission.layout {
            let arity = self.fields.len();
            let columns = &self.columns;
            return quote! {
                let __cq_arena_layout = #layout;
                let __cq_arena_columns: &[::gpu_datalog_runtime::Column] = &[#(#columns),*];
                if __cq_arena_layout.packing().len() != #arity
                    || __cq_arena_layout.packing().packed_fields()
                        .zip(__cq_arena_columns)
                        .any(|(field, column)| (field.signed != 0) != column.is_signed())
                {
                    return Err("GPU arena layout differs from the terminal schema".into());
                }
                let __cq_arena = &mut (#arena);
                if !__cq_arena.pool().shares_memory(&__cq_pool) {
                    return Err("GPU arena and inputs must share a pool's memory".into());
                }
                let __cq_pool = __cq_arena.pool().clone();
                let __cq_arena_packing = __cq_arena_layout.packing().clone();
            };
        }
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

    /// Publish a failed answer gate without reading query inputs or launching.
    pub fn empty(&self, emission: &ArenaEmission) -> TokenStream {
        let arena = &emission.arena;
        if let Some(layout) = &emission.layout {
            return quote! {{
                let __cq_arena_layout = #layout;
                let __cq_arena = &mut (#arena);
                let packing = __cq_arena_layout.packing().clone();
                unsafe { __cq_arena.reserve(packing, 0)?.finish(0, ::std::vec::Vec::new()) }
            }};
        }
        let columns = &self.columns;
        quote! {{
            let __cq_arena = &mut (#arena);
            let packing = ::gpu_datalog_runtime::Packing::with_columns(&[#(#columns),*])?;
            unsafe { __cq_arena.reserve(packing, 0)?.finish(0, ::std::vec::Vec::new()) }
        }}
    }

    /// Pack one answer row; each value is a row slot or a capture kernel scalar.
    pub fn write(&self, values: &[TokenStream]) -> TokenStream {
        if self.dynamic {
            return self.write_dynamic(values);
        }
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

    pub fn parameters(&self) -> Vec<(syn::Ident, syn::Type)> {
        if !self.dynamic { return Vec::new(); }
        let mut parameters = vec![
            (format_ident!("__cq_arena_bits"), syn::parse_quote!(u32)),
            (format_ident!("__cq_arena_lanes"), syn::parse_quote!(usize)),
        ];
        for index in 0..self.fields.len() {
            parameters.extend([
                (format_ident!("__cq_arena_width_{index}"), syn::parse_quote!(u32)),
                (format_ident!("__cq_arena_min_{index}"), syn::parse_quote!(i64)),
                (format_ident!("__cq_arena_max_{index}"), syn::parse_quote!(i64)),
            ]);
        }
        parameters
    }

    pub fn arguments(&self) -> Vec<(syn::Ident, TokenStream)> {
        if !self.dynamic { return Vec::new(); }
        let mut arguments = vec![
            (format_ident!("__cq_arena_bits"), quote!(__cq_arena_layout.packing().bits())),
            (format_ident!("__cq_arena_lanes"), quote!((__cq_arena_layout.carrier_bits() / 64) as usize)),
        ];
        for index in 0..self.fields.len() {
            arguments.extend([
                (format_ident!("__cq_arena_width_{index}"), quote!(__cq_arena_layout.width(#index))),
                (format_ident!("__cq_arena_min_{index}"), quote!(__cq_arena_layout.bounds()[#index].map_or(1, |(minimum, _)| minimum))),
                (format_ident!("__cq_arena_max_{index}"), quote!(__cq_arena_layout.bounds()[#index].map_or(0, |(_, maximum)| maximum))),
            ]);
        }
        arguments
    }

    fn write_dynamic(&self, values: &[TokenStream]) -> TokenStream {
        let parts = values.iter().zip(&self.columns).enumerate().map(|(index, (value, column))| {
            let width = format_ident!("__cq_arena_width_{index}");
            let minimum = format_ident!("__cq_arena_min_{index}");
            let maximum = format_ident!("__cq_arena_max_{index}");
            quote! {
                let raw = (#value) as i64;
                if raw < #minimum || raw > #maximum {
                    ::cq_gpu_sequence_runtime::device::flag_overflow(__meta.add(2));
                }
                let width = #width;
                __shift -= width;
                let logical_lane = (__shift / 64) as usize;
                let lane = if __lanes <= 2 { logical_lane } else { __lanes - 1 - logical_lane };
                let local = __shift % 64;
                let mask = u64::MAX >> (64 - width);
                let sign = if #column.is_signed() { 1u64 << (width - 1) } else { 0 };
                let value = (raw as u64 & mask) ^ sign;
                *__packed.add(lane) |= value << local;
                if local != 0 && local + width > 64 {
                    let next = if __lanes <= 2 { logical_lane + 1 } else { __lanes - 2 - logical_lane };
                    *__packed.add(next) |= value >> (64 - local);
                }
            }
        });
        quote! {
            let __lanes = __cq_arena_lanes;
            let mut __shift = __cq_arena_bits;
            let __packed = __output_keys.add(__destination as usize * __lanes);
            for lane in 0..__lanes { *__packed.add(lane) = 0; }
            #(#parts)*
        }
    }
}
