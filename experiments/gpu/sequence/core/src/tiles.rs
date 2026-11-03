//! Printing recorded cooperative tile scans (P6 C). The GPU realization's
//! `TileScan` and the Structures blocked records decide the leased columns,
//! the run's tests, summary use and identity read points; this module only
//! renders them. Closed operands enter once per invocation in the column's
//! ordinal domain; every vector derives its residual window on the device.
use std::collections::{BTreeMap, BTreeSet};

use cq_lang_sequence::gpu::realization::TileScan;
use cq_lang_sequence::{Analysis, Program};
use cq_lang_structures::blocks::{Strategy, Summary};
use cq_lang_structures::encoded::{Carrier, Family};
use cq_lang_structures::physical::{Definition, Operator, symbol_name};
use cq_lang_structures::syntax::constraint::{Comparison, Constraint, Term};
use cq_lang_structures::syntax::EqualityValue;
use proc_macro2::{Literal, TokenStream};
use quote::{format_ident, quote};
use syn::{Ident, Type};

fn error(message: impl std::fmt::Display) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message.to_string())
}

/// One tile source's device loop parts.
#[derive(Clone)]
pub(crate) struct TileLoop {
    /// Per-vector frames, live mask and recorded tests (`__cq_tile_v` bound).
    pub prologue: TokenStream,
    /// The source's vector count.
    pub vectors: TokenStream,
    pub lanes: u32,
    pub rows_per_lane: u32,
    pub vector_rows: u64,
}

pub(crate) struct Printed {
    pub tiles: TileLoop,
    /// Host checks, descriptors and closed-operand translation.
    pub setup: TokenStream,
    pub parameters: Vec<(Ident, Type, TokenStream)>,
    /// Predicates the prologue realizes; no row predicate prints them.
    pub tests: BTreeSet<String>,
    /// `(consumer, logical field)` → (vector frame, loads here).
    pub reads: BTreeMap<(String, String), (Ident, bool)>,
}

fn closed(term: &Term) -> syn::Result<TokenStream> {
    match term {
        Term::Closed(value) => Ok(quote!(#value)),
        Term::Literal(value) => Ok(quote!(#value)),
        Term::Variable(_) => Err(error("GPU-TilePredicate: a tile window bound is not closed")),
    }
}

/// Formal rules GPU-TilePredicate / GPU-NarrowCarrier: render one recorded
/// tile scan over the retained blocked host `host`.
pub(crate) fn print(
    program: &Program,
    analysis: &Analysis,
    name: &str,
    host: &Ident,
    tiles: &TileScan,
    bound: &BTreeMap<String, Definition>,
) -> syn::Result<Printed> {
    let encoded = &program.structures.encoded;
    let scan = &encoded.blocks.scans[name];
    let binding = &encoded.blocks.sources[&scan.source];
    let mut setup = TokenStream::new();
    let mut parameters = Vec::new();
    let mut frames = BTreeMap::new();
    let mut prologue = TokenStream::new();
    for column in &tiles.columns {
        let record = &binding.columns[column];
        let declaration = &encoded.obligations.declarations[&record.declaration];
        let slot = record.slot as usize;
        let family = match declaration.family {
            Family::Plain => quote!(Plain),
            Family::Bytes => quote!(Bytes),
            Family::IndexByteAddressable => return Err(error("GPU-TilePredicate: blocked column is not S1")),
        };
        let width = match declaration.carrier {
            Carrier::U8 => quote!(W8),
            Carrier::U16 => quote!(W16),
            Carrier::U32 => quote!(W32),
            Carrier::U64 => quote!(W64),
        };
        let (namespace, version) = cq_lang_structures::blocks::runtime_domain(declaration, record);
        let block_rows = binding.block_rows as usize;
        let summaries = binding.summary == Summary::MinMax;
        let descriptor = format_ident!("__cq_tile_descriptor_{slot}");
        let frame = format_ident!("__cq_tile_vector_{slot}");
        setup.extend(quote! {
            #host.check(#slot, ::cq_gpu_runtime::scan_table::ColumnFacts {
                family: ::cq_gpu_runtime::scan_table::FamilyId::#family,
                domain: ::cq_gpu_runtime::scan_table::Domain {
                    id: ::cq_gpu_runtime::scan_table::DomainId { namespace: #namespace, version: #version },
                    carrier: ::cq_gpu_runtime::scan_table::Width::#width,
                    equality: true,
                    order: true,
                },
                block_rows: #block_rows,
                summaries: #summaries,
            })?;
            // SAFETY: launched on the table's pool lane while the table lives.
            let #descriptor = unsafe { #host.descriptor(#slot) }?;
        });
        parameters.push((
            descriptor.clone(),
            syn::parse_quote!(::cq_gpu_runtime::scan::ScanDeviceDescriptor),
            quote!(#descriptor),
        ));
        prologue.extend(quote! {
            let #frame = unsafe { ::cq_gpu_runtime::tile::vector(&#descriptor, __cq_tile_v) };
        });
        frames.insert(column.clone(), (descriptor, frame));
    }
    let Some((first_descriptor, first_frame)) = frames.values().next().cloned() else {
        return Err(error("GPU-TilePredicate: a tile scan leases no tested or read column"));
    };
    prologue.extend(quote! {
        let mut __cq_tile_mask = unsafe { ::cq_gpu_runtime::tile::live(#first_frame, __cq_tile_lane) };
    });
    let lanes = Literal::u32_suffixed(tiles.block_threads);
    let rows_per_lane = Literal::u32_suffixed(tiles.rows_per_lane);
    let mut classify = TokenStream::new();
    let mut test_code = TokenStream::new();
    for (index, test) in tiles.tests.iter().enumerate() {
        let predicate = &encoded.blocks.predicates[&test.predicate];
        let (Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select)) =
            &bound[&test.predicate].operator
        else {
            return Err(error("GPU-TilePredicate: tile test is not a selection"));
        };
        let types = &analysis.structures.physical.rows[&symbol_name(&select.input)].types;
        let source = |logical: &str| -> syn::Result<&str> {
            let representation = predicate.columns.get(logical).ok_or_else(|| error("GPU-TilePredicate: tested column has no representation"))?;
            Ok(encoded.records.representations[representation].column.as_str())
        };
        match test.strategy {
            Strategy::Window | Strategy::Point => {
                let (logical, _) = predicate.columns.iter().next().ok_or_else(|| error("GPU-TilePredicate: unary test without a column"))?;
                let ty = &types[logical];
                let (descriptor, frame) = &frames[source(logical)?];
                let window = format_ident!("__cq_tile_window_{index}");
                let encode = |term: &Term| -> syn::Result<TokenStream> {
                    let value = closed(term)?;
                    Ok(quote!(::cq_gpu_runtime::tile::encode_operand::<#ty>(&(#value))))
                };
                let value = if test.strategy == Strategy::Window {
                    let (_, bounds) = select.condition.range_bounds().ok_or_else(|| error("GPU-TilePredicate: window test without range bounds"))?;
                    let side = |lower: bool| -> syn::Result<Vec<TokenStream>> {
                        bounds.iter().filter(|bound| bound.lower == lower).map(|bound| {
                            let code = encode(&bound.value)?;
                            let inclusive = bound.inclusive;
                            Ok(quote!((#code, #inclusive)))
                        }).collect()
                    };
                    let (lower, upper) = (side(true)?, side(false)?);
                    quote!(::cq_gpu_runtime::tile::window(&[#(#lower),*], &[#(#upper),*])?)
                } else {
                    let constants = cq_lang_structures::selection_constants(select);
                    let code = encode(constants.get(logical).ok_or_else(|| error("GPU-TilePredicate: point test without its constant"))?)?;
                    quote!(::cq_gpu_runtime::tile::point(#code)?)
                };
                setup.extend(quote!(let #window = #value;));
                parameters.push((window.clone(), syn::parse_quote!(::cq_gpu_runtime::tile::Window), quote!(#window)));
                let qualified = if test.summary {
                    let class = format_ident!("__cq_tile_class_{index}");
                    // Formal rule GPU-VectorSummary: summaries classify before any payload load.
                    classify.extend(quote! {
                        let #class = if __cq_tile_mask != 0 {
                            unsafe { ::cq_gpu_runtime::tile::classify(&#descriptor, __cq_tile_v, #window) }
                        } else {
                            ::cq_gpu_runtime::tile::DISQUALIFIED
                        };
                        if #class == ::cq_gpu_runtime::tile::DISQUALIFIED {
                            __cq_tile_mask = 0;
                        }
                    });
                    quote!(&& #class != ::cq_gpu_runtime::tile::QUALIFIED)
                } else {
                    TokenStream::new()
                };
                test_code.extend(quote! {
                    if __cq_tile_mask != 0 #qualified {
                        __cq_tile_mask = unsafe { ::cq_gpu_runtime::tile::test(
                            ::cq_gpu_runtime::tile::RawPayload, #frame, __cq_tile_lane, #window, __cq_tile_mask) };
                    }
                });
            }
            Strategy::Formula => {
                let code = |column: &str| format_ident!("__cq_tile_code_{index}_{column}");
                let mut reads = Vec::new();
                for logical in predicate.columns.keys() {
                    let id = code(logical);
                    let (_, frame) = &frames[source(logical)?];
                    reads.push(quote! {
                        let #id = unsafe { ::cq_gpu_runtime::tile::ordinal(
                            ::cq_gpu_runtime::tile::RawPayload, #frame, __cq_tile_local) };
                    });
                }
                let mut operands = 0usize;
                let mut compare = |left: &Term, comparison: Comparison, right: &Term| -> syn::Result<TokenStream> {
                    let column = match (left, right) {
                        (Term::Variable(column), _) | (_, Term::Variable(column)) => symbol_name(column),
                        _ => return Err(error("GPU-TilePredicate: formula comparison without a column")),
                    };
                    let ty = &types[&column];
                    let mut operand = |term: &Term| -> syn::Result<TokenStream> {
                        if let Term::Variable(column) = term {
                            let id = code(&symbol_name(column));
                            return Ok(quote!(#id));
                        }
                        let value = closed(term)?;
                        let id = format_ident!("__cq_tile_operand_{index}_{operands}");
                        operands += 1;
                        setup.extend(quote!(let #id = ::cq_gpu_runtime::tile::encode_operand::<#ty>(&(#value));));
                        parameters.push((id.clone(), syn::parse_quote!(u64), quote!(#id)));
                        Ok(quote!(#id))
                    };
                    let (left, right) = (operand(left)?, operand(right)?);
                    Ok(comparison.expression(quote!(&(#left)), quote!(&(#right))))
                };
                let mut tests = Vec::new();
                for equality in &select.condition.equalities {
                    let right = match &equality.right {
                        EqualityValue::Variable(variable) => Term::Variable(variable.clone()),
                        EqualityValue::Literal(value) => Term::Literal(Box::new(value.clone())),
                    };
                    tests.push(compare(&Term::Variable(equality.left.clone()), Comparison::Eq(Default::default()), &right)?);
                }
                for constraint in &select.condition.constraints {
                    tests.push(formula(constraint, &mut compare)?);
                }
                test_code.extend(quote! {
                    if __cq_tile_mask != 0 {
                        let mut __cq_tile_k = 0u32;
                        while __cq_tile_k < #rows_per_lane {
                            let __cq_tile_bit = 1u32 << __cq_tile_k;
                            if __cq_tile_mask & __cq_tile_bit != 0 {
                                let __cq_tile_local = __cq_tile_k * #lanes + __cq_tile_lane;
                                #(#reads)*
                                if !(true #(&& (#tests))*) {
                                    __cq_tile_mask &= !__cq_tile_bit;
                                }
                            }
                            __cq_tile_k += 1;
                        }
                    }
                });
            }
        }
    }
    prologue.extend(classify);
    prologue.extend(test_code);
    let reads = tiles
        .reads
        .iter()
        .map(|read| ((read.consumer.clone(), read.field.clone()), (frames[&read.column].1.clone(), read.load)))
        .collect();
    Ok(Printed {
        tiles: TileLoop {
            prologue,
            vectors: quote!(unsafe { ::cq_gpu_runtime::tile::vectors(&#first_descriptor) }),
            lanes: tiles.block_threads,
            rows_per_lane: tiles.rows_per_lane,
            vector_rows: tiles.vector_rows,
        },
        setup,
        parameters,
        tests: tiles.tests.iter().map(|test| test.predicate.clone()).collect(),
        reads,
    })
}

fn formula(
    constraint: &Constraint,
    compare: &mut impl FnMut(&Term, Comparison, &Term) -> syn::Result<TokenStream>,
) -> syn::Result<TokenStream> {
    Ok(match constraint {
        Constraint::Compare { left, comparison, right } => compare(left, *comparison, right)?,
        Constraint::Between { term, low, high } => {
            let low = compare(term, Comparison::Ge(Default::default()), low)?;
            let high = compare(term, Comparison::Le(Default::default()), high)?;
            quote!((#low) && (#high))
        }
        Constraint::In { term, literals } => {
            let tests = literals
                .iter()
                .map(|literal| compare(term, Comparison::Eq(Default::default()), &Term::Literal(Box::new(literal.clone()))))
                .collect::<syn::Result<Vec<_>>>()?;
            quote!(false #(|| (#tests))*)
        }
        Constraint::Or(arms) => {
            let arms = arms
                .iter()
                .map(|arm| {
                    let tests = arm.iter().map(|c| formula(c, compare)).collect::<syn::Result<Vec<_>>>()?;
                    Ok(quote!(true #(&& (#tests))*))
                })
                .collect::<syn::Result<Vec<_>>>()?;
            quote!(false #(|| (#arms))*)
        }
    })
}
