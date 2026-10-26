use cq_lang_sequence::{Capability, Catalog, structures};
use cq_syntax::symbol_name;
use quote::quote;
use structures::{Structure, physical::Operator};

/// Replace built provider types with the first catalog token satisfying all of
/// their read obligations. Entry order and catalog order are significant.
///
/// Catalog levels constrain reads, not construction. Structures has no machine
/// grading; Sequence planning checks each selected type at its actual read levels.
/// A keyed reader carrying strings also needs a host read level, even when
/// the provider's own retained layout is entirely numeric.
/// Host scan adapters are preserved. Unsupported keyed host reads are retained
/// through `build_host_reads`; built indexes replace their keyed fallbacks.
/// Unlike `plan`, this operation explicitly replaces recorded provider types.
// Formal rule PL-Select: Retain unsupported host stream reads and select the first token with legal read levels satisfying every entry obligation through checked rewrites.
pub fn select_from_catalog(
    source: &structures::Program,
    catalog: &Catalog,
) -> syn::Result<structures::Program> {
    select(source, catalog, None)
}

/// Select providers while retaining admitted native results when a checked
/// representation rewrite is inapplicable. `admitted` names results whose
/// existing provider obligations the caller's target catalog already supports;
/// the target must still check scheduled read levels after planning.
/// Native row collections retain their collection ABI in this mode.
pub fn select_from_catalog_preserving(
    source: &structures::Program,
    catalog: &Catalog,
    admitted: &std::collections::BTreeSet<String>,
) -> syn::Result<structures::Program> {
    select(source, catalog, Some(admitted))
}

fn select(
    source: &structures::Program,
    catalog: &Catalog,
    admitted: Option<&std::collections::BTreeSet<String>>,
) -> syn::Result<structures::Program> {
    let analysis = structures::contract::check(source)?;
    let needed = structures::required_columns(&source.physical, &analysis.physical);
    let candidates = structures::physical::candidate_prefixes(&source.physical, &analysis.physical);
    let mut target = source.clone();
    for (index, entry) in source.structures.entries.iter().enumerate() {
        let name = symbol_name(&entry.name);
        let mut required = Vec::new();
        let kind = match &entry.structure {
            Structure::Built { keyed, .. } => {
                if let Some(admitted) = admitted
                    && keyed.levels.len() > 1
                {
                    // Range over flat rows is not the Candidates protocol of a
                    // nested directory. Keep its admitted native representation.
                    if admitted.contains(&name) {
                        continue;
                    }
                    return Err(syn::Error::new_spanned(
                        &entry.name,
                        "PL-Select: native candidate directory is not admitted by the target catalog",
                    ));
                }
                let levels = keyed.level_names();
                for (reader, columns, _) in analysis
                    .physical
                    .keyed_reads
                    .get(&name)
                    .into_iter()
                    .flatten()
                {
                    if structures::prefix_depth(&levels, columns).is_none() {
                        continue;
                    }
                    // A separate positional key set discharges membership reads.
                    let operator = source
                        .physical
                        .program
                        .definitions
                        .iter()
                        .find(|definition| symbol_name(&definition.name) == *reader)
                        .map(|definition| &definition.operator);
                    let capability = match operator {
                        Some(Operator::Semi(..) | Operator::Anti(..)) => {
                            if analysis
                                .keysets
                                .contains_key(&(name.clone(), columns.clone()))
                            {
                                continue;
                            }
                            Capability::Membership
                        }
                        Some(Operator::Probe(..)) if keyed.membership.is_some() => {
                            Capability::Membership
                        }
                        Some(Operator::Merge(..)) => Capability::Ordered,
                        _ => Capability::Exact,
                    };
                    if !required.contains(&capability) {
                        required.push(capability);
                    }
                }
                for (_, columns) in analysis
                    .physical
                    .merge_reads
                    .get(&name)
                    .into_iter()
                    .flatten()
                {
                    if structures::prefix_depth(&levels, columns).is_some()
                        && !required.contains(&Capability::Ordered)
                    {
                        required.push(Capability::Ordered);
                    }
                }
                for (result, prefix) in candidates.iter().flat_map(|candidate| &candidate.sides) {
                    if *result == name && structures::prefix_depth(&levels, prefix).is_some() {
                        for capability in [Capability::Range, Capability::Membership] {
                            if !required.contains(&capability) {
                                required.push(capability);
                            }
                        }
                    }
                }
                if keyed.membership.is_some() && !required.contains(&Capability::Membership) {
                    required.push(Capability::Membership);
                }
                "index"
            }
            Structure::Rows { .. } if admitted.is_some() => continue,
            Structure::Rows { .. } => {
                required.push(Capability::Range);
                "rows"
            }
            Structure::Selection { .. }
            | Structure::KeySet { .. }
            | Structure::JoinFilter { .. } => {
                required.push(Capability::Membership);
                "set"
            }
            Structure::Dedup { .. } | Structure::HeadDedup { .. } => "canon",
            Structure::Host { .. }
            | Structure::Read { .. }
            | Structure::Repr { .. }
            | Structure::Stage { .. }
            | Structure::Share { .. } => continue,
        };
        let ty = if kind == "canon" {
            let columns = &analysis.physical.headings[&name];
            // The declared scalar domains are conservative codec bounds. Setup
            // may pack fewer bits, but evidence never narrows this obligation.
            let bits = columns.iter().try_fold(0u32, |sum, column| {
                let ty = &analysis.physical.rows[&name].types[column];
                let width = match ty {
                    syn::Type::Path(path) => {
                        match path.path.get_ident().map(ToString::to_string).as_deref() {
                            Some("bool") => 1,
                            Some("u8" | "i8") => 8,
                            Some("u16" | "i16") => 16,
                            Some("u32" | "i32") => 32,
                            Some("u64" | "i64" | "usize" | "isize") => 64,
                            _ => return None,
                        }
                    }
                    _ => return None,
                };
                sum.checked_add(width)
            });
            catalog
                .canons
                .iter()
                .find(|token| {
                    !token.levels.is_empty()
                        && token
                            .max_packed_bits
                            .is_none_or(|maximum| bits.is_some_and(|bits| bits <= maximum))
                })
                .map(|token| &token.ty)
        } else {
            catalog
                .providers
                .iter()
                .filter(|token| {
                    !token.levels.is_empty()
                        && accepts_elements(token, &analysis, &name)
                        && match &entry.structure {
                            Structure::Built { keyed, .. } => accepts_read_levels(
                                token,
                                source,
                                &analysis,
                                &name,
                                &keyed.level_names(),
                                &needed,
                            ),
                            _ => true,
                        }
                        && required
                            .iter()
                            .all(|capability| token.capabilities.contains(capability))
                })
                .min_by_key(|token| match &entry.structure {
                    Structure::Built { keyed, .. } => token.ty != keyed.ty,
                    Structure::Selection { ty, .. } | Structure::KeySet { ty, .. } => {
                        token.ty != *ty
                    }
                    Structure::JoinFilter { filter, .. } => token.ty != filter.ty,
                    _ => true,
                })
                .map(|token| &token.ty)
        };
        let Some(ty) = ty else {
            if admitted.is_some_and(|names| names.contains(&name)) {
                continue;
            }
            return Err(syn::Error::new(
                entry.name.span(),
                format!(
                    "PL-Select: structure `{name}` entry {index} ({kind}) has unmet catalog read obligation {required:?}"
                ),
            ));
        };
        let index = syn::LitInt::new(&index.to_string(), entry.name.span());
        match cq_rewrite_passes::retype(&target, quote!(entry(#index) => #ty)) {
            Ok(selected) => target = selected,
            Err(_) if admitted.is_some_and(|names| names.contains(&name)) => {}
            Err(error) => {
                return Err(syn::Error::new(
                    error.span(),
                    format!("PL-Select: structure `{name}` ({kind}): {error}"),
                ));
            }
        }
    }
    let target = retain_host_indexes(&target, catalog, admitted)?;
    structures::check_retention_rewrite(source, &target)?;
    Ok(target)
}

fn retain_host_indexes(
    source: &structures::Program,
    catalog: &Catalog,
    admitted: Option<&std::collections::BTreeSet<String>>,
) -> syn::Result<structures::Program> {
    if admitted.is_none() {
        return retain_host_index(source, catalog, None);
    }
    let mut target = source.clone();
    let names: std::collections::BTreeSet<_> = source
        .structures
        .entries
        .iter()
        .filter(
            |entry| matches!(&entry.structure, Structure::Host { reader, .. } if reader != "scan"),
        )
        .map(|entry| symbol_name(&entry.name))
        .collect();
    for name in names {
        match retain_host_index(&target, catalog, Some(&name)) {
            Ok(selected) => target = selected,
            Err(_) if admitted.is_some_and(|names| names.contains(&name)) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(target)
}

fn retain_host_index(
    source: &structures::Program,
    catalog: &Catalog,
    selected: Option<&str>,
) -> syn::Result<structures::Program> {
    let analysis = structures::contract::check(source)?;
    let needed = structures::required_columns(&source.physical, &analysis.physical);
    let mut target = source.clone();
    let mut retained = std::collections::BTreeSet::new();
    let mut indexes = std::collections::BTreeSet::new();
    for entry in &source.structures.entries {
        let Structure::Host { reader, keyed, .. } = &entry.structure else {
            continue;
        };
        let name = symbol_name(&entry.name);
        if selected.is_some_and(|selected| name != selected) {
            continue;
        }
        let reader_name = symbol_name(reader);
        let mut keys = keyed.level_names().concat();
        keys.sort();
        let stream = analysis
            .physical
            .keyed_reads
            .get(&name)
            .is_some_and(|reads| {
                reads.iter().any(|(read, columns, supply)| {
                    *read == reader_name
                        && *columns == keys
                        && (*supply == structures::physical::syntax::SupplyKind::Stream
                            || source
                                .physical
                                .program
                                .definitions
                                .iter()
                                .any(|definition| {
                                    symbol_name(&definition.name) == *read
                                        && matches!(
                                            &definition.operator,
                                            Operator::Semi(..) | Operator::Anti(..)
                                        )
                                }))
                })
            });
        if !stream || analysis.built_on(&name, &keys).is_some() {
            continue;
        }
        let mut required = Vec::new();
        for (read, columns, _) in &analysis.physical.keyed_reads[&name] {
            if *columns != keys {
                continue;
            }
            let capability = match source
                .physical
                .program
                .definitions
                .iter()
                .find(|definition| symbol_name(&definition.name) == *read)
                .map(|definition| &definition.operator)
            {
                Some(Operator::Semi(..) | Operator::Anti(..)) => Capability::Membership,
                Some(Operator::Merge(..)) => Capability::Ordered,
                _ => Capability::Range,
            };
            if !required.contains(&capability) {
                required.push(capability);
            }
            if required.contains(&Capability::Range) && !required.contains(&Capability::Exact) {
                required.push(Capability::Exact);
            }
        }
        let projected = (source.physical.program.definitions.iter().any(|definition|
            symbol_name(&definition.name) == name
                && matches!(&definition.operator, Operator::Rename(rename) if rename.constants.is_none()))
            && !analysis.physical.scanned.contains(&name)
            && needed[&name].len() < analysis.layouts[&name].len()
            && analysis.physical.keyed_reads[&name].iter().all(|(reader, columns, supply)| {
                *columns == keys && matches!(supply, structures::physical::syntax::SupplyKind::Stream | structures::physical::syntax::SupplyKind::Const)
                    && source.physical.program.definitions.iter().any(|definition| {
                        symbol_name(&definition.name) == *reader
                            && matches!(&definition.operator, Operator::Semi(..) | Operator::Anti(..))
                                || symbol_name(&definition.name) == *reader
                                    && matches!(&definition.operator, Operator::Probe(_, read)
                                        if matches!(read.choice, Some(structures::physical::Choice::First(_))))
                    })
            })).then(|| needed[&name].iter().cloned().collect::<Vec<_>>());
        if catalog.providers.iter().any(|token| {
            token.ty == keyed.ty
                && !token.levels.is_empty()
                && accepts_elements(token, &analysis, &name)
                && accepts_read_levels(
                    token,
                    source,
                    &analysis,
                    &name,
                    &keyed.level_names(),
                    &needed,
                )
                && required
                    .iter()
                    .all(|capability| token.capabilities.contains(capability))
        }) {
            continue;
        }
        let token = catalog.providers.iter().find(|token| {
            !token.levels.is_empty()
                && accepts_columns(token, &analysis, &name, projected.as_deref().unwrap_or(&analysis.layouts[&name]))
                && accepts_read_levels(token, source, &analysis, &name, &keyed.level_names(), &needed)
                && required.iter().all(|capability| token.capabilities.contains(capability))
        }).ok_or_else(|| syn::Error::new(
            entry.name.span(),
            format!("PL-Select: structure `{name}` host adapter `{reader_name}` has unmet catalog read obligation {required:?}"),
        ))?;
        let build = &entry.name;
        if retained.insert(name.clone()) {
            target.physical = cq_rewrite_passes::build_host_reads(&target.physical, quote!(#build))
                .map_err(|error| syn::Error::new(error.span(), format!("PL-Select: {error}")))?;
        }
        if indexes.insert((name.clone(), keys.clone())) {
            let ty = &token.ty;
            let columns: Vec<_> = keys
                .iter()
                .map(|key| syn::Ident::new(key, entry.name.span()))
                .collect();
            let projection = projected.map(|columns| {
                let payload: Vec<_> = columns
                    .iter()
                    .map(|column| syn::Ident::new(column, entry.name.span()))
                    .collect();
                quote!(payload [#(#payload),*] constants {})
            });
            target.structures.entries.push(syn::parse_quote!(
                #build = #ty [#(#columns),*] #projection;
            ));
        }
    }
    // Retention replaces keyed host access, not the original column scan.
    target.structures.entries.retain(|entry| {
        !retained.contains(&symbol_name(&entry.name))
            || !matches!(&entry.structure, Structure::Host { reader, .. } if reader != "scan")
    });
    structures::contract::check(&target)?;
    Ok(target)
}

fn accepts_elements(
    token: &cq_lang_sequence::ProviderToken,
    analysis: &structures::Analysis,
    name: &str,
) -> bool {
    accepts_columns(token, analysis, name, &analysis.layouts[name])
}

fn accepts_columns(
    token: &cq_lang_sequence::ProviderToken,
    analysis: &structures::Analysis,
    name: &str,
    columns: &[String],
) -> bool {
    token.elements.as_ref().is_none_or(|accepted| {
        columns.iter().all(|column| {
            let actual = &analysis.physical.rows[name].types[column];
            accepted
                .iter()
                .any(|ty| matches!(ty, syn::Type::Infer(_)) || ty == actual)
        })
    })
}

// PL-Catalog: element compatibility belongs to the retained provider layout;
// read-level compatibility also includes the row carried by each keyed reader.
// A numeric membership index does not consume an unused right-hand payload.
pub(crate) fn accepts_read_levels(
    token: &cq_lang_sequence::ProviderToken,
    source: &structures::Program,
    analysis: &structures::Analysis,
    name: &str,
    levels: &[Vec<String>],
    needed: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
) -> bool {
    let string_row = |name: &str| {
        needed[name].iter().any(|column| {
            matches!(&analysis.physical.rows[name].types[column], syn::Type::Path(path)
                if path.path.segments.last().is_some_and(|segment| segment.ident == "String"))
        })
    };
    let host_read = analysis.physical.keyed_reads.get(name).into_iter().flatten().any(
        |(reader, columns, _)| {
            if structures::prefix_depth(levels, columns).is_none() {
                return false;
            }
            let Some(definition) = source.physical.program.definitions.iter()
                .find(|definition| symbol_name(&definition.name) == *reader)
            else {
                return false;
            };
            string_row(reader) || match &definition.operator {
                Operator::Probe(_, keyed) | Operator::Merge(_, keyed)
                | Operator::Semi(_, keyed) | Operator::Anti(_, keyed) => {
                    string_row(&symbol_name(&keyed.left))
                        || columns.iter().any(|column| {
                            matches!(&analysis.physical.rows[name].types[column], syn::Type::Path(path)
                                if path.path.segments.last().is_some_and(|segment| segment.ident == "String"))
                        })
                }
                _ => false,
            }
        },
    );
    !host_read
        || token
            .levels
            .iter()
            .any(|level| level.len() == 1 && level[0] == "host")
}
