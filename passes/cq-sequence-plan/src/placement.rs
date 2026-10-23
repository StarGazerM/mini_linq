use std::collections::{BTreeMap, BTreeSet};

use cq_lang_sequence::structures::physical::Operator;
use cq_lang_sequence::{Definition, Guard, Len, Machine, Program, Term};
use cq_syntax::symbol_name;
use syn::Ident;

use crate::Advice;

pub(crate) fn boundaries(
    program: &Program,
    machine: &Machine,
    facts: &cq_lang_sequence::Analysis,
    catalog: &cq_lang_sequence::Catalog,
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
        references(&definition.term, definitions, &indices, &mut refs);
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
        if string_row(facts, result) {
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
        if !facts.structures.filtered.contains_key(result)
            && !physical.operator.operands().iter().any(|input| {
                program
                    .structures
                    .physical
                    .program
                    .definitions
                    .iter()
                    .any(|source| {
                        source.name == **input && matches!(source.operator, Operator::Guard(_))
                    })
            })
        {
            continue;
        }
        if !host_feeds.contains(&symbol_name(&definition.name))
            || !reads_at(&definition.term, Some(result), facts, catalog, &host, &host)
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
    source: &Program,
    catalog: &cq_lang_sequence::Catalog,
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
    let recorded: BTreeMap<_, _> = program
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
    let authored: BTreeSet<_> = source
        .graded
        .iter()
        .flat_map(|graded| &graded.grades)
        .map(|grade| symbol_name(&grade.name))
        .collect();
    let opaque = host_operations(program, facts);
    let row_predicates: BTreeSet<_> = program
        .structures
        .physical
        .program
        .definitions
        .iter()
        .filter(|definition| row_predicate(&definition.operator))
        .map(|definition| symbol_name(&definition.name))
        .collect();
    let mut stores = BTreeSet::new();
    let mut levels: Vec<Option<Vec<Ident>>> = definitions
        .iter()
        .map(|d| recorded.get(&symbol_name(&d.name)).cloned())
        .collect();
    let mut advice = Vec::new();
    for (i, d) in definitions.iter().enumerate() {
        let result = facts.results.get(&symbol_name(&d.name)).map(String::as_str);
        let host_reads = reads_at(&d.term, result, facts, catalog, &host, &host);
        let descendant_reads = reads_at(&d.term, result, facts, catalog, &producer, &publication)
            || reads_at(&d.term, result, facts, catalog, &publication, &publication);
        let at_host = matches!(d.term, Term::Rows(_))
            || requires_host(&d.term, &opaque)
            || (host_reads && !descendant_reads);
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
        references(&definitions[i].term, definitions, &indices, &mut refs);
        for name in refs {
            if let Some(&source) = indices.get(&symbol_name(name))
                && !recorded.contains_key(&symbol_name(name))
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
            let preferred = if stored {
                if i + 1 == definitions.len()
                    || has_row_work(
                        &d.term,
                        definitions,
                        &indices,
                        &row_predicates,
                        &mut BTreeSet::new(),
                    )
                {
                    publication.clone()
                } else {
                    host.clone()
                }
            } else {
                producer.clone()
            };
            let result = facts.results.get(&symbol_name(&d.name)).map(String::as_str);
            let selected = [&preferred, &publication, &producer, &host]
                .into_iter()
                .position(|level| {
                    let read_level = if stored && level == &publication && publication != host {
                        &producer
                    } else {
                        level
                    };
                    reads_at(&d.term, result, facts, catalog, read_level, level)
                });
            // Keep an impossible candidate visible to the catalog checker.
            levels[i] = Some(match selected {
                Some(1) => publication.clone(),
                Some(2) => producer.clone(),
                Some(3) => host.clone(),
                _ => preferred,
            });
        }
        let level = levels[i].as_ref().unwrap().clone();
        let read_level = if stored && level == publication && publication != host {
            producer.clone()
        } else {
            level
        };
        let mut refs = Vec::new();
        references(&d.term, definitions, &indices, &mut refs);
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
    // BE-Scope: a retained descendant result still cannot be read directly by
    // a host operation. Publish its completed producer at host, rather than
    // pulling the lookup pipeline (or an opaque consumer) across its boundary.
    // A bare host `store producer` already expresses that transfer.
    let mut names: BTreeSet<_> = indices.keys().cloned().collect();
    let mut host_publications = BTreeMap::new();
    for (i, definition) in definitions.iter().enumerate() {
        if levels[i].as_ref() != Some(&host)
            || matches!(&definition.term, Term::Store { inner, .. } if matches!(inner.as_ref(), Term::Ref(_)))
        {
            continue;
        }
        let mut refs = Vec::new();
        references(&definition.term, definitions, &indices, &mut refs);
        for name in refs {
            let source = indices[&symbol_name(name)];
            if levels[source].as_ref() == Some(&host)
                || authored.contains(&symbol_name(name))
                || !matches!(definitions[source].term, Term::Store { .. })
                || host_publications.contains_key(&source)
            {
                continue;
            }
            let mut fresh = format!("__sequence_publish_{}", symbol_name(name));
            while !names.insert(fresh.clone()) {
                fresh.push('_');
            }
            let published_producer = Ident::new(&fresh, name.span());
            let level = levels[source].as_ref().unwrap();
            let producer_level = if *level == publication {
                producer.clone()
            } else {
                level.clone()
            };
            host_publications.insert(source, producer_level.clone());
            advice.push(Advice::Publish {
                name: name.clone(),
                producer: published_producer.clone(),
            });
            advice.push(Advice::Grade {
                name: published_producer,
                level: producer_level,
                machine: machine.name.clone(),
            });
            levels[source] = Some(host.clone());
        }
    }
    // A host view stays a view when its retained consumer is also at host.
    // Materialize only actual crossings into a descendant execution extent.
    for (i, definition) in definitions.iter().enumerate() {
        let level = levels[i].as_ref().unwrap();
        let stored = matches!(definition.term, Term::Store { .. }) || stores.contains(&i);
        let read_level = if let Some(producer_level) = host_publications.get(&i) {
            producer_level
        } else if stored && *level == publication && publication != host {
            &producer
        } else {
            level
        };
        let mut refs = Vec::new();
        references(&definition.term, definitions, &indices, &mut refs);
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
        if !authored.contains(&symbol_name(&d.name))
            && recorded.get(&symbol_name(&d.name)) != level.as_ref()
        {
            advice.push(Advice::Grade {
                name: d.name.clone(),
                level: level.unwrap_or_else(|| host.clone()),
                machine: machine.name.clone(),
            });
        }
    }
    advice
}

fn has_row_work(
    term: &Term,
    definitions: &[cq_lang_sequence::Definition],
    indices: &BTreeMap<String, usize>,
    row_predicates: &BTreeSet<String>,
    seen: &mut BTreeSet<usize>,
) -> bool {
    match term {
        Term::Lookup { .. } => true,
        Term::Ref(name) => indices.get(&symbol_name(name)).is_some_and(|&i| {
            seen.insert(i)
                && has_row_work(
                    &definitions[i].term,
                    definitions,
                    indices,
                    row_predicates,
                    seen,
                )
        }),
        Term::Rows(_) => false,
        Term::Map(name, t) | Term::Compact(name, t) => {
            row_predicates.contains(&symbol_name(name))
                || has_row_work(t, definitions, indices, row_predicates, seen)
        }
        Term::Flatten(t)
        | Term::Fold(_, t)
        | Term::Group(_, t)
        | Term::Canon(t)
        | Term::Tile { inner: t, .. }
        | Term::Store { inner: t, .. } => {
            has_row_work(t, definitions, indices, row_predicates, seen)
        }
        Term::Union(terms) => terms
            .iter()
            .any(|t| has_row_work(t, definitions, indices, row_predicates, seen)),
        Term::Choose(arms) => arms
            .iter()
            .any(|(_, t)| has_row_work(t, definitions, indices, row_predicates, seen)),
    }
}

fn requires_host(term: &Term, opaque: &BTreeSet<String>) -> bool {
    match term {
        Term::Group(_, _) => true,
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

// Provider levels describe where a read executes, independently of where its
// index was constructed. This also keeps authored grades untouched: impossible
// authored reads are rejected by the final catalog check, not silently moved.
fn reads_at(
    term: &Term,
    result: Option<&str>,
    facts: &cq_lang_sequence::Analysis,
    catalog: &cq_lang_sequence::Catalog,
    level: &[Ident],
    canon_level: &[Ident],
) -> bool {
    use cq_lang_sequence::Obligation;
    let at = |registered: &[String], level: &[Ident]| {
        registered.len() == level.len()
            && registered
                .iter()
                .zip(level)
                .all(|(name, ident)| ident == name.as_str())
    };
    match term {
        Term::Lookup {
            provider, inner, ..
        } => {
            let provider = symbol_name(provider);
            facts.obligations.iter().all(|obligation| {
                let Obligation::Provider {
                    provider: name,
                    capability,
                    ty,
                    ..
                } = obligation
                else {
                    return true;
                };
                if *name != provider {
                    return true;
                }
                catalog.providers.iter().any(|token| {
                    crate::catalog::matches_type(&token.ty, ty)
                        && token.capabilities.contains(capability)
                        && token.levels.iter().any(|registered| at(registered, level))
                        && token.elements.as_ref().is_none_or(|accepted| {
                            facts.structures.layouts[&provider].iter().all(|column| {
                                accepted.iter().any(|ty| {
                                    crate::catalog::matches_type(
                                        ty,
                                        &facts.structures.physical.rows[&provider].types[column],
                                    )
                                })
                            })
                        })
                })
            }) && reads_at(inner, result, facts, catalog, level, canon_level)
        }
        Term::Canon(inner) => {
            facts.obligations.iter().all(|obligation| {
                let Obligation::Canon { result: name, ty } = obligation else {
                    return true;
                };
                result != Some(name.as_str())
                    || catalog.canons.iter().any(|token| {
                        ty.as_ref()
                            .is_none_or(|ty| crate::catalog::matches_type(&token.ty, ty))
                            && token
                                .levels
                                .iter()
                                .any(|registered| at(registered, canon_level))
                    })
            }) && reads_at(inner, result, facts, catalog, level, canon_level)
        }
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Store { inner, .. }
        | Term::Tile { inner, .. } => reads_at(inner, result, facts, catalog, level, canon_level),
        Term::Union(arms) => arms
            .iter()
            .all(|term| reads_at(term, result, facts, catalog, level, canon_level)),
        Term::Choose(arms) => arms
            .iter()
            .all(|(_, term)| reads_at(term, result, facts, catalog, level, canon_level)),
        Term::Ref(_) | Term::Rows(_) => true,
    }
}

fn references<'a>(
    term: &'a Term,
    definitions: &'a [Definition],
    indices: &BTreeMap<String, usize>,
    result: &mut Vec<&'a Ident>,
) {
    match term {
        Term::Ref(name) => result.push(name),
        Term::Rows(_) => {}
        Term::Map(_, t)
        | Term::Compact(_, t)
        | Term::Flatten(t)
        | Term::Fold(_, t)
        | Term::Group(_, t)
        | Term::Canon(t)
        | Term::Tile { inner: t, .. }
        | Term::Store { inner: t, .. }
        | Term::Lookup { inner: t, .. } => references(t, definitions, indices, result),
        Term::Union(terms) => {
            for t in terms {
                references(t, definitions, indices, result);
            }
        }
        Term::Choose(arms) => {
            for (guard, t) in arms {
                // BE-Choose reads a published count at the choice's own grade.
                // It is a scope dependency even when neither arm reads its rows.
                if let Guard::Lt(length, _) | Guard::Ge(length, _) = guard {
                    length_references(length, definitions, indices, result);
                }
                references(t, definitions, indices, result);
            }
        }
    }
}

fn length_references<'a>(
    length: &Len,
    definitions: &'a [Definition],
    indices: &BTreeMap<String, usize>,
    result: &mut Vec<&'a Ident>,
) {
    match length {
        Len::Input(name) | Len::Scan(name) => {
            if let Some(&index) = indices.get(name) {
                result.push(&definitions[index].name);
            }
        }
        Len::CeilDiv(inner, _) => length_references(inner, definitions, indices, result),
        Len::Add(left, right) => {
            length_references(left, definitions, indices, result);
            length_references(right, definitions, indices, result);
        }
        Len::Lit(_) | Len::Unknown(_) => {}
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
        | Term::Group(_, inner)
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

// Row-to-row predicates require a producer over actual rows, not a host
// descriptor whose interval bounds are constants or host parameters.
fn row_predicate(operator: &Operator) -> bool {
    use cq_syntax::EqualityValue;
    use cq_syntax::constraint::{Constraint, Term as Scalar};

    fn constraint(predicate: &Constraint) -> bool {
        match predicate {
            Constraint::Compare { left, right, .. } => {
                matches!(left, Scalar::Variable(_)) && matches!(right, Scalar::Variable(_))
            }
            Constraint::Between { term, low, high } => {
                matches!(term, Scalar::Variable(_))
                    && (matches!(low, Scalar::Variable(_)) || matches!(high, Scalar::Variable(_)))
            }
            Constraint::Or(arms) => arms.iter().flatten().any(constraint),
            Constraint::In { .. } => false,
        }
    }

    match operator {
        Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
            select
                .condition
                .equalities
                .iter()
                .any(|equality| matches!(equality.right, EqualityValue::Variable(_)))
                || select.condition.constraints.iter().any(constraint)
        }
        _ => false,
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
                || match &definition.operator {
                    // Membership consumes only right-hand keys, not its opaque
                    // payload. Projected integer indexes may therefore execute
                    // below host even when their original relation has strings.
                    Operator::Semi(_, keyed) | Operator::Anti(_, keyed) => {
                        string_row(facts, &symbol_name(&keyed.left))
                            || keyed.columns.iter().any(|column| {
                                string_type(
                                    &facts.structures.physical.rows[&symbol_name(&keyed.right)]
                                        .types[&symbol_name(column)],
                                )
                            })
                    }
                    operator => operator
                        .operands()
                        .iter()
                        .any(|input| string_row(facts, &symbol_name(input))),
                }
        })
        .map(|definition| symbol_name(&definition.name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cq_lang_sequence::{Capability, Catalog, LevelNode, ProviderToken};

    fn machine() -> Machine {
        fn node(name: &str, children: Vec<LevelNode>) -> LevelNode {
            LevelNode {
                name: syn::parse_str(name).unwrap(),
                collaborative: name == "block",
                dynamic: name == "device",
                width: (name == "block").then_some(32),
                shared_bytes: None,
                children,
            }
        }
        Machine {
            name: syn::parse_quote!(gpu),
            items: 1,
            layout: None,
            root: node("host", vec![node("device", vec![node("block", vec![])])]),
        }
    }

    fn membership() -> Program {
        let structures = syn::parse_quote! {
            physical {
                relation L(a:i32); relation R(a:i32,s:String);
                l = pipe rename L@0 {a -> a};
                r = let rename R@1 {a -> a,s -> s};
                answer = let canon semi eager [a] l,r;
                return answer as answer(a);
            }
            structures {
                l host scan = Reader []; r host scan = Reader [];
                r = Index [a]; answer rows = ::std::vec::Vec<_>; answer canon = Canon;
            }
        };
        cq_sequence_lowering::compile(&structures).unwrap()
    }

    fn catalog(levels: Vec<Vec<String>>) -> Catalog {
        Catalog {
            providers: vec![ProviderToken {
                ty: syn::parse_quote!(Index),
                capabilities: vec![Capability::Exact, Capability::Membership],
                levels,
                elements: None,
            }],
            canons: vec![cq_lang_sequence::CanonToken {
                ty: syn::parse_quote!(Canon),
                levels: vec![
                    vec!["host".into()],
                    vec!["host".into(), "device".into()],
                    vec!["host".into(), "device".into(), "block".into()],
                ],
                max_packed_bits: None,
            }],
            codecs: Vec::new(),
        }
    }

    #[test]
    fn membership_uses_keys_not_opaque_right_payload_for_host_placement() {
        let source = membership();
        let facts = cq_lang_sequence::contract::check(&source).unwrap();
        let target = machine();
        let available = catalog(vec![
            vec!["host".into(), "device".into()],
            vec!["host".into(), "device".into(), "block".into()],
        ]);
        assert!(!host_operations(&source, &facts).contains("answer"));
        assert!(host_operations(&source, &facts).contains("r"));
        assert!(
            !boundaries(&source, &target, &facts, &available)
                .iter()
                .any(|advice| matches!(advice, Advice::Publish { name, .. } if name == "answer"))
        );
        let advice = advise(&source, &target, true, &facts, &source, &available);
        assert!(advice.iter().any(|advice| matches!(advice,
            Advice::Grade { name, level, .. }
                if name == "answer" && level.iter().map(symbol_name).collect::<Vec<_>>()
                    == ["host", "device"])));
    }

    #[test]
    fn host_only_provider_reads_keep_integer_membership_at_host() {
        let source = membership();
        let facts = cq_lang_sequence::contract::check(&source).unwrap();
        let advice = advise(
            &source,
            &machine(),
            true,
            &facts,
            &source,
            &catalog(vec![vec!["host".into()]]),
        );
        assert!(advice.iter().any(|advice| matches!(advice,
            Advice::Grade { name, level, .. }
                if name == "answer" && level.iter().map(symbol_name).collect::<Vec<_>>()
                    == ["host"])));
    }

    #[test]
    fn unsupported_authored_provider_read_grade_is_not_replaced() {
        let source =
            cq_sequence_rewrite_passes::grade(&membership(), quote::quote!(gpu, answer at host))
                .unwrap();
        let facts = cq_lang_sequence::contract::check(&source).unwrap();
        let available = catalog(vec![
            vec!["host".into(), "device".into()],
            vec!["host".into(), "device".into(), "block".into()],
        ]);
        let advice = advise(&source, &machine(), true, &facts, &source, &available);
        assert!(
            !advice
                .iter()
                .any(|advice| matches!(advice, Advice::Grade { name, .. } if name == "answer"))
        );
        assert!(!reads_at(
            &source
                .sequence
                .definitions
                .iter()
                .find(|definition| definition.name == "answer")
                .unwrap()
                .term,
            Some("answer"),
            &facts,
            &available,
            &[syn::parse_quote!(host)],
            &[syn::parse_quote!(host)],
        ));
    }
}
