use std::collections::{BTreeMap, BTreeSet};

use cq_lang_sequence::structures::physical::Operator;
use cq_lang_sequence::{Machine, Program, Term};
use cq_syntax::symbol_name;
use syn::Ident;

use crate::Advice;

pub(crate) fn advise(program: &Program, machine: &Machine, parallel: bool) -> Vec<Advice> {
    let host = vec![machine.root.name.clone()];
    let publication = if parallel {
        machine
            .root
            .children
            .first()
            .map(|child| vec![machine.root.name.clone(), child.name.clone()])
            .unwrap_or_else(|| host.clone())
    } else {
        host.clone()
    };
    let producer = if publication.len() > 1 {
        machine.root.children[0]
            .children
            .first()
            .map(|child| {
                let mut path = publication.clone();
                path.push(child.name.clone());
                path
            })
            .unwrap_or_else(|| publication.clone())
    } else {
        host.clone()
    };
    let definitions = &program.sequence.definitions;
    let indices: BTreeMap<_, _> = definitions
        .iter()
        .enumerate()
        .map(|(i, d)| (symbol_name(&d.name), i))
        .collect();
    let authored: BTreeMap<_, _> = program
        .graded
        .as_ref()
        .into_iter()
        .flat_map(|g| &g.grades)
        .map(|g| {
            let mut level = g.level.clone();
            if level
                .first()
                .is_some_and(|n| symbol_name(n) != symbol_name(&machine.root.name))
            {
                level.insert(0, machine.root.name.clone());
            }
            (symbol_name(&g.name), level)
        })
        .collect();
    let opaque: BTreeSet<_> = program
        .structures
        .physical
        .program
        .definitions
        .iter()
        .filter(|d| matches!(d.operator, Operator::Bind(_) | Operator::Guard(_)))
        .map(|d| symbol_name(&d.name))
        .collect();
    let mut opaque_dependencies = opaque.clone();
    for definition in &program.structures.physical.program.definitions {
        if definition
            .operator
            .operands()
            .iter()
            .any(|name| opaque_dependencies.contains(&symbol_name(name)))
        {
            opaque_dependencies.insert(symbol_name(&definition.name));
        }
    }
    let mut stores = BTreeSet::new();
    let mut levels: Vec<Option<Vec<Ident>>> = definitions
        .iter()
        .map(|d| authored.get(&symbol_name(&d.name)).cloned())
        .collect();
    let mut advice = Vec::new();
    for (i, d) in definitions.iter().enumerate() {
        let at_host = matches!(d.term, Term::Rows(_))
            || requires_host(&d.term, &opaque, &opaque_dependencies);
        if at_host && levels[i].is_none() {
            levels[i] = Some(host.clone());
        }
        if i + 1 == definitions.len() && !matches!(d.term, Term::Store { .. }) {
            advice.push(Advice::Store(d.name.clone()));
            stores.insert(i);
        }
    }
    for i in (0..definitions.len()).rev() {
        let d = &definitions[i];
        let stored = matches!(d.term, Term::Store { .. }) || stores.contains(&i);
        if levels[i].is_none() {
            levels[i] = Some(if stored {
                if i + 1 == definitions.len()
                    || has_lookup(&d.term, definitions, &indices, &mut BTreeSet::new())
                {
                    publication.clone()
                } else {
                    host.clone()
                }
            } else {
                producer.clone()
            });
        }
        let level = levels[i].as_ref().unwrap().clone();
        let read_level = if stored && level == publication && publication != host {
            producer.clone()
        } else {
            level
        };
        let mut refs = Vec::new();
        references(&d.term, &mut refs);
        for name in refs {
            if let Some(&source) = indices.get(&symbol_name(name))
                && levels[source].is_none()
                && !matches!(definitions[source].term, Term::Store { .. })
                && !stores.contains(&source)
            {
                levels[source] = Some(read_level.clone());
            }
        }
    }
    // A host view stays a view when its retained consumer is also at host.
    // Materialize only actual crossings into a descendant execution extent.
    for (i, definition) in definitions.iter().enumerate() {
        let level = levels[i].as_ref().unwrap();
        let stored = matches!(definition.term, Term::Store { .. }) || stores.contains(&i);
        let read_level = if stored && *level == publication && publication != host {
            &producer
        } else {
            level
        };
        let mut refs = Vec::new();
        references(&definition.term, &mut refs);
        for name in refs {
            if let Some(&source) = indices.get(&symbol_name(name))
                && let Some(source_level) = &levels[source]
                && source_level != read_level
                && read_level.starts_with(source_level)
                && !matches!(definitions[source].term, Term::Store { .. })
                && stores.insert(source)
            {
                advice.push(Advice::Store(definitions[source].name.clone()));
            }
        }
    }
    for (d, level) in definitions.iter().zip(levels) {
        if !authored.contains_key(&symbol_name(&d.name)) {
            advice.push(Advice::Grade {
                name: d.name.clone(),
                level: level.unwrap_or_else(|| host.clone()),
                machine: machine.name.clone(),
            });
        }
    }
    advice
}

fn has_lookup(
    term: &Term,
    definitions: &[cq_lang_sequence::Definition],
    indices: &BTreeMap<String, usize>,
    seen: &mut BTreeSet<usize>,
) -> bool {
    match term {
        Term::Lookup { .. } => true,
        Term::Ref(name) => indices.get(&symbol_name(name)).is_some_and(|&i| {
            seen.insert(i) && has_lookup(&definitions[i].term, definitions, indices, seen)
        }),
        Term::Rows(_) => false,
        Term::Map(_, t)
        | Term::Compact(_, t)
        | Term::Flatten(t)
        | Term::Fold(_, t)
        | Term::Canon(t)
        | Term::Tile { inner: t, .. }
        | Term::Store { inner: t, .. } => has_lookup(t, definitions, indices, seen),
        Term::Union(terms) => terms
            .iter()
            .any(|t| has_lookup(t, definitions, indices, seen)),
        Term::Choose(arms) => arms
            .iter()
            .any(|(_, t)| has_lookup(t, definitions, indices, seen)),
    }
}

fn requires_host(term: &Term, opaque: &BTreeSet<String>, dependencies: &BTreeSet<String>) -> bool {
    match term {
        Term::Map(name, inner) | Term::Compact(name, inner) => {
            opaque.contains(&symbol_name(name)) || requires_host(inner, opaque, dependencies)
        }
        Term::Lookup {
            provider, inner, ..
        } => {
            dependencies.contains(&symbol_name(provider))
                || requires_host(inner, opaque, dependencies)
        }
        Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Canon(inner)
        | Term::Store { inner, .. }
        | Term::Tile { inner, .. } => requires_host(inner, opaque, dependencies),
        Term::Union(arms) => arms
            .iter()
            .any(|term| requires_host(term, opaque, dependencies)),
        Term::Choose(arms) => arms
            .iter()
            .any(|(_, term)| requires_host(term, opaque, dependencies)),
        _ => false,
    }
}

fn references<'a>(term: &'a Term, result: &mut Vec<&'a Ident>) {
    match term {
        Term::Ref(name) => result.push(name),
        Term::Rows(_) => {}
        Term::Map(_, t)
        | Term::Compact(_, t)
        | Term::Flatten(t)
        | Term::Fold(_, t)
        | Term::Canon(t)
        | Term::Tile { inner: t, .. }
        | Term::Store { inner: t, .. }
        | Term::Lookup { inner: t, .. } => references(t, result),
        Term::Union(terms) => {
            for t in terms {
                references(t, result);
            }
        }
        Term::Choose(arms) => {
            for (_, t) in arms {
                references(t, result);
            }
        }
    }
}
