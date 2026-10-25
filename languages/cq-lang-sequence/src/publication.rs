//! Checked publication ownership and native routed-provider representation.
use super::{Analysis, Fact, Meaning, Program, Term, fail};
use crate::Publication;
use cq_syntax::symbol_name;
use std::collections::{BTreeMap, BTreeSet};

/// Preserve only publication provenance before support-equal reheading.
pub(super) struct Input {
    result: Option<String>,
    depth: usize,
    partition: Option<Vec<String>>,
}

impl From<&Fact> for Input {
    fn from(fact: &Fact) -> Self {
        Self {
            result: match &fact.meaning {
                Meaning::Result(result) => Some(result.clone()),
                _ => None,
            },
            depth: fact.depth,
            partition: fact.partition.clone(),
        }
    }
}

pub(super) fn derive(
    program: &Program,
    analysis: &mut Analysis,
    facts: &BTreeMap<String, Input>,
) -> syn::Result<()> {
    let mut inputs = BTreeSet::new();
    for definition in &program.sequence.definitions {
        partition_inputs(&definition.term, &mut inputs);
    }
    // This is a representation contract for the native runtime ABI, not a
    // suffix-name policy: unrelated user types named Sharded remain ordinary
    // Build providers. Validate the constructor before emission sees it.
    for (result, providers) in &analysis.structures.built {
        for (ordinal, (_, ty)) in providers.iter().enumerate() {
            let syn::Type::Path(path) = ty else { continue };
            if path.qself.is_some()
                || path.path.segments.len() != 3
                || path.path.segments[0].ident != "cq_storage"
                || path.path.segments[1].ident != "sharded"
                || path.path.segments[2].ident != "Sharded"
            {
                continue;
            }
            let segment = path.path.segments.last().expect("canonical path");
            let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
                return fail(
                    ty,
                    "BE-Publication",
                    "routed provider requires its inner provider type",
                );
            };
            if !(1..=2).contains(&arguments.args.len())
                || !arguments
                    .args
                    .iter()
                    .all(|arg| matches!(arg, syn::GenericArgument::Type(_)))
                || path
                    .path
                    .segments
                    .iter()
                    .take(2)
                    .any(|segment| !segment.arguments.is_none())
            {
                return fail(
                    ty,
                    "BE-Publication",
                    "invalid native routed-provider representation",
                );
            }
            let syn::GenericArgument::Type(inner) = &arguments.args[0] else {
                unreachable!()
            };
            analysis
                .routed_providers
                .entry(result.clone())
                .or_default()
                .insert(ordinal, inner.clone());
        }
    }
    for definition in &program.sequence.definitions {
        let name = symbol_name(&definition.name);
        let mut tiles = Vec::new();
        host_tiles(&definition.term, &analysis.host_sources, &mut tiles);
        if !tiles.is_empty() {
            analysis.host_tiles.insert(name.clone(), tiles);
        }
        let base = base_source(&definition.term, &analysis.host_sources);
        if let Some(result) = &base {
            analysis.host_sources.insert(name.clone(), result.clone());
        }
        let Term::Store { inner, .. } = &definition.term else {
            continue;
        };
        let fact = &facts[&name];
        let result = fact.result.as_ref();
        let routed = result.is_some_and(|result| analysis.routed_providers.contains_key(result));
        // Formal rule BE-Publication: a resident host borrow and a published
        // alias allocate no rows; every other retained row publication owns them.
        let publication = if fact.depth == 0
            && base.is_some()
            && result.is_some_and(|result| {
                analysis.structures.physical.hosts.contains_key(result)
                    && (routed
                        || analysis
                            .structures
                            .built
                            .get(result)
                            .is_none_or(Vec::is_empty))
                    && !analysis.structures.selections.contains_key(result)
            }) {
            let result = result.expect("checked host provenance").clone();
            Publication::BorrowHost {
                host: analysis.structures.physical.hosts[&result].clone(),
                result,
            }
        } else if fact.depth == 0 && (inputs.contains(&name) || routed) {
            let source = match inner.as_ref() {
                Term::Ref(source) => Some(symbol_name(source)),
                _ => None,
            };
            if let Some(source) = source.filter(|source| {
                matches!(
                    analysis.publications.get(source),
                    Some(
                        Publication::BorrowHost { .. }
                            | Publication::Alias { .. }
                            | Publication::OwnRows { staged: true }
                    )
                )
            }) {
                Publication::Alias { source }
            } else {
                Publication::OwnRows { staged: true }
            }
        } else if fact.depth == 1 && routed && fact.partition.is_some() {
            let keys = fact.partition.clone().expect("checked key cover");
            let result = result.expect("routed result");
            for ordinal in analysis.routed_providers[result].keys() {
                let (levels, ty) = &analysis.structures.built[result][*ordinal];
                if !levels.first().is_some_and(|outer| {
                    keys.iter().all(|column| {
                        outer.contains(column)
                            && !analysis
                                .structures
                                .filtered
                                .get(result)
                                .is_some_and(|filtered| {
                                    filtered
                                        .constants
                                        .iter()
                                        .any(|binding| symbol_name(&binding.column) == *column)
                                })
                    })
                }) {
                    return fail(
                        ty,
                        "BE-Publication",
                        "provider routing must be determined by its stored outer lookup key",
                    );
                }
            }
            Publication::Parts { keys }
        } else {
            if routed && fact.depth != 0 {
                return fail(
                    &definition.term,
                    "BE-Publication",
                    "routed provider publication requires one checked key-disjoint cover",
                );
            }
            Publication::OwnRows { staged: false }
        };
        analysis.publications.insert(name, publication);
    }
    Ok(())
}

fn base_source(term: &Term, sources: &BTreeMap<String, String>) -> Option<String> {
    match term {
        Term::Rows(name) => Some(symbol_name(name)),
        Term::Ref(name) => sources.get(&symbol_name(name)).cloned(),
        Term::Store { inner, .. } => base_source(inner, sources),
        _ => None,
    }
}

fn partition_inputs(term: &Term, inputs: &mut BTreeSet<String>) {
    match term {
        Term::PartitionK { inner, .. } => {
            if let Term::Ref(name) = inner.as_ref() {
                inputs.insert(symbol_name(name));
            }
            partition_inputs(inner, inputs);
        }
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Canon(inner)
        | Term::Tile { inner, .. }
        | Term::Store { inner, .. }
        | Term::Lookup { inner, .. } => partition_inputs(inner, inputs),
        Term::Union(arms) => {
            for arm in arms {
                partition_inputs(arm, inputs);
            }
        }
        Term::Choose(arms) => {
            for (_, arm) in arms {
                partition_inputs(arm, inputs);
            }
        }
        Term::Rows(_) | Term::Ref(_) => {}
    }
}

fn host_tiles(term: &Term, sources: &BTreeMap<String, String>, output: &mut Vec<Option<String>>) {
    if let Term::Tile { inner, .. } = term {
        output.push(base_source(inner, sources));
    }
    match term {
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Canon(inner)
        | Term::Tile { inner, .. }
        | Term::Store { inner, .. }
        | Term::PartitionK { inner, .. }
        | Term::Lookup { inner, .. } => host_tiles(inner, sources, output),
        Term::Union(arms) => {
            for arm in arms {
                host_tiles(arm, sources, output);
            }
        }
        Term::Choose(arms) => {
            for (_, arm) in arms {
                host_tiles(arm, sources, output);
            }
        }
        Term::Rows(_) | Term::Ref(_) => {}
    }
}
