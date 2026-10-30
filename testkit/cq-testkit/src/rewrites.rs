//! Authored rewrite candidates for correspondence and execution tests.
//!
//! These helpers depend only on language contracts. They do not prove pass
//! correspondence; callers must check each candidate against its source and
//! compare execution with their independent reference.
//! `merged` also adjusts retention modes for the rewritten readers.
use cq_lang_physical::{self as physical, Choice, Mode, Operator};
use std::collections::{BTreeMap, BTreeSet};

/// Replace keyed probes with merges and adjust retention for their readers.
pub fn merged(source: &physical::Program) -> syn::Result<physical::Program> {
    let mut target = source.clone();
    for definition in &mut target.program.definitions {
        if let Operator::Probe(_, k) = &definition.operator {
            if !k.columns.is_empty() {
                let mut k = k.clone();
                k.choice = None;
                definition.operator = Operator::Merge(Default::default(), k);
            }
        }
    }
    retain(&mut target);
    Ok(target)
}

/// Mark projections and the returned relation canonical where no key is inherited.
pub fn canonical_projects(source: &physical::Program) -> syn::Result<physical::Program> {
    let mut target = source.clone();
    let facts = physical::contract::check(source)?;
    let mut keys: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for d in &mut target.program.definitions {
        let name = d.name.to_string();
        let heading = facts.headings[&name].clone();
        let inherited =
            physical::inherited_key(&d.operator, &heading, &target.program.host_sets, |id| {
                keys.get(&id.to_string()).cloned()
            });
        d.canon = (inherited.is_none()
            && (matches!(d.operator, Operator::Project(_)) || d.name == target.program.returned))
            .then(Default::default);
        if let Some(key) = inherited.or_else(|| d.canon.is_some().then_some(heading)) {
            keys.insert(name, key);
        }
    }
    Ok(target)
}

/// Select the smallest branch for recognized binary intersections.
pub fn smallest(source: &physical::Program) -> syn::Result<physical::Program> {
    let mut target = source.clone();
    let facts = physical::contract::check(source)?;
    for i in 0..source.program.definitions.len().saturating_sub(1) {
        if physical::binary_intersection(
            &source.program.definitions[i],
            &source.program.definitions[i + 1],
            &facts,
        )
        .is_some()
        {
            if let Operator::Probe(_, k) = &mut target.program.definitions[i].operator {
                k.choice = Some(Choice::Smallest(Default::default()));
            }
        }
    }
    Ok(target)
}

fn retain(target: &mut physical::Program) {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut keyed = BTreeSet::new();
    let mut ordered: BTreeMap<String, Vec<Vec<String>>> = BTreeMap::new();
    for d in &target.program.definitions {
        if let Some(input) = d.operator.input() {
            *counts.entry(input.to_string()).or_default() += 1;
        }
        match &d.operator {
            Operator::Probe(_, k) | Operator::Semi(_, k) => {
                *counts.entry(k.right.to_string()).or_default() += 1;
                keyed.insert(k.right.to_string());
            }
            Operator::Merge(_, k) => {
                *counts.entry(k.right.to_string()).or_default() += 1;
                let cols: Vec<_> = k.columns.iter().map(ToString::to_string).collect();
                ordered
                    .entry(k.left.to_string())
                    .or_default()
                    .push(cols.clone());
                ordered.entry(k.right.to_string()).or_default().push(cols);
            }
            _ => {}
        }
    }
    *counts
        .entry(target.program.returned.to_string())
        .or_default() += 1;
    for d in &mut target.program.definitions {
        if matches!(d.operator, Operator::Rename(..)) {
            continue;
        }
        let name = d.name.to_string();
        let own = match &d.operator {
            Operator::Merge(_, k) => Some(
                k.columns
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        };
        let retained = counts.get(&name).copied().unwrap_or(0) >= 2
            || keyed.contains(&name)
            || ordered
                .get(&name)
                .into_iter()
                .flatten()
                .any(|c| Some(c) != own.as_ref());
        d.mode = if retained {
            Mode::Let(Default::default())
        } else {
            Mode::Pipe(Default::default())
        };
    }
}
