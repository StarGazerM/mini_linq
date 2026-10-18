//! Batch boundaries certify total lookahead from an indexable producer.
use super::*;

fn total(operator: &Operator) -> bool {
    // ST-Total: Relational constraints, including Constraint::Total, are total;
    // opaque Rust binds and guards and branch scopes are not stage steps.
    matches!(
        operator,
        Operator::Rename(_)
            | Operator::Project(_)
            | Operator::Select(_)
            | Operator::Point(_, _)
            | Operator::Range(_, _)
            | Operator::Probe(_, _)
            | Operator::Semi(_, _)
            | Operator::Anti(_, _)
            | Operator::Merge(_, _)
            | Operator::Unit(_)
    )
}

pub(crate) fn check(program: &Program, facts: &Analysis) -> syn::Result<BTreeMap<String, usize>> {
    let mut stages = BTreeMap::new();
    let mut expands = BTreeSet::new();
    for entry in &program.structures.entries {
        let Structure::Stage { size, expand, .. } = &entry.structure else {
            continue;
        };
        let size = size.base10_parse::<usize>()?;
        if size == 0 {
            return error(entry, "stage batch size must be positive");
        }
        if stages.insert(symbol_name(&entry.name), size).is_some() {
            return error(entry, "duplicate stage decision");
        }
        if expand.is_some() {
            expands.insert(symbol_name(&entry.name));
        }
    }
    let definitions: BTreeMap<_, _> = program
        .physical
        .program
        .definitions
        .iter()
        .map(|definition| (symbol_name(&definition.name), definition))
        .collect();
    let scoped: BTreeSet<_> = facts
        .physical
        .fused
        .values()
        .flat_map(|group| group.members.iter())
        .collect();
    // ST-Expand: SortedRun lookups expose total length-known ranges, so an
    // expanded probe is a segmented for producer, not a partial cursor.
    for name in &expands {
        let definition = definitions[name];
        let Operator::Probe(_, probe) = &definition.operator else {
            return error(definition, "expand requires a probe");
        };
        let keys = names(&probe.columns);
        let range = facts.built.get(&symbol_name(&probe.right)).into_iter().flatten()
            .any(|(levels, ty)| levels.len() == 1 && levels[0] == keys
                && matches!(ty, syn::Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "SortedRun")));
        if !range {
            return error(definition, "expand requires a SortedRun range-cursor build");
        }
    }
    // ST-Stage: A positive, unique boundary ends a total segment before any
    // partial step, starting at a for producer, never a probe/merge cursor.
    for boundary in stages.keys() {
        let mut current = boundary.clone();
        loop {
            let definition = definitions[&current];
            if scoped.contains(&current) {
                return error(definition, "stage cannot cross a fused branch scope");
            }
            // An earlier retained collection is already evaluated; its build is
            // not part of the pipe being staged. Earlier stage boundaries have
            // their own independently checked producer and total segment.
            if current != *boundary
                && (facts.retained.contains_key(&current) || stages.contains_key(&current))
            {
                break;
            }
            if !total(&definition.operator) {
                return error(
                    definition,
                    "stage segment contains a partial or unsupported step",
                );
            }
            if matches!(definition.operator, Operator::Rename(_))
                || facts.reads.contains_key(&current)
            {
                break;
            }
            if matches!(
                definition.operator,
                Operator::Probe(_, _) | Operator::Merge(_, _)
            ) && !expands.contains(&current)
            {
                return error(
                    definition,
                    "stage requires a for producer, not a probe or merge cursor",
                );
            }
            let Some(input) = definition.operator.input() else {
                return error(
                    definition,
                    "stage requires a host scan, retained rows, or positional read producer",
                );
            };
            current = symbol_name(input);
            if facts.physical.modes[&current] != ModeKind::Pipe
                && !facts.retained.contains_key(&current)
                && !facts.reads.contains_key(&current)
            {
                return error(
                    definition,
                    "stage producer requires recorded retained rows or a positional read",
                );
            }
        }
    }
    Ok(stages)
}
