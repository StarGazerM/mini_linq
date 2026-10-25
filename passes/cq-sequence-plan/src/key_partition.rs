//! Machine-selected collective and provider decomposition. Keyed groups route
//! their keys; global groups route complete valuations before per-part Canon,
//! or use contiguous tiles when BE-GroupKey already proves global uniqueness.
use crate::{Advice, Planned};
use cq_lang_sequence::structures::physical::Operator;
use cq_lang_sequence::{Analysis, Catalog, Machine, Program, Term};
use cq_sequence_rewrite_passes::KeyPartition;
use cq_syntax::symbol_name;
use std::collections::{BTreeMap, BTreeSet};
use syn::Ident;

/// An explicit static decision, following adaptive advice's `select` convention.
/// `true` requires a fixed checked partition; `false` retains the native provider.
/// Unmentioned providers use runtime count choice; collectives remain PL-Cost choices.
/// For global aggregates this controls both tiled partial folds and key-routed
/// canonical partial folds; both return one host-merged row, even on empty input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPartitionPolicy {
    pub name: Ident,
    pub select: bool,
}

impl syn::parse::Parse for KeyPartitionPolicy {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let name = input.parse()?;
        input.parse::<syn::Token![,]>()?;
        let keyword: Ident = input.parse()?;
        if keyword != "select" {
            return Err(syn::Error::new_spanned(keyword, "expected select"));
        }
        let select = input.parse::<syn::LitBool>()?.value;
        Ok(Self { name, select })
    }
}

impl quote::ToTokens for KeyPartitionPolicy {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let Self { name, select } = self;
        tokens.extend(quote::quote!(#name, select #select));
    }
}
/// Decompose with forced/forbidden key-partition decisions. The returned trace
/// contains the same public checked rewrites as ordinary automatic planning.
pub fn decompose_with_key_partitions(
    program: &Program,
    machine: &Machine,
    catalog: &Catalog,
    policies: &[KeyPartitionPolicy],
) -> syn::Result<Planned> {
    super::decompose_in(&mut Default::default(), program, machine, catalog, policies)
}

pub(crate) fn validate(program: &Program, policies: &[KeyPartitionPolicy]) -> syn::Result<()> {
    let mut seen = BTreeSet::new();
    for policy in policies {
        if !seen.insert(symbol_name(&policy.name)) {
            return Err(syn::Error::new_spanned(
                &policy.name,
                "duplicate key-partition policy",
            ));
        }
        if !program
            .sequence
            .definitions
            .iter()
            .any(|d| d.name == policy.name)
        {
            return Err(syn::Error::new_spanned(
                &policy.name,
                "unknown key-partition definition",
            ));
        }
    }
    Ok(())
}

/// Compare independent decisions as well as the joint partition candidate.
/// A provider build can profit even when routing a downstream canon would
/// move its producer out of an existing tiled segment.
pub(crate) fn choices(
    program: &Program,
    facts: &Analysis,
    policies: &[KeyPartitionPolicy],
) -> Vec<Vec<KeyPartitionPolicy>> {
    let names: Vec<_> = program
        .sequence
        .definitions
        .iter()
        .filter(|definition| !policies.iter().any(|policy| policy.name == definition.name))
        .filter(|definition| !provider_store(&definition.term))
        .filter(|definition| keys(program, facts, &definition.name, &definition.term).is_some())
        .map(|definition| definition.name.clone())
        .collect();
    let mut choices = vec![policies.to_vec()];
    if names.len() > 1 {
        for selected in &names {
            let mut choice = policies.to_vec();
            choice.extend(names.iter().filter(|name| *name != selected).map(|name| {
                KeyPartitionPolicy {
                    name: name.clone(),
                    select: false,
                }
            }));
            choices.push(choice);
        }
    }
    if policies.iter().any(|policy| policy.select) && !names.is_empty() {
        let mut forced_only = policies.to_vec();
        forced_only.extend(names.into_iter().map(|name| KeyPartitionPolicy {
            name,
            select: false,
        }));
        choices.push(forced_only);
    }
    choices
}

pub(crate) fn apply(
    checker: &mut cq_lang_sequence::contract::Checker,
    planned: &mut Planned,
    authored: &Program,
    machine: &Machine,
    catalog: &Catalog,
    policies: &[KeyPartitionPolicy],
    enabled: bool,
    providers: bool,
    partial_groups: bool,
    unique_groups: &BTreeSet<String>,
) -> syn::Result<()> {
    let applies = |policy: &&KeyPartitionPolicy| {
        policy.select
            && authored
                .sequence
                .definitions
                .iter()
                .find(|d| d.name == policy.name)
                .is_some_and(|d| provider_store(&d.term) == providers)
    };
    let forced = policies.iter().find(applies);
    if !enabled && let Some(policy) = forced {
        return Err(syn::Error::new_spanned(
            &policy.name,
            "key partition is forced by advice",
        ));
    }
    let pool = machine.root.children.iter().find(|level| {
        !level.collaborative && !level.children.is_empty() && level.width.is_some_and(|n| n > 0)
    });
    let Some(pool) = pool else {
        return if let Some(policy) = forced {
            Err(syn::Error::new_spanned(
                &policy.name,
                "BE-KeyPart: machine has no separated distributing pool",
            ))
        } else {
            Ok(())
        };
    };
    // Decomposition may require range cursors even when construction needed
    // only exact lookup. Provider routing must inspect the completed schedule.
    let facts = if providers {
        std::rc::Rc::new(checker.check_grading(&planned.program, machine)?)
    } else {
        checker.check_shared(&planned.program)?
    };
    let mut candidates = Vec::new();
    let mut names: BTreeSet<_> = planned
        .program
        .sequence
        .definitions
        .iter()
        .map(|d| symbol_name(&d.name))
        .collect();
    for definition in &planned.program.sequence.definitions {
        if policies
            .iter()
            .any(|p| p.name == definition.name && !p.select)
            || authored
                .graded
                .iter()
                .flat_map(|g| &g.grades)
                .any(|g| g.name == definition.name)
        {
            continue;
        }
        let provider = provider_store(&definition.term);
        if provider != providers || (!enabled && !provider) {
            continue;
        }
        let Some(mut keys) = keys(&planned.program, &facts, &definition.name, &definition.term)
        else {
            continue;
        };
        if group_name(&definition.term).is_some()
            && (partial_groups || !keyed_group(&planned.program, &definition.term))
        {
            // BE-GroupKey was checked once before candidate enumeration.
            if unique_groups.contains(&symbol_name(&definition.name)) {
                keys.clear();
            }
        } else if keys.is_empty() {
            continue;
        }
        if provider {
            let result = &facts.results[&symbol_name(&definition.name)];
            // A type/layout belongs to the physical provider, not to one of its
            // Sequence publication aliases. A forbid must retain that type.
            if policies.iter().any(|policy| {
                !policy.select && facts.results.get(&symbol_name(&policy.name)) == Some(result)
            }) {
                continue;
            }
            let supported = planned
                .program
                .structures
                .structures
                .entries
                .iter()
                .all(|entry| {
                    if symbol_name(&entry.name) != *result {
                        return true;
                    }
                    let cq_lang_sequence::structures::Structure::Built { keyed, .. } =
                        &entry.structure
                    else {
                        return true;
                    };
                    let ty = &keyed.ty;
                    let routed: syn::Type = syn::parse_quote!(::cq_storage::sharded::Sharded<#ty>);
                    facts.obligations.iter().all(|obligation| match obligation {
                        cq_lang_sequence::Obligation::Provider {
                            provider,
                            capability,
                            ..
                        } if provider == result => crate::catalog::provider_available(
                            &facts, catalog, provider, capability, &routed,
                        ),
                        _ => true,
                    })
                });
            if !supported {
                continue;
            }
        }
        let mut input = format!("__cq_key_input_{}", symbol_name(&definition.name));
        while !names.insert(input.clone()) {
            input.push('_');
        }
        candidates.push(KeyPartition {
            name: definition.name.clone(),
            input: Ident::new(&input, definition.name.span()),
            keys,
            parts: pool.width.expect("selected distributing pool"),
            threshold: (provider
                && !policies
                    .iter()
                    .any(|p| p.name == definition.name && p.select))
            .then_some(machine.key_partition_threshold),
        });
    }
    if providers {
        // Host staging and the final provider publication can denote the same
        // physical result. Retype exactly once, preferring explicit force and
        // otherwise the final publication preceding every keyed consumer.
        let mut selected = BTreeMap::new();
        let positions: BTreeMap<_, _> = planned
            .program
            .sequence
            .definitions
            .iter()
            .enumerate()
            .map(|(index, definition)| (symbol_name(&definition.name), index))
            .collect();
        for (index, candidate) in candidates.iter().enumerate() {
            let result = &facts.results[&symbol_name(&candidate.name)];
            let forced = policies
                .iter()
                .any(|p| p.name == candidate.name && p.select);
            let position = positions[&symbol_name(&candidate.name)];
            if facts.provider_reads.iter().any(|(reader, providers)| {
                providers.contains(result) && positions[reader] <= position
            }) {
                if forced {
                    return Err(syn::Error::new_spanned(
                        &candidate.name,
                        "BE-KeyPartProvider: publication must precede every keyed read of its physical provider",
                    ));
                }
                continue;
            }
            if let Some(&previous) = selected.get(result) {
                let previous: &KeyPartition = &candidates[previous];
                let previous_forced = policies.iter().any(|p| p.name == previous.name && p.select);
                if forced && previous_forced {
                    return Err(syn::Error::new_spanned(
                        &candidate.name,
                        "BE-KeyPartProvider: multiple forced publications name the same physical provider",
                    ));
                }
                if previous_forced {
                    continue;
                }
            }
            selected.insert(result, index);
        }
        candidates = candidates
            .into_iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                let result = &facts.results[&symbol_name(&candidate.name)];
                (selected.get(result) == Some(&index)).then_some(candidate)
            })
            .collect();
    }
    for policy in policies.iter().filter(applies) {
        if !candidates.iter().any(|c| c.name == policy.name) {
            return Err(syn::Error::new_spanned(
                &policy.name,
                "BE-KeyPart: forced definition is not an unauthored Canon, Group, or catalog-supported exact-key provider Store",
            ));
        }
    }
    for candidate in candidates {
        let input = candidate.input.clone();
        let name = candidate.name.clone();
        let partial = candidate.keys.is_empty();
        let producer_level = (!partial)
            .then(|| publication_producer_level(&planned.program, machine, &name))
            .flatten();
        super::push(checker, planned, Advice::KeyPartition(candidate))?;
        if planned
            .program
            .sequence
            .definitions
            .iter()
            .any(|d| d.name == input)
        {
            if let Some(level) = producer_level {
                let mut fresh = format!("__cq_key_producer_{}", symbol_name(&name));
                while !names.insert(fresh.clone()) {
                    fresh.push('_');
                }
                let producer = Ident::new(&fresh, name.span());
                super::push(
                    checker,
                    planned,
                    Advice::Publish {
                        name: input.clone(),
                        producer: producer.clone(),
                    },
                )?;
                super::push(
                    checker,
                    planned,
                    Advice::Grade {
                        name: producer,
                        level,
                        machine: machine.name.clone(),
                    },
                )?;
            }
            super::push(
                checker,
                planned,
                Advice::Grade {
                    name: input,
                    level: vec![machine.root.name.clone()],
                    machine: machine.name.clone(),
                },
            )?;
        }
        if partial {
            let mut producer = vec![machine.root.name.clone(), pool.name.clone()];
            if let [child] = pool.children.as_slice() {
                producer.push(child.name.clone());
            }
            for view in partial_views(&planned.program, &name) {
                if authored
                    .graded
                    .iter()
                    .flat_map(|g| &g.grades)
                    .any(|g| g.name == view)
                {
                    continue;
                }
                super::push(
                    checker,
                    planned,
                    Advice::Grade {
                        name: view,
                        level: producer.clone(),
                        machine: machine.name.clone(),
                    },
                )?;
            }
        }
        super::push(
            checker,
            planned,
            Advice::Grade {
                name,
                level: vec![machine.root.name.clone(), pool.name.clone()],
                machine: machine.name.clone(),
            },
        )?;
    }
    Ok(())
}

fn publication_producer_level(
    program: &Program,
    machine: &Machine,
    name: &Ident,
) -> Option<Vec<Ident>> {
    let grades = &program.graded.as_ref()?.grades;
    let mut level = grades
        .iter()
        .find(|grade| grade.name == *name)?
        .level
        .clone();
    if level.len() == 1 {
        return None;
    }
    let definition = program
        .sequence
        .definitions
        .iter()
        .find(|d| d.name == *name)?;
    if let Term::Store { inner, .. } = &definition.term
        && let Term::Ref(source) = inner.as_ref()
        && grades
            .iter()
            .any(|grade| grade.name == *source && grade.level.len() == 1)
    {
        // The original boundary already uploads a completed host publication.
        return None;
    }
    let node = level.iter().skip(1).try_fold(&machine.root, |node, name| {
        node.children.iter().find(|child| child.name == *name)
    })?;
    if matches!(definition.term, Term::Store { .. })
        && !node.collaborative
        && let [child] = node.children.as_slice()
    {
        level.push(child.name.clone());
    }
    Some(level)
}

fn partial_views(program: &Program, name: &Ident) -> Vec<Ident> {
    fn visit(program: &Program, term: &Term, views: &mut Vec<Ident>) {
        match term {
            Term::Ref(name) => {
                let definition = program
                    .sequence
                    .definitions
                    .iter()
                    .find(|d| d.name == *name)
                    .expect("checked reference");
                if matches!(definition.term, Term::Store { .. }) {
                    return;
                }
                views.push(name.clone());
                visit(program, &definition.term, views);
            }
            Term::Map(_, inner)
            | Term::Compact(_, inner)
            | Term::Flatten(inner)
            | Term::Tile { inner, .. }
            | Term::Lookup { inner, .. } => visit(program, inner, views),
            _ => {}
        }
    }
    let mut term = &program
        .sequence
        .definitions
        .iter()
        .find(|d| d.name == *name)
        .expect("checked group definition")
        .term;
    while let Term::Store { inner, .. } = term {
        term = inner;
    }
    let mut views = Vec::new();
    if let Term::Group(_, inner) = term {
        visit(program, inner, &mut views);
    }
    views
}

fn provider_store(term: &Term) -> bool {
    matches!(term, Term::Store { inner, .. }
        if !matches!(inner.as_ref(), Term::Canon(_) | Term::Group(_, _)))
}

fn keys(program: &Program, facts: &Analysis, name: &Ident, term: &Term) -> Option<Vec<Ident>> {
    match term {
        Term::Canon(_) => {
            let result = facts.results.get(&symbol_name(name))?;
            Some(
                facts
                    .structures
                    .physical
                    .headings
                    .get(result)?
                    .iter()
                    .map(|column| Ident::new(column, name.span()))
                    .collect(),
            )
        }
        Term::Group(group, _) => program
            .structures
            .physical
            .program
            .definitions
            .iter()
            .find(|d| d.name == *group)
            .and_then(|d| match &d.operator {
                Operator::Group(group) if group.keys.is_empty() => Some(
                    facts
                        .structures
                        .physical
                        .headings
                        .get(&symbol_name(&group.input))?
                        .iter()
                        .map(|column| Ident::new(column, name.span()))
                        .collect(),
                ),
                Operator::Group(group) => Some(group.keys.iter().cloned().collect()),
                _ => None,
            }),
        Term::Store { inner, .. }
            if matches!(inner.as_ref(), Term::Canon(_) | Term::Group(_, _)) =>
        {
            keys(program, facts, name, inner)
        }
        Term::Store { inner, .. } if !contains_partition(inner) => {
            // BE-KeyPart consumes flat valuations, not an existing tiled or
            // nested lookup carrier. Keep those authored/native schedules intact.
            if facts
                .shapes
                .get(&symbol_name(name))
                .is_none_or(|shape| shape.len() != 1)
            {
                return None;
            }
            let result = facts.results.get(&symbol_name(name))?;
            let reads: Vec<_> = facts
                .obligations
                .iter()
                .filter_map(|obligation| match obligation {
                    cq_lang_sequence::Obligation::Provider {
                        provider,
                        keys,
                        capability,
                        ..
                    } if provider == result => Some((keys, capability)),
                    _ => None,
                })
                .collect();
            let (first, _) = *reads.first()?;
            // Filtered native layouts may reconstruct constant keys that are
            // absent from their carried valuations. Routing cannot read them.
            if first.iter().any(|key| {
                !facts.structures.physical.headings[result].contains(key)
                    || facts
                        .structures
                        .filtered
                        .get(result)
                        .is_some_and(|filtered| {
                            filtered
                                .constants
                                .iter()
                                .any(|binding| symbol_name(&binding.column) == *key)
                        })
            }) {
                return None;
            }
            // Sharded providers hash the complete outer key tuple, including
            // its order. Prefix/range routing is not this runtime protocol.
            if reads.iter().any(|(keys, capability)| {
                *keys != first
                    || !matches!(
                        capability,
                        cq_lang_sequence::Capability::Exact
                            | cq_lang_sequence::Capability::Membership
                    )
            }) {
                return None;
            }
            let layout = program
                .structures
                .structures
                .entries
                .iter()
                .find_map(|entry| {
                    if symbol_name(&entry.name) != *result {
                        return None;
                    }
                    match &entry.structure {
                        cq_lang_sequence::structures::Structure::Built { keyed, .. } => {
                            Some(keyed.level_names())
                        }
                        _ => None,
                    }
                })?;
            if layout.first()? != first {
                return None;
            }
            Some(
                first
                    .iter()
                    .map(|key| Ident::new(key, name.span()))
                    .collect(),
            )
        }
        _ => None,
    }
}

fn group_name(term: &Term) -> Option<&Ident> {
    match term {
        Term::Group(name, _) => Some(name),
        Term::Store { inner, .. } => group_name(inner),
        _ => None,
    }
}

fn keyed_group(program: &Program, term: &Term) -> bool {
    group_name(term).is_some_and(|name| {
        program
            .structures
            .physical
            .program
            .definitions
            .iter()
            .any(|definition| {
                definition.name == *name
                    && matches!(&definition.operator,
                Operator::Group(group) if !group.keys.is_empty())
            })
    })
}

pub(crate) fn has_keyed_partials(program: &Program, unique_groups: &BTreeSet<String>) -> bool {
    program.sequence.definitions.iter().any(|definition| {
        unique_groups.contains(&symbol_name(&definition.name))
            && keyed_group(program, &definition.term)
    })
}

pub(crate) fn contains_partition(term: &Term) -> bool {
    match term {
        Term::PartitionK { .. } => true,
        Term::Map(_, t)
        | Term::Compact(_, t)
        | Term::Flatten(t)
        | Term::Fold(_, t)
        | Term::Group(_, t)
        | Term::Canon(t)
        | Term::Tile { inner: t, .. }
        | Term::Lookup { inner: t, .. }
        | Term::Store { inner: t, .. } => contains_partition(t),
        Term::Union(arms) => arms.iter().any(contains_partition),
        Term::Choose(arms) => arms.iter().any(|(_, t)| contains_partition(t)),
        Term::Rows(_) | Term::Ref(_) => false,
    }
}
