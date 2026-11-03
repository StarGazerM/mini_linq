//! Catalog-backed GPU provider realization; printers only consume these facts.
use cq_lang_sequence::gpu::bindings::{
    Bindings, DirectoryPolicy, KeyRecipe, Materialization, NativeLookup, PayloadRepresentation,
    ProviderAbi, ProviderLayout, RadixParams, RadixRead, RadixRealization, RadixRelationRead,
    RadixWordCapture, SourceBuild, SourceOwnership, SourceRowMap,
};
use cq_lang_sequence::{Analysis, Program};
use cq_lang_structures::provider::NativeAbi;
use cq_lang_structures::{ReaderRef, Structure, physical::symbol_name};
use std::collections::{BTreeMap, BTreeSet};

fn error(message: &str) -> syn::Error {
    syn::Error::new(proc_macro2::Span::call_site(), message)
}

/// The recorded device carrier of a column.
fn carrier(carriers: &BTreeMap<String, syn::Type>, column: &str) -> syn::Result<syn::Type> {
    carriers
        .get(column)
        .cloned()
        .ok_or_else(|| error("GPU-RadixProvider: column lacks a recorded device carrier"))
}

fn relation_read(provider: &cq_lang_structures::Provider, host: &str) -> syn::Result<RadixRelationRead> {
    let input = provider.input.as_ref().ok_or_else(|| error("GPU-RadixRelation: missing input schema"))?;
    let advice = provider.radix.as_ref().ok_or_else(|| error("GPU-RadixRelation: missing radix advice"))?;
    if input.key_count != input.columns.len() || advice.offset != syn::parse_quote!(u64) {
        return Err(error("GPU-RadixRelation: full-key set reads require all columns and u64 offsets"));
    }
    Ok(RadixRelationRead {
        provider: provider.clone(),
        key_types: input.columns.iter().map(|column| column.storage.clone()).collect(),
        word: advice.word.clone(),
        word_capture: RadixWordCapture::InputView { host: host.to_owned() },
        offset: advice.offset.clone(),
        params: RadixParams {
            ratio: advice.ratio, radix_bits: advice.radix_bits,
            merge_tile_rows: advice.merge_tile, block_threads: crate::BLOCK_THREADS,
        },
        fields: (0..input.columns.len()).collect(),
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
            if side == name && keys.last() == Some(&candidate.column) && leading == *prefix {
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
    carriers: &BTreeMap<String, syn::Type>,
    name: &str,
    provider: &cq_lang_structures::Provider,
    keys: &[String],
    payload: &[String],
) -> syn::Result<RadixRealization> {
    let advice = provider
        .radix
        .as_ref()
        .ok_or_else(|| error("GPU-RadixProvider: radix instance lacks its advice"))?;
    let fields = analysis.structures.layouts[name].clone();
    let positions = |columns: &[String]| -> syn::Result<Vec<usize>> {
        columns.iter().map(|column| fields.iter().position(|field| field == column)
            .ok_or_else(|| error("GPU-RadixProvider: source field is missing"))).collect()
    };
    let key_fields = positions(keys)?;
    let payload_fields = positions(payload)?;
    let row_map = SourceRowMap { source: name.to_owned() };
    Ok(RadixRealization {
        key_types: keys
            .iter()
            .map(|column| carrier(carriers, column))
            .collect::<syn::Result<_>>()?,
        lanes: payload
            .iter()
            .map(|column| carrier(carriers, column))
            .collect::<syn::Result<_>>()?,
        word: advice.word.clone(),
        offset: advice.offset.clone(),
        params: RadixParams {
            ratio: advice.ratio,
            radix_bits: advice.radix_bits,
            merge_tile_rows: advice.merge_tile,
            block_threads: crate::BLOCK_THREADS,
        },
        reads: radix_reads(program, analysis, name, keys),
        source_build: SourceBuild {
            source: name.to_owned(), fields,
            ownership: SourceOwnership::RetainedDescriptors,
            row_map: row_map.clone(),
        },
        key_recipe: KeyRecipe::DirectSourceOrder { fields: key_fields },
        payload_representation: PayloadRepresentation::RetainedSources {
            fields: payload_fields, row_map,
        },
    })
}

/// GPU-RadixProvider checker, independent of the read selection: carriers
/// are the recorded column carriers, `W`/`O`/params the instance's advice
/// at the collective block width. Every recorded read is legal: a Group or
/// Prefix read is a recorded capability lookup (not a PH-Smallest bound-prefix
/// read) whose sorted keys are the layout's leading keys, Group exactly when
/// all of them; a Candidates read is a PH-Smallest side of this provider whose
/// prefix is every key but the last, which is the candidate column. Every
/// such lookup and side of the layout is read exactly once.
fn check_radix(
    program: &Program,
    analysis: &Analysis,
    carriers: &BTreeMap<String, syn::Type>,
    name: &str,
    provider: &cq_lang_structures::Provider,
    layout: &ProviderLayout,
    radix: &RadixRealization,
) -> bool {
    let Some(advice) = &provider.radix else {
        return false;
    };
    let keys = &layout.keys;
    if keys.is_empty() {
        return false;
    }
    let source = &radix.source_build;
    if source.source != name
        || source.fields != analysis.structures.layouts[name]
        || source.ownership != SourceOwnership::RetainedDescriptors
        || source.row_map.source != name
    {
        return false;
    }
    let fields_match = |fields: &[usize], columns: &[String]| {
        fields.len() == columns.len()
            && fields.iter().zip(columns)
                .all(|(field, column)| source.fields.get(*field) == Some(column))
    };
    let KeyRecipe::DirectSourceOrder { fields: key_fields } = &radix.key_recipe;
    if !fields_match(key_fields, keys) {
        return false;
    }
    let PayloadRepresentation::RetainedSources { fields, row_map } =
        &radix.payload_representation;
    if !fields_match(fields, &layout.payload) || *row_map != source.row_map {
        return false;
    }
    let typed = |columns: &[String], types: &[syn::Type]| {
        columns.len() == types.len()
            && columns
                .iter()
                .zip(types)
                .all(|(column, ty)| carrier(carriers, column).is_ok_and(|expected| expected == *ty))
    };
    let leading = |lookup: &[String]| {
        lookup.len() <= keys.len()
            && keys[..lookup.len()].iter().collect::<BTreeSet<_>>()
                == lookup.iter().collect::<BTreeSet<_>>()
    };
    let mut prefix = keys[..keys.len() - 1].to_vec();
    prefix.sort();
    let candidates = cq_lang_structures::physical::candidate_prefixes(
        &program.structures.physical,
        &analysis.structures.physical,
    );
    let mut seen = BTreeSet::new();
    for (lookup, form) in &radix.reads {
        let sorted = lookup.windows(2).all(|pair| pair[0] < pair[1]);
        let legal = sorted
            && match form {
                RadixRead::Group | RadixRead::Prefix => {
                    leading(lookup)
                        && matches!(form, RadixRead::Group) == (lookup.len() == keys.len())
                        && analysis
                            .capabilities
                            .contains_key(&(name.to_owned(), lookup.clone()))
                        && !smallest_reader(program, name, lookup)
                }
                RadixRead::Candidates { column } => {
                    keys.last() == Some(column)
                        && prefix == *lookup
                        && candidates.iter().any(|candidate| {
                            candidate.column == *column
                                && candidate.sides.iter().any(|(side, side_prefix)| {
                                    side == name && side_prefix == lookup
                                })
                        })
                }
            };
        if !legal || !seen.insert(format!("{lookup:?}{form:?}")) {
            return false;
        }
    }
    let read = |lookup: &Vec<String>| {
        radix.reads.iter().any(|(recorded, form)| {
            recorded == lookup && matches!(form, RadixRead::Group | RadixRead::Prefix)
        })
    };
    let lookups_covered = analysis.capabilities.keys().all(|(owner, lookup)| {
        owner != name || !leading(lookup) || smallest_reader(program, name, lookup) || read(lookup)
    });
    let sides_covered = candidates.iter().all(|candidate| {
        candidate.sides.iter().all(|(side, side_prefix)| {
            side != name
                || keys.last() != Some(&candidate.column)
                || prefix != *side_prefix
                || radix.reads.contains(&(
                    side_prefix.clone(),
                    RadixRead::Candidates {
                        column: candidate.column.clone(),
                    },
                ))
        })
    });
    typed(keys, &radix.key_types)
        && typed(&layout.payload, &radix.lanes)
        && radix.word == advice.word
        && radix.offset == advice.offset
        && radix.params
            == RadixParams {
                ratio: advice.ratio,
                radix_bits: advice.radix_bits,
                merge_tile_rows: advice.merge_tile,
                block_threads: crate::BLOCK_THREADS,
            }
        && lookups_covered
        && sides_covered
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
    result.carriers = crate::domains::carriers(program, analysis)?;
    for (name, layouts) in &analysis.structures.built {
        let mut recorded = Vec::new();
        for (levels, provider) in layouts {
            let keys: Vec<_> = levels.iter().flatten().cloned().collect();
            let (abi, directory) = match analysis.structures.facts(provider).abi {
                Some(native @ (NativeAbi::Range | NativeAbi::RadixRelation)) => {
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
                        NativeAbi::RadixRelation => (ProviderAbi::RadixRelation, None),
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
                .then(|| {
                    radix_realization(
                        program,
                        analysis,
                        &result.carriers,
                        name,
                        provider,
                        &keys,
                        &payload,
                    )
                })
                .transpose()?;
            recorded.push(ProviderLayout {
                provider: provider.clone(),
                abi,
                keys,
                payload,
                directory,
                radix,
            });
        }
        result.providers.insert(name.clone(), recorded);
    }
    for (index, entry) in program.structures.structures.entries.iter().enumerate() {
        let (provider, adapter) = match &entry.structure {
            Structure::Rows { provider, .. } => (provider, "vec"),
            Structure::Built { keyed, .. } => (&keyed.provider, "sorted_run"),
            _ => continue,
        };
        if matches!(
            analysis.structures.facts(provider).abi,
            Some(NativeAbi::Range | NativeAbi::RadixRelation | NativeAbi::Radix)
        ) {
            result.host_materializations.insert(
                index,
                cq_lang_structures::provider::base::declaration(adapter).instance(),
            );
        }
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
            if analysis.structures.facts(&keyed.provider).abi == Some(NativeAbi::RadixRelation) {
                native_hosts.insert(host.clone());
                if matches!(reader, ReaderRef::Scan) {
                    let fact = relation_read(&keyed.provider, host)?;
                    if result.relation_inputs.insert(host.clone(), fact.clone()).is_some_and(|prior| prior != fact) {
                        return Err(error("GPU-RadixRelation: inconsistent input snapshots"));
                    }
                }
            }
            if matches!(reader, ReaderRef::Scan)
                && matches!(
                    analysis.structures.facts(&keyed.provider).abi,
                    Some(NativeAbi::RadixRelation | NativeAbi::Columns)
                )
            {
                result.downloads.insert(
                    name,
                    cq_lang_structures::provider::base::declaration("host_scan").instance(),
                );
            }
        }
    }
    for name in definitions.keys() {
        let native_lookup = analysis.structures.built.get(name).is_some_and(|layouts| {
            layouts.iter().any(|(_, provider)| {
                analysis.structures.facts(provider).abi == Some(NativeAbi::RadixRelation)
            })
        }) || analysis.structures.fallbacks.iter().any(
            |((owner, reader, _), provider)| {
                owner == name
                    && !matches!(reader, ReaderRef::Scan)
                    && analysis.structures.facts(provider).abi == Some(NativeAbi::RadixRelation)
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
                    .ok_or_else(|| error("GPU-RadixRelation: supplied layout lost a packed column"))
            })
            .collect::<syn::Result<Vec<_>>>()?;
        let mut record = |id: String, keys: Vec<String>| -> syn::Result<()> {
            if keys.len() > source.len() || keys.iter().any(|key| !source.contains(key)) {
                return Err(error("GPU-RadixRelation: native key absent from source"));
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
            let materialization = (!leading).then(|| {
                let input = &result.relation_inputs[&origin.host];
                let permutation: Vec<_> = payload.iter()
                    .map(|column| source.iter().position(|c| c == column).unwrap()).collect();
                let mut provider = input.provider.clone();
                let schema = provider.input.as_mut().expect("recorded relation schema");
                schema.columns = permutation.iter().map(|&index| schema.columns[index].clone()).collect();
                let key_types = &input.key_types;
                let active_bits = syn::parse_quote!(
                    0u32 #(+ <#key_types as ::gpu_datalog_runtime::Scalar>::COLUMN.bits())*
                );
                let types = permutation.iter().map(|&index| &input.key_types[index]);
                let word = &input.word;
                let offset = &input.offset;
                provider.ty = syn::parse_quote!(::gpu_datalog_runtime::RadixLayout<(#(#types,)*), (), #word, #offset, ()>);
                Materialization {
                    permutation, provider, params: input.params.clone(),
                    active_bits,
                    permutation_threads: crate::BLOCK_THREADS,
                }
            });
            result.native.insert(
                id,
                NativeLookup {
                    host: origin.host.clone(),
                    source: source.clone(),
                    keys,
                    radix: {
                        let mut fact = result.relation_inputs[&origin.host].clone();
                        fact.fields = payload.iter().map(|column| source.iter().position(|c| c == column).unwrap()).collect();
                        fact
                    },
                    payload,
                    materialization,
                },
            );
            Ok(())
        };
        if let Some(layouts) = analysis.structures.built.get(name) {
            if layouts.len() == 1
                && layouts[0].1.input.is_some()
                && analysis.structures.facts(&layouts[0].1).abi == Some(NativeAbi::RadixRelation)
            {
                record(name.clone(), layouts[0].0.concat())?;
            }
        } else {
            for ((owner, reader, keys), provider) in &analysis.structures.fallbacks {
                if owner != name
                    || matches!(reader, ReaderRef::Scan)
                    || analysis.structures.facts(provider).abi != Some(NativeAbi::RadixRelation)
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
    let mut input_hosts = BTreeSet::new();
    for entry in &program.structures.structures.entries {
        let Structure::Host { reader: ReaderRef::Scan, keyed, .. } = &entry.structure else { continue };
        if analysis.structures.facts(&keyed.provider).abi != Some(NativeAbi::RadixRelation) { continue; }
        let host = analysis.structures.physical.hosts.get(&symbol_name(&entry.name))
            .ok_or_else(|| error("GPU-RadixRelation: input has no host provenance"))?;
        input_hosts.insert(host.clone());
        let read = records.relation_inputs.get(host)
            .ok_or_else(|| error("GPU-RadixRelation: missing retained input read"))?;
        let input = keyed.provider.input.as_ref()
            .ok_or_else(|| error("GPU-RadixRelation: missing input schema"))?;
        let advice = keyed.provider.radix.as_ref()
            .ok_or_else(|| error("GPU-RadixRelation: missing radix advice"))?;
        if read.provider != keyed.provider
            || input.key_count != input.columns.len()
            || read.key_types != input.columns.iter().map(|column| column.storage.clone()).collect::<Vec<_>>()
            || read.fields != (0..input.columns.len()).collect::<Vec<_>>()
            || read.word != advice.word || read.offset != advice.offset
            || !matches!(&read.word_capture, RadixWordCapture::InputView { host: source } if source == host)
            || read.offset != syn::parse_quote!(u64)
            || read.params.ratio != advice.ratio || read.params.radix_bits != advice.radix_bits
            || read.params.merge_tile_rows != advice.merge_tile
            || read.params.block_threads != crate::BLOCK_THREADS
        {
            return Err(error("GPU-RadixRelation: stale full-key input read facts"));
        }
    }
    if records.relation_inputs.keys().cloned().collect::<BTreeSet<_>>() != input_hosts {
        return Err(error("GPU-RadixRelation: extraneous input read facts"));
    }
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
            if !layout.provider.same_token(provider)
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
                ProviderAbi::RadixRelation => {
                    analysis.structures.facts(provider).abi == Some(NativeAbi::RadixRelation)
                        && provider.input.is_some()
                        && catalog
                            .providers
                            .iter()
                            .any(|token| token.provider.same_token(provider))
                        && layout.directory.is_none()
                }
                ProviderAbi::Radix => {
                    analysis.structures.facts(provider).abi == Some(NativeAbi::Radix)
                        && catalog
                            .providers
                            .iter()
                            .any(|token| token.provider.same_token(provider))
                        && layout.directory.is_none()
                        && layout.radix.as_ref().is_some_and(|radix| {
                            check_radix(
                                program,
                                analysis,
                                &records.carriers,
                                name,
                                provider,
                                layout,
                                radix,
                            )
                        })
                }
                ProviderAbi::Host => {
                    layout.directory.is_none()
                        && !matches!(
                            analysis.structures.facts(provider).abi,
                            Some(NativeAbi::Range | NativeAbi::RadixRelation | NativeAbi::Radix)
                        )
                }
            };
            if (layout.abi == ProviderAbi::Radix) != layout.radix.is_some() {
                return Err(error(
                    "GPU-RadixProvider: radix facts recorded off a radix layout",
                ));
            }
            if !valid {
                return Err(error("GPU-ProviderLayout: provider catalog ABI mismatch"));
            }
        }
    }
    let definitions: BTreeMap<_, _> = program.structures.physical.program.definitions.iter()
        .map(|definition| (symbol_name(&definition.name), definition.clone())).collect();
    let mut required_native = BTreeSet::new();
    let mut check_lookup = |name: &str, id: String, keys: Vec<String>| -> syn::Result<()> {
        let Some(origin) = crate::rows::origin(name, &definitions)? else { return Ok(()) };
        if !records.relation_inputs.contains_key(&origin.host) || origin.selection.is_some() {
            return Ok(());
        }
        let lookup = records.native.get(&id)
            .ok_or_else(|| error("GPU-RadixRelation: missing native prefix read"))?;
        let physical = &analysis.structures.host_layouts[&origin.host];
        if lookup.keys != keys || lookup.host != origin.host
            || lookup.source.len() != physical.len()
            || lookup.source.iter().zip(physical)
                .any(|(column, original)| origin.columns.get(column) != Some(original))
        {
            return Err(error("GPU-RadixRelation: prefix read differs from recorded source provenance"));
        }
        required_native.insert(id);
        Ok(())
    };
    for (name, layouts) in &analysis.structures.built {
        if layouts.len() == 1
            && analysis.structures.facts(&layouts[0].1).abi == Some(NativeAbi::RadixRelation)
        {
            check_lookup(name, name.clone(), layouts[0].0.concat())?;
        }
    }
    for ((name, reader, keys), provider) in &analysis.structures.fallbacks {
        if analysis.structures.built.contains_key(name) || matches!(reader, ReaderRef::Scan)
            || analysis.structures.facts(provider).abi != Some(NativeAbi::RadixRelation) { continue; }
        let mut sorted = keys.clone();
        sorted.sort();
        check_lookup(name, format!("{name}__{}", sorted.join("__")), keys.clone())?;
    }
    if records.native.keys().cloned().collect::<BTreeSet<_>>() != required_native {
        return Err(error("GPU-RadixRelation: extraneous native prefix read"));
    }
    for lookup in records.native.values() {
        let source = records.relation_inputs.get(&lookup.host)
            .ok_or_else(|| error("GPU-RadixRelation: lookup has no retained input"))?;
        if lookup.radix.provider != source.provider || lookup.radix.key_types != source.key_types
            || lookup.radix.word != source.word || lookup.radix.offset != source.offset
            || lookup.radix.word_capture != source.word_capture
            || lookup.radix.params != source.params
            || lookup.radix.fields.len() != lookup.payload.len()
            || lookup.source.len() != source.key_types.len()
            || lookup.radix.fields.iter().zip(&lookup.payload)
                .any(|(field, column)| lookup.source.get(*field) != Some(column))
        {
            return Err(error("GPU-RadixRelation: stale key-field read mapping"));
        }
        if lookup.source.len() != lookup.payload.len()
            || lookup.keys.len() > lookup.payload.len()
            || lookup.payload[..lookup.keys.len()]
                .iter()
                .any(|key| !lookup.keys.contains(key))
        {
            return Err(error("GPU-NativeAbi: invalid lookup schema"));
        }
        if let Some(materialization) = &lookup.materialization {
            let key_types = &source.key_types;
            let word = &source.word;
            if materialization.active_bits != syn::parse_quote!(
                0u32 #(+ <#key_types as ::gpu_datalog_runtime::Scalar>::COLUMN.bits())*
            ) && materialization.active_bits != syn::parse_quote!(
                <#word as ::gpu_datalog_runtime::Word>::BITS
            ) {
                return Err(error("GPU-NativeAbi: active radix window must cover the complete codec widths"));
            }
            let order = &materialization.permutation;
            let RadixParams {
                ratio, radix_bits, merge_tile_rows, block_threads,
            } = materialization.params;
            if !materialization
                .provider
                .same_token(&crate::providers::declaration("radix_relation").instance())
                || ratio < 2
                || radix_bits > 32
                || merge_tile_rows == 0
                || block_threads != crate::BLOCK_THREADS
                || materialization.permutation_threads != crate::BLOCK_THREADS
                || materialization.provider.radix != source.provider.radix
                || materialization.params != source.params
                || materialization.permutation != lookup.radix.fields
            {
                return Err(error(
                    "GPU-NativeAbi: materialization names an undeclared provider or unsupported run parameters",
                ));
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
            let input = source.provider.input.as_ref().expect("checked source schema");
            let mut schema = input.clone();
            schema.columns = order.iter().map(|&index| input.columns[index].clone()).collect();
            let types = order.iter().map(|&index| &source.key_types[index]);
            let word = &source.word;
            let offset = &source.offset;
            let expected: syn::Type = syn::parse_quote!(
                ::gpu_datalog_runtime::RadixLayout<(#(#types,)*), (), #word, #offset, ()>);
            if materialization.provider.input.as_ref() != Some(&schema)
                || materialization.provider.ty != expected
            {
                return Err(error("GPU-RadixRelation: permutation schema differs from its full-key provider"));
            }
        } else if lookup.source != lookup.payload {
            return Err(error(
                "GPU-NativeAbi: borrowed lookup changed provider or order",
            ));
        }
    }
    // GPU-NativeAbi downloads: a recorded download is the declared host scan
    // provider over a host scan of a device-resident input (runs or columns
    // ABI); every such scan has one, since host Rust cannot iterate device
    // memory.
    let host_scan = cq_lang_structures::provider::base::declaration("host_scan").instance();
    let device_scan = |name: &str| {
        program.structures.structures.entries.iter().any(|entry| {
            symbol_name(&entry.name) == name
                && matches!(&entry.structure, Structure::Host {
                    reader: ReaderRef::Scan,
                    keyed,
                    ..
                } if matches!(
                    analysis.structures.facts(&keyed.provider).abi,
                    Some(NativeAbi::RadixRelation | NativeAbi::Columns)
                ))
        })
    };
    for (name, provider) in &records.downloads {
        if !provider.same_token(&host_scan) || !device_scan(name) {
            return Err(error("GPU-NativeAbi: invalid download adapter"));
        }
    }
    if program.structures.structures.entries.iter().any(|entry| {
        let name = symbol_name(&entry.name);
        device_scan(&name) && !records.downloads.contains_key(&name)
    }) {
        return Err(error(
            "GPU-NativeAbi: download adapter records are incomplete",
        ));
    }
    // GPU-HostMaterialization: each device-only collection/index has a
    // separately recorded host adapter. Its collection protocol is determined
    // by the Structures entry, never by inspecting its Rust type spelling.
    let mut required = BTreeSet::new();
    for (index, entry) in program.structures.structures.entries.iter().enumerate() {
        let (provider, collection) = match &entry.structure {
            Structure::Rows { provider, .. } => (provider, true),
            Structure::Built { keyed, .. } => (&keyed.provider, false),
            _ => continue,
        };
        if !matches!(
            analysis.structures.facts(provider).abi,
            Some(NativeAbi::Range | NativeAbi::RadixRelation | NativeAbi::Radix)
        ) {
            continue;
        }
        required.insert(index);
        let adapter = records.host_materializations.get(&index).ok_or_else(|| {
            error("GPU-HostMaterialization: missing host collection adapter")
        })?;
        let declaration = cq_lang_structures::provider::base::declaration(
            if collection { "vec" } else { "sorted_run" },
        );
        if *adapter != declaration.instance() {
            return Err(error("GPU-HostMaterialization: invalid host collection adapter"));
        }
    }
    if records.host_materializations.keys().any(|index| !required.contains(index)) {
        return Err(error("GPU-HostMaterialization: adapter names no device collection"));
    }
    let host = host_structures(program, records)?;
    cq_lang_structures::check_rewrite(&program.structures, &host)?;
    cq_lang_structures::contract::check(&host)?;
    Ok(())
}

/// Apply recorded adapters without selecting a provider or inspecting Rust.
pub(crate) fn host_structures(
    program: &Program,
    records: &Bindings,
) -> syn::Result<cq_lang_structures::Program> {
    let mut host = program.structures.clone();
    for (index, entry) in host.structures.entries.iter_mut().enumerate() {
        if let Some(provider) = records.host_materializations.get(&index) {
            match &mut entry.structure {
                Structure::Rows { provider: slot, .. } => *slot = provider.clone(),
                Structure::Built { keyed, .. } => keyed.provider = provider.clone(),
                _ => unreachable!("checked host materialization entry"),
            }
        }
        if let Some(provider) = records.downloads.get(&symbol_name(&entry.name))
            && let Structure::Host { keyed, .. } = &mut entry.structure
        {
            keyed.provider = provider.clone();
            keyed.supports = None;
        }
    }
    let tokens: BTreeSet<_> = records.downloads.values()
        .chain(records.host_materializations.values())
        .map(|provider| provider.token.to_string())
        .collect();
    let declarations: Vec<_> = tokens.iter()
        .map(|token| cq_lang_structures::provider::base::declaration(token))
        .collect();
    host.structures.declare(&declarations)?;
    Ok(host)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::{quote, ToTokens};
    use syn::visit_mut::VisitMut;

    fn triangle(smallest: bool) -> Program {
        let choice = if smallest { quote!(smallest) } else { quote!(first) };
        syn::parse_quote! {
            physical {
                relation E(src:i64,dst:i64) = (edges);
                l = pipe rename E@0 {src -> a,dst -> b};
                r = let rename E@1 {src -> b,dst -> c};
                s = let rename E@2 {src -> a,dst -> c};
                candidate = pipe probe #choice [b] l,r;
                checked = let canon probe first [a,c] candidate,s;
                return checked as answer(a,b,c);
            }
            structures {
                providers {
                    device_columns = ::cq_gpu_runtime::DeviceColumns<_> { reads [], build typed, abi columns, bounds resident_columns };
                    range_provider = ::cq_gpu_sequence_runtime::RangeProvider { reads [exact, range, membership], build inferred, abi range, contiguous_range };
                    radix_layout = ::gpu_datalog_runtime::RadixLayout<_, _, _, _, _> { reads [exact, range, membership, candidates], build typed, abi radix };
                    radix_set = ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()> { reads [distinct, resident], build inferred };
                }
                l host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
                r host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
                s host scan = @device_columns input { keys 0; columns [i64 => i64, i64 => i64] } as ::cq_gpu_runtime::DeviceColumns<'static,(i64,i64)> [];
                r = @radix_layout radix { word u128; offset u32; ratio 4; radix_bits 16; merge_tile 1024 } as ::gpu_datalog_runtime::RadixLayout<(i64,i64),(i64,i64),u128,u32,::gpu_datalog_runtime::SourceColumns<(i64,i64)>> [b][c];
                s = @radix_layout radix { word u128; offset u32; ratio 4; radix_bits 16; merge_tile 1024 } as ::gpu_datalog_runtime::RadixLayout<(i64,i64),(i64,i64),u128,u32,::gpu_datalog_runtime::SourceColumns<(i64,i64)>> [a][c];
                r rows = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider;
                s rows = @range_provider as ::cq_gpu_sequence_runtime::RangeProvider;
                checked rows = @radix_set as ::gpu_datalog_runtime::RadixLayout<::gpu_datalog_runtime::radix_run::EncodedRow, (), ::gpu_datalog_runtime::radix_run::PackedRow, u64, ()>;
                checked canon = rows;
            }
            sequence {
                Edges = store rows l;
                ByB = store rows r;
                ByA = store rows s;
                result = store canon flatten map checked flatten lookup s [a,c]
                    flatten lookup r [b] tile 1024 items 4 Edges;
                output count;
            }
            graded gpu { Edges at host; ByB at host; ByA at host; result at device; }
        }
    }

    #[test]
    fn source_recipes_reject_missing_fields_and_stale_records_independently() {
        let program = triangle(true);
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        check(&program, &analysis, &analysis.gpu_bindings).unwrap();
        let mutations: &[fn(&mut RadixRealization)] = &[
            |record| record.source_build.source = "s".into(),
            |record| { record.source_build.fields.pop(); },
            |record| record.source_build.fields.swap(0, 1),
            |record| record.source_build.row_map.source = "s".into(),
            |record| {
                let KeyRecipe::DirectSourceOrder { fields } = &mut record.key_recipe;
                fields.swap(0, 1);
            },
            |record| {
                let KeyRecipe::DirectSourceOrder { fields } = &mut record.key_recipe;
                fields.pop();
            },
            |record| {
                let PayloadRepresentation::RetainedSources { fields, .. } = &mut record.payload_representation;
                fields.swap(0, 1);
            },
            |record| {
                let PayloadRepresentation::RetainedSources { fields, .. } = &mut record.payload_representation;
                fields.clear();
            },
            |record| {
                let PayloadRepresentation::RetainedSources { row_map, .. } = &mut record.payload_representation;
                row_map.source = "s".into();
            },
        ];
        for mutate in mutations {
            let mut stale = analysis.gpu_bindings.clone();
            mutate(stale.providers.get_mut("r").unwrap()[0].radix.as_mut().unwrap());
            assert!(check(&program, &analysis, &stale).is_err());
        }
    }

    #[test]
    fn retained_recipes_require_owned_source_provenance_and_selection() {
        use cq_lang_sequence::gpu::realization::{SourceAccess, SourceOrigin};
        let program = triangle(true);
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let valid = &analysis.gpu_bindings.realization;
        crate::realize::check(&program, &analysis, valid).unwrap();
        let mut missing = valid.clone();
        missing.sources.remove("r");
        assert!(crate::realize::check(&program, &analysis, &missing).is_err());
        let mut scan = valid.clone();
        scan.sources.get_mut("r").unwrap().access = SourceAccess::Scan;
        assert!(crate::realize::check(&program, &analysis, &scan).is_err());
        let mut selection = valid.clone();
        selection.sources.get_mut("r").unwrap().access = SourceAccess::Provider {
            selection: Some(cq_lang_sequence::gpu::realization::Interval {
                column: "absent".into(), bounds: Vec::new(),
            }),
        };
        assert!(crate::realize::check(&program, &analysis, &selection).is_err());
        let mut coverage = valid.clone();
        coverage.sources.get_mut("r").unwrap().columns.remove("c");
        assert!(crate::realize::check(&program, &analysis, &coverage).is_err());
        let mut provenance = valid.clone();
        provenance.sources.get_mut("r").unwrap().origin = SourceOrigin::Resident { host: "missing".into() };
        assert!(crate::realize::check(&program, &analysis, &provenance).is_err());
        let mut identity = valid.clone();
        identity.sources.get_mut("r").unwrap().columns.insert("c".into(), "src".into());
        assert!(crate::realize::check(&program, &analysis, &identity).is_err());
    }

    #[derive(Default)]
    struct Builds {
        radix: usize,
        owned_columns: usize,
    }
    impl VisitMut for Builds {
        fn visit_expr_call_mut(&mut self, call: &mut syn::ExprCall) {
            if let syn::Expr::Path(function) = call.func.as_ref() {
                let segments = &function.path.segments;
                let last = segments.last().unwrap();
                if last.ident == "radix_layout" {
                    let syn::PathArguments::AngleBracketed(arguments) = &last.arguments else { panic!("missing scalar schema") };
                    assert_eq!(arguments.args.len(), 4, "no native item-column generic");
                    self.radix += 1;
                }
                if last.ident == "new" && segments.iter().any(|segment| segment.ident == "ColumnDescriptor") {
                    assert!(matches!(&call.args[0], syn::Expr::MethodCall(method) if method.method == "clone"));
                    self.owned_columns += 1;
                }
            }
            syn::visit_mut::visit_expr_call_mut(self, call);
        }
    }

    #[test]
    fn source_builds_and_all_radix_read_forms_consume_recorded_representation() {
        for smallest in [false, true] {
            let program = triangle(smallest);
            let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
            let rows = crate::rows::Rows::new(&program, &analysis).unwrap();
            let setup = &rows.setup;
            let build = &rows.build;
            let mut block: syn::Block = syn::parse2(quote!({ #setup #build })).unwrap();
            let mut calls = Builds::default();
            calls.visit_block_mut(&mut block);
            assert_eq!(calls.radix, 2);
            assert_eq!(calls.owned_columns, 8);
            for name in ["r", "s"] {
                let record = analysis.gpu_bindings.providers[name][0].radix.as_ref().unwrap();
                let source = &record.source_build;
                assert_eq!(source.source, name);
                assert_eq!(source.ownership, SourceOwnership::RetainedDescriptors);
                assert!(matches!(&record.key_recipe, KeyRecipe::DirectSourceOrder { fields } if fields.len() == 2));
                let kernel = rows.radix_provider(name).unwrap();
                let value: syn::Expr = syn::parse2(kernel.item(1, quote!(run), quote!(occurrence))).unwrap();
                let syn::Expr::Call(call) = value else { panic!("retained read must call explicit source helper") };
                let syn::Expr::Path(function) = call.func.as_ref() else { panic!("source helper must be explicit") };
                let expected: syn::Path = syn::parse_quote!(::gpu_datalog_runtime::radix_run::source_items::device::item);
                assert_eq!(function.path, expected);
                assert_eq!(call.args.len(), 4);
            }
            let reads = &analysis.gpu_bindings.providers["r"][0].radix.as_ref().unwrap().reads;
            assert!(reads.iter().any(|(_, read)| if smallest {
                matches!(read, RadixRead::Candidates { .. })
            } else {
                matches!(read, RadixRead::Prefix)
            }));
            assert!(analysis.gpu_bindings.providers["s"][0].radix.as_ref().unwrap().reads
                .iter().any(|(_, read)| matches!(read, RadixRead::Group)));
            // Translation exercises both the candidate and ordinary payload consumers.
            let _ = crate::emit::translate(&program, &analysis).unwrap().to_token_stream();
        }
    }

    #[test]
    fn group_payload_retains_nonkey_source_fields() {
        let mut program = triangle(false);
        let provider = program.structures.structures.entries.iter_mut()
            .find(|entry| entry.name == "r" && matches!(entry.structure, Structure::Built { .. }))
            .unwrap();
        *provider = syn::parse_quote!(
            r = @radix_layout radix { word u128; offset u32; ratio 4; radix_bits 16; merge_tile 1024 }
                as ::gpu_datalog_runtime::RadixLayout<(i64,),(i64,i64),u128,u32,::gpu_datalog_runtime::SourceColumns<(i64,i64)>> [b];
        );
        program.structures.structures.declare(&crate::providers::declarations()).unwrap();
        let analysis = cq_lang_sequence::check_grading(&program, &crate::machine()).unwrap();
        let layout = &analysis.gpu_bindings.providers["r"][0];
        assert_eq!(layout.keys, ["b"]);
        assert_eq!(layout.payload, ["b", "c"]);
        let radix = layout.radix.as_ref().unwrap();
        assert!(radix.reads.iter().any(|(_, read)| matches!(read, RadixRead::Group)));
        assert!(matches!(&radix.payload_representation,
            PayloadRepresentation::RetainedSources { fields, .. } if fields == &[0, 1]));
        crate::emit::translate(&program, &analysis).unwrap();
    }
}
