use std::collections::{BTreeMap, BTreeSet};

use cq_lang_sequence::structures::physical::Operator;
use cq_lang_sequence::{Machine, Program, Term};
use cq_syntax::symbol_name;
use syn::Ident;

use crate::Advice;

pub(crate) fn boundaries(
    program: &Program,
    machine: &Machine,
    facts: &cq_lang_sequence::Analysis,
) -> Vec<Advice> {
    let Some(child) = machine.root.children.first() else {
        return Vec::new();
    };
    let host = vec![machine.root.name.clone()];
    let mut producer_level = vec![machine.root.name.clone(), child.name.clone()];
    if let Some(producer) = child.children.first() {
        producer_level.push(producer.name.clone());
    }
    let definitions = &program.sequence.definitions;
    let indices: BTreeMap<_, _> = definitions
        .iter()
        .enumerate()
        .map(|(i, definition)| (symbol_name(&definition.name), i))
        .collect();
    let host_operations = host_operations(program, facts);
    let mut host_feeds = BTreeSet::new();
    for definition in definitions {
        let mut refs = Vec::new();
        references(&definition.term, &mut refs);
        if requires_host(&definition.term, &host_operations)
            || refs.iter().any(|name| {
                host_feeds.contains(&symbol_name(name))
                    && !matches!(
                        definitions[indices[&symbol_name(name)]].term,
                        Term::Store { .. }
                    )
            })
        {
            host_feeds.insert(symbol_name(&definition.name));
        }
    }
    let authored: BTreeSet<_> = program
        .graded
        .iter()
        .flat_map(|graded| &graded.grades)
        .map(|grade| symbol_name(&grade.name))
        .collect();
    let mut names: BTreeSet<_> = indices.keys().cloned().collect();
    let mut published = BTreeSet::new();
    let mut advice = Vec::new();
    for definition in definitions {
        if !matches!(definition.term, Term::Store { .. })
            || authored.contains(&symbol_name(&definition.name))
        {
            continue;
        }
        let Some(result) = facts.results.get(&symbol_name(&definition.name)) else {
            continue;
        };
        if !facts.structures.filtered.contains_key(result) || string_row(facts, result) {
            continue;
        }
        let physical = program
            .structures
            .physical
            .program
            .definitions
            .iter()
            .find(|physical| symbol_name(&physical.name) == *result)
            .expect("checked result");
        if !host_predicate(&physical.operator)
            && !physical
                .operator
                .operands()
                .iter()
                .any(|input| string_row(facts, &symbol_name(input)))
        {
            continue;
        }
        let mut fresh = format!("__sequence_host_{}", symbol_name(&definition.name));
        while !names.insert(fresh.clone()) {
            fresh.push('_');
        }
        let producer = Ident::new(&fresh, definition.name.span());
        advice.push(Advice::Publish {
            name: definition.name.clone(),
            producer: producer.clone(),
        });
        advice.push(Advice::Grade {
            name: producer,
            level: host.clone(),
            machine: machine.name.clone(),
        });
        advice.push(Advice::Grade {
            name: definition.name.clone(),
            level: vec![machine.root.name.clone(), child.name.clone()],
            machine: machine.name.clone(),
        });
    }
    for definition in definitions {
        let term = match &definition.term {
            Term::Store { inner, .. } => inner.as_ref(),
            term => term,
        };
        let Term::Union(arms) = term else { continue };
        let mut domains = BTreeSet::new();
        driver_domains(term, definitions, &indices, &mut domains);
        if domains.len() <= 1
            || authored.contains(&symbol_name(&definition.name))
            || !arms.iter().all(|arm| matches!(arm, Term::Ref(_)))
        {
            continue;
        }
        advice.push(Advice::Grade {
            name: definition.name.clone(),
            level: host.clone(),
            machine: machine.name.clone(),
        });
        for arm in arms {
            let Term::Ref(name) = arm else { unreachable!() };
            if authored.contains(&symbol_name(name)) || !published.insert(symbol_name(name)) {
                continue;
            }
            if host_feeds.contains(&symbol_name(name)) {
                advice.push(Advice::Store(name.clone()));
                advice.push(Advice::Grade {
                    name: name.clone(),
                    level: host.clone(),
                    machine: machine.name.clone(),
                });
                continue;
            }
            let mut fresh = format!("__sequence_publish_{}", symbol_name(name));
            while !names.insert(fresh.clone()) {
                fresh.push('_');
            }
            let producer = Ident::new(&fresh, name.span());
            advice.push(Advice::Publish {
                name: name.clone(),
                producer: producer.clone(),
            });
            advice.push(Advice::Grade {
                name: producer,
                level: producer_level.clone(),
                machine: machine.name.clone(),
            });
            advice.push(Advice::Grade {
                name: name.clone(),
                level: host.clone(),
                machine: machine.name.clone(),
            });
        }
    }
    advice
}

pub(crate) fn advise(
    program: &Program,
    machine: &Machine,
    parallel: bool,
    facts: &cq_lang_sequence::Analysis,
) -> Vec<Advice> {
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
    let opaque = host_operations(program, facts);
    let mut stores = BTreeSet::new();
    let mut levels: Vec<Option<Vec<Ident>>> = definitions
        .iter()
        .map(|d| authored.get(&symbol_name(&d.name)).cloned())
        .collect();
    let mut advice = Vec::new();
    for (i, d) in definitions.iter().enumerate() {
        let at_host = matches!(d.term, Term::Rows(_)) || requires_host(&d.term, &opaque);
        if at_host && levels[i].is_none() {
            levels[i] = Some(host.clone());
        }
        if i + 1 == definitions.len() && !matches!(d.term, Term::Store { .. }) {
            advice.push(Advice::Store(d.name.clone()));
            stores.insert(i);
        }
    }
    // Host-only evaluation wins before descendant consumers propagate their
    // preferred grades. Follow all unstored feeds back to the host boundary.
    for i in (0..definitions.len()).rev() {
        if levels[i].as_ref() != Some(&host) {
            continue;
        }
        let mut refs = Vec::new();
        references(&definitions[i].term, &mut refs);
        for name in refs {
            if let Some(&source) = indices.get(&symbol_name(name))
                && !authored.contains_key(&symbol_name(name))
                && !matches!(definitions[source].term, Term::Store { .. })
                && !stores.contains(&source)
            {
                levels[source] = Some(host.clone());
            }
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

fn requires_host(term: &Term, opaque: &BTreeSet<String>) -> bool {
    match term {
        Term::Map(name, inner) | Term::Compact(name, inner) => {
            opaque.contains(&symbol_name(name)) || requires_host(inner, opaque)
        }
        // Provider construction has its own stored boundary. Reading its
        // recorded layout does not re-evaluate its host predicate.
        Term::Lookup { inner, .. }
        | Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Canon(inner)
        | Term::Store { inner, .. }
        | Term::Tile { inner, .. } => requires_host(inner, opaque),
        Term::Union(arms) => arms.iter().any(|term| requires_host(term, opaque)),
        Term::Choose(arms) => arms.iter().any(|(_, term)| requires_host(term, opaque)),
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

fn driver_domains(
    term: &Term,
    definitions: &[cq_lang_sequence::Definition],
    indices: &BTreeMap<String, usize>,
    domains: &mut BTreeSet<String>,
) {
    match term {
        Term::Rows(name) => {
            domains.insert(symbol_name(name));
        }
        Term::Ref(name) => {
            let definition = &definitions[indices[&symbol_name(name)]];
            if matches!(definition.term, Term::Store { .. }) {
                domains.insert(symbol_name(name));
            } else {
                driver_domains(&definition.term, definitions, indices, domains);
            }
        }
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Canon(inner)
        | Term::Tile { inner, .. }
        | Term::Store { inner, .. }
        | Term::Lookup { inner, .. } => driver_domains(inner, definitions, indices, domains),
        Term::Union(arms) => {
            for arm in arms {
                driver_domains(arm, definitions, indices, domains);
            }
        }
        Term::Choose(arms) => {
            for (_, arm) in arms {
                driver_domains(arm, definitions, indices, domains);
            }
        }
    }
}

fn host_predicate(operator: &Operator) -> bool {
    use cq_syntax::constraint::{Constraint, Term as Scalar};
    fn string(expression: &syn::Expr) -> bool {
        matches!(expression, syn::Expr::Lit(literal) if matches!(literal.lit, syn::Lit::Str(_)))
    }
    fn scalar(term: &Scalar) -> bool {
        matches!(term, Scalar::Literal(expression) if string(expression))
    }
    fn constraint(predicate: &Constraint) -> bool {
        match predicate {
            Constraint::Compare { left, right, .. } => scalar(left) || scalar(right),
            Constraint::Between { term, low, high } => scalar(term) || scalar(low) || scalar(high),
            Constraint::In { term, literals } => scalar(term) || literals.iter().any(string),
            Constraint::Or(arms) => arms.iter().flatten().any(constraint),
            Constraint::Total { .. } => false,
        }
    }
    match operator {
        Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
            select.condition.constraints.iter().any(constraint)
        }
        _ => false,
    }
}

fn string_type(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Path(path) if path.path.segments.last()
        .is_some_and(|segment| segment.ident == "String"))
}

fn string_row(facts: &cq_lang_sequence::Analysis, name: &str) -> bool {
    facts.structures.layouts.get(name).is_some_and(|columns| {
        columns
            .iter()
            .any(|column| string_type(&facts.structures.physical.rows[name].types[column]))
    })
}

fn host_operations(program: &Program, facts: &cq_lang_sequence::Analysis) -> BTreeSet<String> {
    program
        .structures
        .physical
        .program
        .definitions
        .iter()
        .filter(|definition| {
            matches!(definition.operator, Operator::Bind(_) | Operator::Guard(_))
                || host_predicate(&definition.operator)
                || string_row(facts, &symbol_name(&definition.name))
                || definition
                    .operator
                    .operands()
                    .iter()
                    .any(|input| string_row(facts, &symbol_name(input)))
        })
        .map(|definition| symbol_name(&definition.name))
        .collect()
}
