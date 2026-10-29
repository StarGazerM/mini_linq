use cq_lang_sequence::{Capability, Catalog, structures};
use cq_syntax::symbol_name;
use quote::quote;
use structures::{Structure, physical::Operator};

/// Select the first eligible provider in the stable representation-preserving
/// catalog order: recorded-type matches first, then the remaining tokens.
///
/// Catalog levels constrain reads, not construction. Structures has no machine
/// grading; Sequence planning checks each selected type at its actual read levels.
/// Keyed readers whose carried columns lack a matching catalog element capability
/// need a host read level, independently of the provider's retained layout.
/// Host scan adapters are preserved. Unsupported keyed host reads are retained
/// through `build_host_reads`; built indexes preserve their membership certificates.
/// Unlike `plan`, this operation explicitly replaces recorded provider types.
/// Formal rule PL-Select: Stable-partition catalog order by recorded representation, then select its first eligible token.
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
/// Keyed host adapters are retained even when their storage type is already
/// admitted: a lazy host fallback is not the published storage value itself.
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
                    // A merge consumes ordered payload rows: an ordered key set
                    // is not ordered-admissible for a Merge build.
                    let capabilities: &[Capability] = match operator {
                        Some(Operator::Semi(..) | Operator::Anti(..)) => {
                            if analysis
                                .keysets
                                .contains_key(&(name.clone(), columns.clone()))
                            {
                                continue;
                            }
                            &[Capability::Membership]
                        }
                        Some(Operator::Probe(..)) if keyed.membership.is_some() => {
                            &[Capability::Membership]
                        }
                        Some(Operator::Merge(..)) => &[Capability::Ordered, Capability::Exact],
                        _ => &[Capability::Exact],
                    };
                    for capability in capabilities {
                        if !required.contains(capability) {
                            required.push(capability.clone());
                        }
                    }
                }
                for (_, columns) in analysis
                    .physical
                    .merge_reads
                    .get(&name)
                    .into_iter()
                    .flatten()
                {
                    if structures::prefix_depth(&levels, columns).is_some() {
                        for capability in [Capability::Ordered, Capability::Exact] {
                            if !required.contains(&capability) {
                                required.push(capability);
                            }
                        }
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
            | Structure::Group { .. }
            | Structure::Read { .. }
            | Structure::Repr { .. }
            | Structure::Stage { .. }
            | Structure::Share { .. } => continue,
        };
        let ty = if kind == "canon" {
            // Packed bounds are runtime codec facts, never inferred from Rust types.
            catalog
                .canons
                .iter()
                .find(|token| !token.levels.is_empty() && token.max_packed_bits.is_none())
                .map(|token| &token.ty)
        } else {
            let recorded = |token: &&cq_lang_sequence::ProviderToken| match &entry.structure {
                Structure::Built { keyed, .. } => {
                    crate::catalog::matches_type(&token.ty, &keyed.ty)
                }
                Structure::Selection { ty, .. } | Structure::KeySet { ty, .. } => token.ty == *ty,
                Structure::JoinFilter { filter, .. } => token.ty == filter.ty,
                _ => false,
            };
            catalog
                .providers
                .iter()
                .filter(recorded)
                .chain(catalog.providers.iter().filter(|token| !recorded(token)))
                .find(|token| {
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
                .map(|token| match &entry.structure {
                    // Catalog patterns admit concrete native providers; they
                    // are not replacement Rust types with inferred arguments.
                    Structure::Built { keyed, .. }
                        if crate::catalog::matches_type(&token.ty, &keyed.ty) =>
                    {
                        &keyed.ty
                    }
                    _ => &token.ty,
                })
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
        match cq_structure_selection::retype(&target, quote!(entry(#index) => #ty)) {
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
            |entry| matches!(&entry.structure, Structure::Host { reader, .. } if !matches!(reader, structures::ReaderRef::Scan)),
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
        let reader_name = reader.result().map(symbol_name);
        let mut keys = keyed.level_names().concat();
        keys.sort();
        let stream = analysis
            .physical
            .keyed_reads
            .get(&name)
            .is_some_and(|reads| {
                reads.iter().any(|(read, columns, supply)| {
                    Some(read) == reader_name.as_ref()
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
        let membership = analysis.membership.contains(&(name.clone(), keys.clone()));
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
                Some(Operator::Probe(..)) if membership => Capability::Membership,
                Some(Operator::Merge(..)) => Capability::Ordered,
                _ => Capability::Range,
            };
            if !required.contains(&capability) {
                required.push(capability);
            }
            // Range and merge reads consume payload rows, not only keys.
            if (required.contains(&Capability::Range) || required.contains(&Capability::Ordered))
                && !required.contains(&Capability::Exact)
            {
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
        let native = catalog.providers.iter().find(|token| {
            crate::catalog::matches_type(&token.ty, &keyed.ty)
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
        });
        if selected.is_none() && native.is_some() {
            continue;
        }
        let token = native.or_else(|| catalog.providers.iter().find(|token| {
            !token.levels.is_empty()
                && accepts_columns(token, &analysis, &name, projected.as_deref().unwrap_or(&analysis.layouts[&name]))
                && accepts_read_levels(token, source, &analysis, &name, &keyed.level_names(), &needed)
                && required.iter().all(|capability| token.capabilities.contains(capability))
        })).ok_or_else(|| syn::Error::new(
            entry.name.span(),
            format!("PL-Select: structure `{name}` host adapter `{reader_name:?}` has unmet catalog read obligation {required:?}"),
        ))?;
        let build = &entry.name;
        if retained.insert(name.clone()) {
            target.physical =
                cq_structure_selection::build_host_reads(&target.physical, quote!(#build))
                    .map_err(|error| {
                        syn::Error::new(error.span(), format!("PL-Select: {error}"))
                    })?;
        }
        if indexes.insert((name.clone(), keys.clone())) {
            // A retained native representation keeps its original row ABI.
            let projected = if native.is_some() { None } else { projected };
            let ty = if native.is_some() {
                &keyed.ty
            } else {
                &token.ty
            };
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
            // Retention changes the provider's ownership, not its read protocol.
            let membership = membership.then(|| quote!(membership));
            target.structures.entries.push(syn::parse_quote!(
                #build = #ty [#(#columns),*] #membership #projection;
            ));
        }
    }
    // Retention replaces keyed host access, not the original column scan.
    target.structures.entries.retain(|entry| {
        !retained.contains(&symbol_name(&entry.name))
            || !matches!(&entry.structure, Structure::Host { reader, .. } if !matches!(reader, structures::ReaderRef::Scan))
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
// A membership index consumes its keys, not an unused right-hand payload.
pub(crate) fn accepts_read_levels(
    token: &cq_lang_sequence::ProviderToken,
    source: &structures::Program,
    analysis: &structures::Analysis,
    name: &str,
    levels: &[Vec<String>],
    needed: &std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
) -> bool {
    let accepted_row = |name: &str| {
        accepts_columns(
            token,
            analysis,
            name,
            &needed[name].iter().cloned().collect::<Vec<_>>(),
        )
    };
    let host_read = analysis
        .physical
        .keyed_reads
        .get(name)
        .into_iter()
        .flatten()
        .any(|(reader, columns, _)| {
            if structures::prefix_depth(levels, columns).is_none() {
                return false;
            }
            let Some(definition) = source
                .physical
                .program
                .definitions
                .iter()
                .find(|definition| symbol_name(&definition.name) == *reader)
            else {
                return false;
            };
            !accepted_row(reader)
                || match &definition.operator {
                    Operator::Probe(_, keyed)
                    | Operator::Merge(_, keyed)
                    | Operator::Semi(_, keyed)
                    | Operator::Anti(_, keyed) => {
                        !accepted_row(&symbol_name(&keyed.left))
                            || !accepts_columns(token, analysis, name, columns)
                    }
                    _ => false,
                }
        });
    !host_read
        || token
            .levels
            .iter()
            .any(|level| level.len() == 1 && level[0] == "host")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_patterns_preserve_concrete_native_built_and_host_providers() {
        for (mode, entry) in [
            (
                quote!(pipe),
                quote!(r host result(answer) = Native<(i32,), (i32,)> [a];),
            ),
            (quote!(let), quote!(r = Native<(i32,), (i32,)> [a];)),
        ] {
            let source: structures::Program = syn::parse_quote! {
                physical {
                    relation L(a:i32); relation R(a:i32,b:i32);
                    consumer canonical;
                    l = pipe rename L@0 {a -> a};
                    r = #mode rename R@1 {a -> a,b -> b};
                    answer = pipe probe first [a] l,r;
                    return answer as answer(a,b);
                }
                structures {
                    l host scan = HostColumns [];
                    r host scan = Native<(i32,), (i32,)> [];
                    #entry
                }
            };
            let token = |ty| cq_lang_sequence::ProviderToken {
                ty,
                capabilities: vec![Capability::Exact, Capability::Range, Capability::Membership],
                levels: vec![vec!["host".into(), "device".into()]],
                elements: None,
            };
            let catalog = Catalog {
                providers: vec![
                    token(syn::parse_quote!(Index)),
                    token(syn::parse_quote!(Native<_, _>)),
                ],
                canons: Vec::new(),
                codecs: Vec::new(),
            };
            let selected = select_from_catalog(&source, &catalog).unwrap();
            structures::check_rewrite(&source, &selected).unwrap();
            assert_eq!(selected, source);
        }
    }
}
