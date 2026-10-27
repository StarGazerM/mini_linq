//! Checked publication ownership and native routed-provider representation.
use super::{Analysis, Fact, Guard, Len, Meaning, Program, Term, fail};
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
    defer_single_consumer_rows(program, analysis, facts);
    Ok(())
}

/// Fuse only a total, single-consumer raw Store with its recorded worker
/// cover. Nested count guards may stage it, but every leaf reuses that input.
fn defer_single_consumer_rows(
    program: &Program,
    analysis: &mut Analysis,
    facts: &BTreeMap<String, Input>,
) {
    if analysis.routed_providers.is_empty() {
        return;
    }
    let definitions: BTreeMap<_, _> = program
        .sequence
        .definitions
        .iter()
        .enumerate()
        .map(|(index, definition)| (symbol_name(&definition.name), (index, &definition.term)))
        .collect();
    let physical: BTreeMap<_, _> = program
        .structures
        .physical
        .program
        .definitions
        .iter()
        .map(|definition| (symbol_name(&definition.name), &definition.operator))
        .collect();
    let mut deferred = Vec::new();
    for (consumer, &(consumer_index, term)) in &definitions {
        if !matches!(
            analysis.publications.get(consumer),
            Some(Publication::Parts { .. })
        ) {
            continue;
        }
        let Term::Store { inner, .. } = term else {
            continue;
        };
        let Term::Choose(arms) = inner.as_ref() else {
            continue;
        };
        let Some((Guard::WorkersLt(2), worker_arm)) = arms.first() else {
            continue;
        };
        let Some((input, true)) = cover_input(worker_arm) else {
            continue;
        };
        if !matches!(
            analysis.publications.get(&input),
            Some(Publication::OwnRows { staged: true })
        ) || !arms
            .iter()
            .all(|(_, arm)| cover_input(arm).is_some_and(|(source, _)| source == input))
        {
            continue;
        }
        let Some(result) = facts[consumer].result.as_ref() else {
            continue;
        };
        // A support-equal reheading is not a proof that the raw carrier, or the
        // provider constructed from it, is interchangeable with another one.
        if facts[&input].result.as_ref() != Some(result)
            || analysis.results.get(&input) != Some(result)
            || analysis.results.get(consumer) != Some(result)
            || analysis
                .structures
                .built
                .get(result)
                .is_none_or(|built| built.len() != 1)
            || analysis
                .routed_providers
                .get(result)
                .is_none_or(|built| built.len() != 1)
            || analysis.structures.retained.contains_key(result)
            || analysis
                .output
                .as_ref()
                .is_some_and(|(name, _)| name == consumer)
        {
            continue;
        }
        // References include aliases and written count guards. Rows(result)
        // is a replay too, even when it bypasses the Sequence publication name.
        // Other Stores of this physical result would build the provider again.
        if definitions.iter().any(|(name, &(_, term))| {
            (name != consumer && demands(term, &input, result))
                || demands(term, consumer, result)
                || (name != &input
                    && name != consumer
                    && matches!(term, Term::Store { .. })
                    && facts.get(name).is_some_and(|fact| {
                        fact.result.as_ref() == Some(result)
                            || analysis.results.get(name) == Some(result)
                    }))
        }) {
            continue;
        }
        let (input_index, Term::Store { inner, .. }) = definitions[&input] else {
            continue;
        };
        let mut dependencies = BTreeSet::new();
        if !producer_dependencies(
            inner,
            &definitions,
            &analysis.publications,
            &mut dependencies,
        ) {
            continue;
        }
        let mut pending: Vec<_> = dependencies.iter().cloned().collect();
        let mut visited = BTreeSet::new();
        let mut safe = true;
        while let Some(name) = pending.pop() {
            if !visited.insert(name.clone()) {
                continue;
            }
            // Scan filter bindings are shadowed when covering providers are
            // built. Moving their source expression can change the binding it
            // captures, so retain the original Store for any such dependency.
            if analysis.structures.join_filters.contains_key(&name) {
                safe = false;
                break;
            }
            if let Some(operator) = physical.get(&name) {
                if analysis.structures.physical.opaque(operator) {
                    safe = false;
                    break;
                }
                pending.extend(operator.operands().into_iter().map(symbol_name));
            } else if !analysis.structures.host_layouts.contains_key(&name) {
                safe = false;
                break;
            }
        }
        // Do not capture a different published source/provider binding after
        // crossing another Store. Pure views do not create such bindings.
        if safe
            && !program.sequence.definitions[input_index + 1..consumer_index]
                .iter()
                .any(|definition| {
                    if !matches!(definition.term, Term::Store { .. }) {
                        return false;
                    }
                    let name = symbol_name(&definition.name);
                    facts.get(&name).is_some_and(|fact| {
                        fact.result
                            .as_ref()
                            .is_some_and(|result| visited.contains(result))
                            || analysis
                                .results
                                .get(&name)
                                .is_some_and(|result| visited.contains(result))
                    })
                })
        {
            deferred.push((input, consumer.clone()));
        }
    }
    for (input, consumer) in deferred {
        analysis
            .publications
            .insert(input, Publication::DeferredRows { consumer });
    }
}

/// Checked choices already prove guard totality and compatible key covers.
/// Restrict fusion further to direct partitions of one publication, with a
/// singleton somewhere in the leading worker arm. Host bounds can select that
/// singleton without evaluating the producer; input counts stage it once for
/// either the singleton or routed leaf.
fn cover_input(term: &Term) -> Option<(String, bool)> {
    match term {
        Term::PartitionK { parts, inner, .. } => match inner.as_ref() {
            Term::Ref(input) => Some((symbol_name(input), *parts == 1)),
            _ => None,
        },
        Term::Choose(arms) => {
            let mut arms = arms.iter();
            let (input, mut singleton) = cover_input(&arms.next()?.1)?;
            for (_, arm) in arms {
                let (source, has_singleton) = cover_input(arm)?;
                if source != input {
                    return None;
                }
                singleton |= has_singleton;
            }
            Some((input, singleton))
        }
        _ => None,
    }
}

fn length_demands(len: &Len, name: &str, result: &str) -> bool {
    match len {
        Len::Input(input) | Len::Scan(input) => input == name || input == result,
        Len::Add(left, right) => {
            length_demands(left, name, result) || length_demands(right, name, result)
        }
        Len::CeilDiv(inner, _) => length_demands(inner, name, result),
        Len::Lit(_) | Len::Unknown(_) => false,
    }
}

fn demands(term: &Term, name: &str, result: &str) -> bool {
    match term {
        Term::Ref(input) => symbol_name(input) == name,
        Term::Rows(input) => symbol_name(input) == result,
        Term::Choose(arms) => arms.iter().any(|(guard, arm)| {
            matches!(guard, Guard::Lt(len, _) | Guard::Ge(len, _)
                if length_demands(len, name, result))
                || demands(arm, name, result)
        }),
        Term::Union(arms) => arms.iter().any(|arm| demands(arm, name, result)),
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Canon(inner)
        | Term::Tile { inner, .. }
        | Term::Store { inner, .. }
        | Term::PartitionK { inner, .. }
        | Term::Lookup { inner, .. } => demands(inner, name, result),
    }
}

fn producer_dependencies(
    term: &Term,
    definitions: &BTreeMap<String, (usize, &Term)>,
    publications: &BTreeMap<String, Publication>,
    dependencies: &mut BTreeSet<String>,
) -> bool {
    match term {
        Term::Rows(result) => {
            dependencies.insert(symbol_name(result));
            true
        }
        Term::Ref(name) => {
            let name = symbol_name(name);
            if let Some(Publication::BorrowHost { result, .. }) = publications.get(&name) {
                dependencies.insert(result.clone());
                true
            } else {
                definitions.get(&name).is_some_and(|(_, term)| {
                    producer_dependencies(term, definitions, publications, dependencies)
                })
            }
        }
        // Prior owned materializations stay pinned. A checked resident host
        // borrow above is replayable without moving or rebuilding a Store.
        Term::Store { .. } => false,
        Term::Map(result, inner)
        | Term::Compact(result, inner)
        | Term::Group(result, inner)
        | Term::Lookup {
            provider: result,
            inner,
            ..
        } => {
            dependencies.insert(symbol_name(result));
            producer_dependencies(inner, definitions, publications, dependencies)
        }
        Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Canon(inner)
        | Term::Tile { inner, .. }
        | Term::PartitionK { inner, .. } => {
            producer_dependencies(inner, definitions, publications, dependencies)
        }
        Term::Choose(arms) => arms
            .iter()
            .all(|(_, arm)| producer_dependencies(arm, definitions, publications, dependencies)),
        Term::Union(arms) => arms
            .iter()
            .all(|arm| producer_dependencies(arm, definitions, publications, dependencies)),
    }
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
