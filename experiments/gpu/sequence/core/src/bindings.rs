//! Catalog-backed GPU provider realization; printers only consume these facts.
use cq_lang_sequence::gpu::bindings::{
    Bindings, DirectoryPolicy, NativeLookup, ProviderAbi, ProviderLayout, RadixParams, RadixRead,
    RadixRealization,
};
use cq_lang_sequence::{Analysis, Program};
use cq_lang_structures::provider::NativeAbi;
use cq_lang_structures::{ReaderRef, Structure, physical::symbol_name};
use std::collections::{BTreeMap, BTreeSet};

fn error(message: &str) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message)
}

/// The device carrier of a recorded Physical column: codec columns are u32 codes.
fn carrier(
    analysis: &Analysis,
    codecs: &crate::runs::Codecs,
    name: &str,
    column: &str,
) -> syn::Result<syn::Type> {
    let ty = analysis
        .structures
        .physical
        .rows
        .get(name)
        .and_then(|row| row.types.get(column))
        .ok_or_else(|| error("GPU-RadixProvider: column lacks a recorded Physical type"))?;
    Ok(if codecs.get(ty).is_some() {
        syn::parse_quote!(u32)
    } else {
        ty.clone()
    })
}

/// Whether the Sequence lookup `(name, keys)` is the bound-prefix read of a
/// PH-Smallest probe, which its recorded Candidates read realizes instead.
fn smallest_reader(program: &Program, name: &str, keys: &[String]) -> bool {
    use cq_lang_structures::physical::{Choice, Operator};
    program
        .structures
        .physical
        .program
        .definitions
        .iter()
        .any(|definition| match &definition.operator {
            Operator::Probe(_, keyed) if matches!(keyed.choice, Some(Choice::Smallest(_))) => {
                let mut columns: Vec<_> = keyed.columns.iter().map(symbol_name).collect();
                columns.sort();
                symbol_name(&keyed.right) == name && columns == keys
            }
            _ => false,
        })
}

/// Every read one radix layout of `name` (key order `keys`) serves: the
/// Sequence lookups whose keys are a leading prefix of its keys, and the
/// PH-Smallest sides `[prefix][column]` it supplies.
fn radix_reads(
    program: &Program,
    analysis: &Analysis,
    name: &str,
    keys: &[String],
) -> Vec<(Vec<String>, RadixRead)> {
    let mut reads = Vec::new();
    for (provider, lookup) in analysis.capabilities.keys() {
        let leading: BTreeSet<_> = keys.iter().take(lookup.len()).collect();
        if provider != name
            || lookup.len() > keys.len()
            || lookup.iter().collect::<BTreeSet<_>>() != leading
            || smallest_reader(program, name, lookup)
        {
            continue;
        }
        let form = if lookup.len() == keys.len() {
            RadixRead::Group
        } else {
            RadixRead::Prefix
        };
        reads.push((lookup.clone(), form));
    }
    for candidate in cq_lang_structures::physical::candidate_prefixes(
        &program.structures.physical,
        &analysis.structures.physical,
    ) {
        for (side, prefix) in &candidate.sides {
            let mut leading = keys[..keys.len().saturating_sub(1)].to_vec();
            leading.sort();
            if side == name
                && keys.last() == Some(&candidate.column)
                && leading == *prefix
            {
                reads.push((
                    prefix.clone(),
                    RadixRead::Candidates {
                        column: candidate.column.clone(),
                    },
                ));
            }
        }
    }
    reads.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    reads.dedup();
    reads
}

/// Formal rule GPU-RadixProvider: `K`/`V` lanes are the recorded carriers
/// of the key and payload columns; `W`/`O`/params are the instance's advice.
fn radix_realization(
    program: &Program,
    analysis: &Analysis,
    codecs: &crate::runs::Codecs,
    name: &str,
    provider: &cq_lang_structures::Provider,
    keys: &[String],
    payload: &[String],
) -> syn::Result<RadixRealization> {
    let advice = provider
        .radix
        .as_ref()
        .ok_or_else(|| error("GPU-RadixProvider: radix instance lacks its advice"))?;
    Ok(RadixRealization {
        key_types: keys
            .iter()
            .map(|column| carrier(analysis, codecs, name, column))
            .collect::<syn::Result<_>>()?,
        lanes: payload
            .iter()
            .map(|column| carrier(analysis, codecs, name, column))
            .collect::<syn::Result<_>>()?,
        word: advice.word.clone(),
        offset: advice.offset.clone(),
        params: RadixParams {
            ratio: advice.ratio,
            radix_bits: advice.radix_bits,
            merge_tile_rows: advice.merge_tile,
        },
        reads: radix_reads(program, analysis, name, keys),
    })
}

/// GPU-RadixProvider checker: the recorded carriers, advice and read forms
/// agree with their sources; the key word is never chosen here.
fn check_radix(
    program: &Program,
    analysis: &Analysis,
    codecs: &crate::runs::Codecs,
    name: &str,
    provider: &cq_lang_structures::Provider,
    layout: &ProviderLayout,
    radix: &RadixRealization,
) -> bool {
    let Some(advice) = &provider.radix else {
        return false;
    };
    let carriers = |columns: &[String], types: &[syn::Type]| {
        columns.len() == types.len()
            && columns.iter().zip(types).all(|(column, ty)| {
                carrier(analysis, codecs, name, column).is_ok_and(|expected| expected == *ty)
            })
    };
    let mut reads = radix.reads.clone();
    reads.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
    reads.dedup();
    !layout.keys.is_empty()
        && carriers(&layout.keys, &radix.key_types)
        && carriers(&layout.payload, &radix.lanes)
        && radix.word == advice.word
        && radix.offset == advice.offset
        && radix.params
            == RadixParams {
                ratio: advice.ratio,
                radix_bits: advice.radix_bits,
                merge_tile_rows: advice.merge_tile,
            }
        && reads.len() == radix.reads.len()
        && reads == radix_reads(program, analysis, name, &layout.keys)
}

/// Formal rule GPU-ProviderLayout: catalog selection records construction ABI,
/// complete native permutations, download adapters and directory policy upstream.
pub(crate) fn select(program: &Program, analysis: &Analysis) -> syn::Result<Bindings> {
    let mut result = Bindings::default();
    let catalog = crate::catalog();
    let scanned: BTreeSet<_> = analysis
        .dispatches
        .iter()
        .map(|dispatch| {
            analysis
                .results
                .get(&dispatch.outer_store)
                .cloned()
                .unwrap_or_else(|| dispatch.outer_store.clone())
        })
        .collect();
    let context = cq_print::Context::new(&program.structures, &analysis.structures);
    let codecs = crate::runs::Codecs::check(&program.structures)?;
    for (name, layouts) in &analysis.structures.built {
        let mut recorded = Vec::new();
        for (levels, provider) in layouts {
            let ty = &provider.ty;
            let keys: Vec<_> = levels.iter().flatten().cloned().collect();
            let (abi, directory) = match analysis.structures.facts(provider).abi {
                Some(native @ (NativeAbi::Range | NativeAbi::Runs)) => {
                    catalog
                        .providers
                        .iter()
                        .find(|token| token.provider.same_token(provider))
                        .ok_or_else(|| {
                            error("GPU-ProviderLayout: built provider has no catalog token")
                        })?;
                    match native {
                        NativeAbi::Range => (
                            ProviderAbi::Range,
                            Some(DirectoryPolicy::DenseU32 {
                                terminal_entries: 1,
                            }),
                        ),
                        NativeAbi::Runs => (ProviderAbi::Runs, None),
                        _ => unreachable!(),
                    }
                }
                Some(NativeAbi::Radix) => {
                    catalog
                        .providers
                        .iter()
                        .find(|token| token.provider.same_token(provider))
                        .ok_or_else(|| {
                            error("GPU-ProviderLayout: built provider has no catalog token")
                        })?;
                    (ProviderAbi::Radix, None)
                }
                _ => (ProviderAbi::Host, None),
            };
            let payload = if scanned.contains(name) || abi != ProviderAbi::Range {
                analysis.structures.layouts[name].clone()
            } else {
                let mut payload = context.built_payload(name, levels);
                payload.retain(|column| keys.first() != Some(column));
                payload.extend(keys.iter().skip(1).cloned());
                payload.sort();
                payload.dedup();
                payload
            };
            let radix = (abi == ProviderAbi::Radix)
                .then(|| radix_realization(program, analysis, &codecs, name, provider, &keys, &payload))
                .transpose()?;
            recorded.push(ProviderLayout {
                ty: ty.clone(),
                abi,
                keys,
                payload,
                directory,
                radix,
            });
        }
        result.providers.insert(name.clone(), recorded);
    }
    let definitions: BTreeMap<_, _> = program
        .structures
        .physical
        .program
        .definitions
        .iter()
        .map(|d| (symbol_name(&d.name), d.clone()))
        .collect();
    let mut native_hosts = BTreeSet::new();
    for entry in &program.structures.structures.entries {
        let Structure::Host { reader, keyed, .. } = &entry.structure else {
            continue;
        };
        let name = symbol_name(&entry.name);
        if let Some(host) = analysis.structures.physical.hosts.get(&name) {
            if analysis.structures.facts(&keyed.provider).abi == Some(NativeAbi::Runs) {
                native_hosts.insert(host.clone());
            }
            if matches!(reader, ReaderRef::Scan)
                && matches!(
                    analysis.structures.facts(&keyed.provider).abi,
                    Some(NativeAbi::Runs | NativeAbi::Columns)
                )
            {
                result
                    .downloads
                    .insert(name, syn::parse_quote!(::core::iter::Map<_, _>));
            }
        }
    }
    for name in definitions.keys() {
        let native_lookup = analysis.structures.built.get(name).is_some_and(|layouts| {
            layouts.iter().any(|(_, provider)| {
                analysis.structures.facts(provider).abi == Some(NativeAbi::Runs)
            })
        }) || analysis.structures.fallbacks.iter().any(
            |((owner, reader, _), provider)| {
                owner == name
                    && !matches!(reader, ReaderRef::Scan)
                    && analysis.structures.facts(provider).abi == Some(NativeAbi::Runs)
            },
        );
        if !native_lookup {
            continue;
        }
        let Some(origin) = crate::rows::origin(name, &definitions)? else {
            continue;
        };
        if !native_hosts.contains(&origin.host) || origin.selection.is_some() {
            continue;
        }
        let source = analysis.structures.host_layouts[&origin.host]
            .iter()
            .map(|column| {
                origin
                    .columns
                    .iter()
                    .find(|(_, original)| *original == column)
                    .map(|(name, _)| name.clone())
                    .ok_or_else(|| error("GPU-Runs: supplied layout lost a packed column"))
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let mut record = |id: String, keys: Vec<String>| -> syn::Result<()> {
            if keys.len() > source.len() || keys.iter().any(|key| !source.contains(key)) {
                return Err(error("GPU-Runs: native key absent from source"));
            }
            let leading = source[..keys.len()].iter().all(|key| keys.contains(key));
            let payload = if leading {
                source.clone()
            } else {
                keys.iter()
                    .cloned()
                    .chain(
                        source
                            .iter()
                            .filter(|column| !keys.contains(column))
                            .cloned(),
                    )
                    .collect()
            };
            let permutation = (!leading).then(|| {
                payload
                    .iter()
                    .map(|column| source.iter().position(|c| c == column).unwrap())
                    .collect()
            });
            let materialized_provider =
                (!leading).then(|| syn::parse_quote!(::gpu_datalog_runtime::PackedProvider));
            result.native.insert(
                id,
                NativeLookup {
                    host: origin.host.clone(),
                    source: source.clone(),
                    keys,
                    payload,
                    permutation,
                    materialized_provider,
                },
            );
            Ok(())
        };
        if let Some(layouts) = analysis.structures.built.get(name) {
            if layouts.len() == 1
                && layouts[0].1.input.is_some()
                && analysis.structures.facts(&layouts[0].1).abi == Some(NativeAbi::Runs)
            {
                record(name.clone(), layouts[0].0.concat())?;
            }
        } else {
            for ((owner, reader, keys), provider) in &analysis.structures.fallbacks {
                if owner != name
                    || matches!(reader, ReaderRef::Scan)
                    || analysis.structures.facts(provider).abi != Some(NativeAbi::Runs)
                    || provider.input.is_none()
                {
                    continue;
                }
                let mut sorted = keys.clone();
                sorted.sort();
                record(format!("{name}__{}", sorted.join("__")), keys.clone())?;
            }
        }
    }
    crate::domains::select(program, analysis, &mut result)?;
    check(program, analysis, &result)?;
    Ok(result)
}

/// Validate recorded facts without selecting providers or replaying selection.
pub(crate) fn check(program: &Program, analysis: &Analysis, records: &Bindings) -> syn::Result<()> {
    let catalog = crate::catalog();
    crate::domains::check(program, analysis, records)?;
    let context = cq_print::Context::new(&program.structures, &analysis.structures);
    let codecs = crate::runs::Codecs::check(&program.structures)?;
    if records.providers.len() != analysis.structures.built.len() {
        return Err(error("GPU-ProviderLayout: incomplete provider records"));
    }
    for (name, layouts) in &records.providers {
        let source = analysis
            .structures
            .built
            .get(name)
            .ok_or_else(|| error("GPU-ProviderLayout: unknown provider"))?;
        if source.len() != layouts.len() {
            return Err(error("GPU-ProviderLayout: missing layout"));
        }
        for (layout, (levels, provider)) in layouts.iter().zip(source) {
            let ty = &provider.ty;
            if &layout.ty != ty
                || layout.keys != levels.concat()
                || layout
                    .payload
                    .iter()
                    .any(|column| !analysis.structures.layouts[name].contains(column))
            {
                return Err(error("GPU-ProviderLayout: stale provider schema"));
            }
            let scanned = analysis.dispatches.iter().any(|dispatch| {
                analysis
                    .results
                    .get(&dispatch.outer_store)
                    .unwrap_or(&dispatch.outer_store)
                    == name
            });
            if scanned || layout.abi != ProviderAbi::Range {
                if layout.payload != analysis.structures.layouts[name] {
                    return Err(error(
                        "GPU-ProviderLayout: scan payload differs from its complete layout",
                    ));
                }
            } else {
                let needed: BTreeSet<_> = context
                    .built_payload(name, levels)
                    .into_iter()
                    .filter(|column| layout.keys.first() != Some(column))
                    .chain(layout.keys.iter().skip(1).cloned())
                    .collect();
                let supplied: BTreeSet<_> = layout.payload.iter().cloned().collect();
                if supplied != needed || supplied.len() != layout.payload.len() {
                    return Err(error(
                        "GPU-ProviderLayout: lookup payload does not exactly reconstruct its demands",
                    ));
                }
            }
            let valid = match layout.abi {
                ProviderAbi::Range => {
                    catalog.providers.iter().any(|token| {
                        token.provider.same_token(provider)
                            && analysis.structures.facts(provider).abi == Some(NativeAbi::Range)
                    }) && layout.directory
                        == Some(DirectoryPolicy::DenseU32 {
                            terminal_entries: 1,
                        })
                }
                ProviderAbi::Runs => {
                    analysis.structures.facts(provider).abi == Some(NativeAbi::Runs)
                        && provider.input.is_some()
                        && catalog
                            .providers
                            .iter()
                            .any(|token| token.provider.same_token(provider))
                        && layout.directory.is_none()
                }
                ProviderAbi::Radix => {
                    analysis.structures.facts(provider).abi == Some(NativeAbi::Radix)
                        && catalog.providers.iter().any(|token| token.provider.same_token(provider))
                        && layout.directory.is_none()
                        && layout.radix.as_ref().is_some_and(|radix| {
                            check_radix(program, analysis, &codecs, name, provider, layout, radix)
                        })
                }
                ProviderAbi::Host => {
                    layout.directory.is_none()
                        && !matches!(
                            analysis.structures.facts(provider).abi,
                            Some(NativeAbi::Range | NativeAbi::Runs | NativeAbi::Radix)
                        )
                }
            };
            if (layout.abi == ProviderAbi::Radix) != layout.radix.is_some() {
                return Err(error("GPU-RadixProvider: radix facts recorded off a radix layout"));
            }
            if !valid {
                return Err(error("GPU-ProviderLayout: provider catalog ABI mismatch"));
            }
        }
    }
    for lookup in records.native.values() {
        if lookup.source.len() != lookup.payload.len()
            || lookup.keys.len() > lookup.payload.len()
            || lookup.payload[..lookup.keys.len()]
                .iter()
                .any(|key| !lookup.keys.contains(key))
        {
            return Err(error("GPU-NativeAbi: invalid lookup schema"));
        }
        if let Some(order) = &lookup.permutation {
            if lookup.materialized_provider
                != Some(syn::parse_quote!(::gpu_datalog_runtime::PackedProvider))
            {
                return Err(error("GPU-NativeAbi: unrecorded materialization provider"));
            }
            let unique: BTreeSet<_> = order.iter().copied().collect();
            if unique != (0..lookup.source.len()).collect()
                || order.len() != lookup.source.len()
                || order
                    .iter()
                    .zip(&lookup.payload)
                    .any(|(index, column)| &lookup.source[*index] != column)
            {
                return Err(error("GPU-NativeAbi: incomplete provider permutation"));
            }
        } else if lookup.source != lookup.payload || lookup.materialized_provider.is_some() {
            return Err(error(
                "GPU-NativeAbi: borrowed lookup changed provider or order",
            ));
        }
    }
    let required_downloads: BTreeSet<_> = program
        .structures
        .structures
        .entries
        .iter()
        .filter_map(|entry| {
            if let Structure::Host {
                reader: ReaderRef::Scan,
                keyed,
                ..
            } = &entry.structure
                && matches!(
                    analysis.structures.facts(&keyed.provider).abi,
                    Some(NativeAbi::Runs | NativeAbi::Columns)
                )
            {
                Some(symbol_name(&entry.name))
            } else {
                None
            }
        })
        .collect();
    if records.downloads.keys().cloned().collect::<BTreeSet<_>>() != required_downloads {
        return Err(error(
            "GPU-NativeAbi: download adapter records are incomplete",
        ));
    }
    for (name, ty) in &records.downloads {
        if *ty != syn::parse_quote!(::core::iter::Map<_, _>)
            || !program.structures.structures.entries.iter().any(|entry| {
                symbol_name(&entry.name) == *name
                    && matches!(
                        &entry.structure,
                        Structure::Host {
                            reader: ReaderRef::Scan,
                            ..
                        }
                    )
            })
        {
            return Err(error("GPU-NativeAbi: invalid download adapter"));
        }
    }
    Ok(())
}
