//! Machine-selected collective and provider decomposition. Keyed groups route
//! their keys; global groups route complete valuations before per-part Canon,
//! or use contiguous tiles when BE-GroupKey already proves global uniqueness.
use std::collections::BTreeSet;
use cq_lang_sequence::structures::physical::Operator;
use cq_lang_sequence::{Analysis, Catalog, Machine, Program, Term};
use cq_sequence_rewrite_passes::KeyPartition;
use cq_syntax::symbol_name;
use syn::Ident;
use crate::{Advice, Planned};

/// An explicit static decision, following adaptive advice's `select` convention.
/// `true` requires a checked partition; `false` forbids it. Unmentioned definitions
/// remain PL-Cost choices. Authored grades are never overridden.
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
            return Err(syn::Error::new_spanned(&policy.name, "duplicate key-partition policy"));
        }
        if !program.sequence.definitions.iter().any(|d| d.name == policy.name) {
            return Err(syn::Error::new_spanned(&policy.name, "unknown key-partition definition"));
        }
    }
    Ok(())
}

/// Compare independent decisions as well as the joint partition candidate.
/// A provider build can profit even when routing a downstream canon would
/// move its producer out of an existing tiled segment.
pub(crate) fn choices(program: &Program, facts: &Analysis, policies: &[KeyPartitionPolicy]) -> Vec<Vec<KeyPartitionPolicy>> {
    let names: Vec<_> = program.sequence.definitions.iter()
        .filter(|definition| !policies.iter().any(|policy| policy.name == definition.name))
        .filter(|definition| keys(program, facts, &definition.name, &definition.term).is_some())
        .map(|definition| definition.name.clone()).collect();
    let mut choices = vec![policies.to_vec()];
    if names.len() > 1 {
        for selected in &names {
            let mut choice = policies.to_vec();
            choice.extend(names.iter().filter(|name| *name != selected)
                .map(|name| KeyPartitionPolicy { name: name.clone(), select: false }));
            choices.push(choice);
        }
    }
    if policies.iter().any(|policy| policy.select) && !names.is_empty() {
        let mut forced_only = policies.to_vec();
        forced_only.extend(names.into_iter().map(|name| KeyPartitionPolicy { name, select: false }));
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
) -> syn::Result<()> {
    let forced = policies.iter().find(|p| p.select);
    if !enabled {
        return if let Some(policy) = forced {
            Err(syn::Error::new_spanned(&policy.name, "key partition is forced by advice"))
        } else { Ok(()) };
    }
    let pool = machine.root.children.iter().find(|level| {
        !level.collaborative && !level.children.is_empty() && level.width.is_some_and(|n| n > 0)
    });
    let Some(pool) = pool else {
        return if let Some(policy) = forced {
            Err(syn::Error::new_spanned(&policy.name, "BE-KeyPart: machine has no separated distributing pool"))
        } else { Ok(()) };
    };
    let facts = checker.check_shared(&planned.program)?;
    let mut candidates = Vec::new();
    let mut names: BTreeSet<_> = planned.program.sequence.definitions.iter()
        .map(|d| symbol_name(&d.name)).collect();
    for definition in &planned.program.sequence.definitions {
        if policies.iter().any(|p| p.name == definition.name && !p.select)
            || authored.graded.iter().flat_map(|g| &g.grades).any(|g| g.name == definition.name)
        {
            continue;
        }
        let Some(mut keys) = keys(&planned.program, &facts, &definition.name, &definition.term) else { continue };
        let global = global_group(&planned.program, &definition.term);
        if global {
            let name = &definition.name;
            // The earlier checked BE-GroupKey pass removes all unnecessary
            // canonicalizers. An unchanged replay proves that tiles need no
            // cross-part duplicate elimination.
            if cq_sequence_rewrite_passes::group_without_canon_in(
                checker, &planned.program, quote::quote!(#name),
            ).is_ok_and(|target| target == planned.program) {
                keys.clear();
            }
        } else if keys.is_empty() {
            continue;
        }
        if matches!(&definition.term, Term::Store { inner, .. }
            if !matches!(inner.as_ref(), Term::Canon(_) | Term::Group(_, _)))
        {
            let result = &facts.results[&symbol_name(&definition.name)];
            let supported = planned.program.structures.structures.entries.iter().all(|entry| {
                if symbol_name(&entry.name) != *result { return true; }
                let cq_lang_sequence::structures::Structure::Built { keyed, .. } = &entry.structure else { return true };
                let ty = &keyed.ty;
                let routed: syn::Type = syn::parse_quote!(::cq_storage::sharded::Sharded<#ty>);
                catalog.providers.iter().any(|token| crate::catalog::matches_type(&token.ty, &routed))
            });
            if !supported { continue; }
        }
        let mut input = format!("__cq_key_input_{}", symbol_name(&definition.name));
        while !names.insert(input.clone()) { input.push('_'); }
        candidates.push(KeyPartition {
            name: definition.name.clone(),
            input: Ident::new(&input, definition.name.span()),
            keys,
            parts: pool.width.expect("selected distributing pool"),
        });
    }
    for policy in policies.iter().filter(|p| p.select) {
        if !candidates.iter().any(|c| c.name == policy.name) {
            return Err(syn::Error::new_spanned(&policy.name, "BE-KeyPart: forced definition is not an unauthored Canon, Group, or catalog-supported exact-key provider Store"));
        }
    }
    for candidate in candidates {
        let input = candidate.input.clone();
        let name = candidate.name.clone();
        super::push(checker, planned, Advice::KeyPartition(candidate))?;
        super::push(checker, planned, Advice::Grade {
            name: input, level: vec![machine.root.name.clone()], machine: machine.name.clone(),
        })?;
        super::push(checker, planned, Advice::Grade {
            name, level: vec![machine.root.name.clone(), pool.name.clone()], machine: machine.name.clone(),
        })?;
    }
    Ok(())
}

fn keys(program: &Program, facts: &Analysis, name: &Ident, term: &Term) -> Option<Vec<Ident>> {
    match term {
        Term::Canon(_) => {
            let result = facts.results.get(&symbol_name(name))?;
            Some(facts.structures.physical.headings.get(result)?.iter()
                .map(|column| Ident::new(column, name.span())).collect())
        }
        Term::Group(group, _) => program.structures.physical.program.definitions.iter()
            .find(|d| d.name == *group).and_then(|d| match &d.operator {
                Operator::Group(group) if group.keys.is_empty() => Some(
                    facts.structures.physical.headings.get(&symbol_name(&group.input))?.iter()
                        .map(|column| Ident::new(column, name.span())).collect(),
                ),
                Operator::Group(group) => Some(group.keys.iter().cloned().collect()),
                _ => None,
            }),
        Term::Store { inner, .. } if matches!(inner.as_ref(), Term::Canon(_) | Term::Group(_, _)) => keys(program, facts, name, inner),
        Term::Store { inner, .. } if !contains_partition(inner) => {
            let result = facts.results.get(&symbol_name(name))?;
            let mut reads = Vec::new();
            for d in &program.sequence.definitions { lookup_keys(&d.term, result, &mut reads); }
            let first = reads.first()?;
            // Sharded providers hash the complete outer key tuple, including
            // its order. Prefix/range routing is not this runtime protocol.
            if reads.iter().any(|keys| *keys != *first) { return None; }
            let layout = program.structures.structures.entries.iter().find_map(|entry| {
                if symbol_name(&entry.name) != *result { return None; }
                match &entry.structure {
                    cq_lang_sequence::structures::Structure::Built { keyed, .. } => Some(keyed.level_names()),
                    _ => None,
                }
            })?;
            let lookup: Vec<_> = first.iter().map(symbol_name).collect();
            if layout.first()? != &lookup { return None; }
            Some(first.to_vec())
        }
        _ => None,
    }
}

fn global_group(program: &Program, term: &Term) -> bool {
    match term {
        Term::Group(name, _) => program.structures.physical.program.definitions.iter()
            .find(|definition| definition.name == *name)
            .is_some_and(|definition| matches!(&definition.operator,
                Operator::Group(group) if group.keys.is_empty())),
        Term::Store { inner, .. } => global_group(program, inner),
        _ => false,
    }
}

fn lookup_keys<'a>(term: &'a Term, result: &str, reads: &mut Vec<&'a [Ident]>) {
    match term {
        Term::Lookup { provider, keys, inner } => {
            if symbol_name(provider) == result { reads.push(keys); }
            lookup_keys(inner, result, reads);
        }
        Term::Map(_, t) | Term::Compact(_, t) | Term::Flatten(t) | Term::Fold(_, t)
        | Term::Group(_, t) | Term::Canon(t) | Term::Tile { inner: t, .. }
        | Term::PartitionK { inner: t, .. } | Term::Store { inner: t, .. } => lookup_keys(t, result, reads),
        Term::Union(arms) => { for t in arms { lookup_keys(t, result, reads); } }
        Term::Choose(arms) => { for (_, t) in arms { lookup_keys(t, result, reads); } }
        Term::Rows(_) | Term::Ref(_) => {}
    }
}

pub(crate) fn contains_partition(term: &Term) -> bool {
    match term {
        Term::PartitionK { .. } => true,
        Term::Map(_, t) | Term::Compact(_, t) | Term::Flatten(t) | Term::Fold(_, t)
        | Term::Group(_, t) | Term::Canon(t) | Term::Tile { inner: t, .. }
        | Term::Lookup { inner: t, .. } | Term::Store { inner: t, .. } => contains_partition(t),
        Term::Union(arms) => arms.iter().any(contains_partition),
        Term::Choose(arms) => arms.iter().any(|(_, t)| contains_partition(t)),
        Term::Rows(_) | Term::Ref(_) => false,
    }
}
