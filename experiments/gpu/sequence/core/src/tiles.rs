//! Render the checked resident FOR tile refinement; storage and predicate
//! decisions remain in GPU realization records.
use std::collections::{BTreeMap, BTreeSet};
use cq_lang_sequence::gpu::realization::{Side, SourceRealization, TileScan};
use cq_lang_structures::physical::{Operator, symbol_name};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Ident, Type};

pub(crate) struct TileLoop {
    pub prologue: TokenStream,
    pub predicates: TokenStream,
    pub live_frame: Option<Ident>,
    pub vector_rows: u32,
    pub positions_per_thread: u32,
}

pub(crate) struct Printed {
    pub tiles: TileLoop,
    pub load: TokenStream,
    pub parameters: Vec<(Ident, Type)>,
    pub arguments: Vec<(Ident, TokenStream)>,
    pub uses: BTreeSet<String>,
}

pub(crate) fn print(
    rows: &crate::rows::Rows,
    source_name: &str,
    source: &SourceRealization,
    tiles: &TileScan,
    maps: &[(String, String)],
    terminal: &BTreeSet<usize>,
) -> syn::Result<Printed> {
    let length = rows.source_len(source_name)?;
    let mut prologue = TokenStream::new();
    let mut parameters = Vec::new();
    let mut definitions = Vec::new();
    for (name, input) in maps {
        definitions.extend(rows.map_definitions(name, input)?.into_iter().rev());
    }
    let mut demanded: BTreeSet<_> = tiles.tests.iter().map(|test| test.column.clone())
        .chain(tiles.reads.iter().map(|read| read.column.clone())).collect();
    let mut slots = terminal.clone();
    for definition in &definitions {
        if let Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) = &definition.operator {
            slots.extend(select.condition.columns().iter().filter_map(|field| rows.columns.get(field)).copied());
        }
    }
    demanded.extend(source.columns.iter().filter(|(field, _)| rows.columns.get(*field)
        .is_some_and(|slot| slots.contains(slot))).map(|(_, column)| column.clone()));
    let mut arguments = Vec::new();
    let mut uses = BTreeSet::new();
    let mut frames: BTreeMap<String, Ident> = BTreeMap::new();
    // Source aliases share their checked row slot; a frame is leased once.
    let mut field_frames = BTreeMap::new();
    for (field, column) in source.columns.iter().filter(|(_, column)| demanded.contains(*column)) {
        let Some(&slot) = rows.columns.get(field) else { continue };
        let frame = if let Some(frame) = frames.get(column) {
            frame.clone()
        } else {
            let view = rows.tile_view(source_name, field)?;
            let frame = format_ident!("__cq_tile_frame_{}", frames.len());
            prologue.extend(quote! {
                let #frame = ::cq_gpu_runtime::tile::frame(#view, __cq_tile_vector);
            });
            frames.insert(column.clone(), frame.clone());
            frame
        };
        field_frames.insert(slot, frame);
    }
    let mut tests = BTreeMap::new();
    let mut mask_tests = BTreeMap::new();
    for (index, test) in tiles.tests.iter().enumerate() {
        let frame = &frames[&test.column];
        let low = format_ident!("__cq_tile_low_{index}");
        let high = format_ident!("__cq_tile_high_{index}");
        let low_inclusive = format_ident!("__cq_tile_low_inclusive_{index}");
        let high_inclusive = format_ident!("__cq_tile_high_inclusive_{index}");
        let window = format_ident!("__cq_tile_window_{index}");
        prologue.extend(quote! {
            let mut #low = i64::MIN;
            let mut #high = i64::MAX;
            let mut #low_inclusive = true;
            let mut #high_inclusive = true;
        });
        for (bound_index, bound) in test.bounds.iter().enumerate() {
            let parameter = format_ident!("__cq_tile_bound_{index}_{bound_index}");
            let value = &bound.value;
            let inclusive = bound.inclusive;
            parameters.push((parameter.clone(), syn::parse_quote!(i64)));
            arguments.push((parameter.clone(), quote!(if (#length) != 0 { (#value) as i64 } else { 0i64 })));
            uses.insert(parameter.to_string());
            match bound.side {
                Side::Lower => prologue.extend(quote! {
                    if #parameter > #low { #low = #parameter; #low_inclusive = #inclusive; }
                    else if #parameter == #low { #low_inclusive &= #inclusive; }
                }),
                Side::Upper => prologue.extend(quote! {
                    if #parameter < #high { #high = #parameter; #high_inclusive = #inclusive; }
                    else if #parameter == #high { #high_inclusive &= #inclusive; }
                }),
            }
        }
        prologue.extend(quote! {
            let #window = ::cq_gpu_runtime::tile::window(#frame, #low, #low_inclusive, #high, #high_inclusive);
        });
        tests.insert(test.definition.clone(), quote! {
            if __live {
                __live = ::cq_gpu_runtime::tile::window_lanes(
                    #frame, __tid, 1u8 << __cq_tile_k, #window,
                    ::cq_gpu_runtime::tile::RawPayload) != 0;
            }
        });
        mask_tests.insert(test.definition.clone(), quote! {
            __cq_tile_mask = ::cq_gpu_runtime::tile::window_lanes(
                #frame, __tid, __cq_tile_mask, #window, ::cq_gpu_runtime::tile::RawPayload);
        });
    }
    let mapped: BTreeSet<_> = definitions.iter().map(|d| symbol_name(&d.name)).collect();
    let mut load = TokenStream::new();
    let mut predicates = TokenStream::new();
    let mut prefix = BTreeSet::new();
    // A leading run of recorded tests acts on all four bits together. Stop
    // before any row operation: no predicate moves across an authored consumer.
    for test in &tiles.tests {
        if !mapped.contains(&test.definition) {
            predicates.extend(mask_tests[&test.definition].clone());
            prefix.insert(test.definition.clone());
        }
    }
    for definition in &definitions {
        let name = symbol_name(&definition.name);
        if let Some(test) = mask_tests.get(&name) {
            predicates.extend(test.clone());
            prefix.insert(name);
        } else if !matches!(definition.operator, Operator::Rename(_) | Operator::Project(_)) {
            break;
        }
    }
    let read = |slot: usize, frame: &Ident| quote! {
        if __live {
            __row[#slot] = ::cq_gpu_runtime::tile::read(
                #frame, __cq_tile_physical, ::cq_gpu_runtime::tile::RawPayload);
        }
    };
    let mut loaded = BTreeSet::new();
    for definition in definitions {
        let name = symbol_name(&definition.name);
        if prefix.contains(&name) { continue; }
        if let Some(test) = tests.get(&name) {
            load.extend(test.clone());
            continue;
        }
        for demand in tiles.reads.iter().filter(|r| r.consumer == name && r.first_load) {
            let slot = rows.columns[&demand.field];
            // DECODE R: checked TileRead rust_reads demand authorizes this survivor first load.
            load.extend(read(slot, &frames[&demand.column]));
            loaded.insert(slot);
        }
        if let Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) = &definition.operator {
            for field in select.condition.columns() {
                if let Some(&slot) = rows.columns.get(&field)
                    && let Some(frame) = field_frames.get(&slot)
                    && loaded.insert(slot) {
                    // DECODE V: P6 residual relational predicates reconstruct only their demanded survivor fields.
                    load.extend(read(slot, frame));
                }
            }
        }
        if let Some(input) = definition.operator.input() {
            load.extend(rows.map(&name, &symbol_name(input), quote!(__row), quote!(__live))?);
        }
    }
    for slot in terminal.difference(&loaded) {
        // DECODE V: P6 direct aggregate source operands reconstruct surviving fold inputs.
        if let Some(frame) = field_frames.get(slot) { load.extend(read(*slot, frame)); }
    }
    Ok(Printed {
        tiles: TileLoop { prologue, predicates, live_frame: frames.values().next().cloned(), vector_rows: tiles.vector_rows, positions_per_thread: tiles.positions_per_thread },
        load, parameters, arguments, uses,
    })
}
