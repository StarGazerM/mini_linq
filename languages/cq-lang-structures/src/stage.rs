//! Batch boundaries certify supported adapters from an indexable producer.
use super::*;

fn supported(operator: &Operator) -> bool {
    // ST-Total: The chunk adapter supports relational operators. Rust is total
    // too, but its adapters and branch scopes do not implement this batch protocol.
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

fn membership_step(operator: &Operator, facts: &Analysis) -> bool {
    let Operator::Probe(_, probe) = operator else {
        return false;
    };
    let key = (symbol_name(&probe.right), names(&probe.columns));
    facts.membership.contains(&key)
        && if let Some(filtered) = facts.filtered.get(&key.0) {
            matches!(probe.choice, Some(physical::Choice::First(_)))
                && filtered
                    .payload
                    .columns
                    .iter()
                    .all(|column| key.1.contains(&symbol_name(column)))
        } else {
            // STR-FullKey certificates make even an each-probe zero-or-one:
            // its complete set row is already present in the supplied key.
            facts.physical.headings[&key.0] == key.1
        }
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
    // ST-Expand: The selected provider declares finite, exact-size retained
    // cursors. This law does not confer ordering or contiguous range placement.
    let candidates = physical::candidate_prefixes(&program.physical, &facts.physical);
    let ordered = &program.physical.program.definitions;
    let smallest_forces: BTreeSet<_> = ordered
        .iter()
        .enumerate()
        .flat_map(|(position, first)| {
            let count = candidates
                .iter()
                .find(|candidate| candidate.reader == &first.name)
                .map_or(1, |candidate| candidate.sides.len());
            ordered[position + 1..position + count]
                .iter()
                .map(|definition| symbol_name(&definition.name))
        })
        .collect();
    for name in &expands {
        let definition = definitions[name];
        let Operator::Probe(_, probe) = &definition.operator else {
            return error(definition, "expand requires a probe");
        };
        if smallest_forces.contains(name) {
            return error(
                definition,
                "expand boundary must be on the introducing Smallest probe, not its forcing read",
            );
        }
        if matches!(probe.choice, Some(physical::Choice::Smallest(_))) {
            let range = candidates
                .iter()
                .find(|candidate| candidate.reader == &definition.name)
                .is_some_and(|candidate| {
                    candidate.sides.iter().all(|(side, prefix)| {
                        facts.candidate_support(
                            side,
                            prefix,
                            &candidate.column,
                            ProviderCapability::Expand,
                        )
                    })
                });
            if !range {
                return error(
                    definition,
                    "expand smallest requires declared expand support on every selected two-level candidate build",
                );
            }
        } else {
            let keys = names(&probe.columns);
            let side = symbol_name(&probe.right);
            let built_range = facts
                .selected_built(&side, &keys)
                .is_some_and(|((levels, _), depth)| depth == levels.len())
                && facts.provider_support(&side, &keys, None, ProviderCapability::Expand);
            let resident_range = facts.host_read(&side)
                && facts.provider_support(&side, &keys, Some(name), ProviderCapability::Expand);
            if !built_range && !resident_range {
                return error(
                    definition,
                    "expand requires declared expand support on its selected exact provider",
                );
            }
        }
    }
    // ST-Stage: A positive, unique boundary ends a supported adapter segment,
    // starting at a for producer, never an unrecorded probe/merge cursor.
    // Certified full-key membership probes are zero-or-one lane-local steps:
    // their only non-key observations are recorded, proven constants. They
    // need no range cursor and assert nothing about source-key uniqueness.
    // A Smallest force belongs to its introducing probe's exact intersection,
    // not a second cursor. Later stages may cross it, but cannot expand it alone.
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
            if !supported(&definition.operator) {
                return error(definition, "stage segment contains an unsupported adapter");
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
                && !membership_step(&definition.operator, facts)
                && !smallest_forces.contains(&current)
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
