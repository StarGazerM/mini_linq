//! Batch boundaries certify total lookahead from an indexable producer.
use super::*;

fn total(operator: &Operator) -> bool {
    // ST-Total: Relational constraints are total under the Eq/Ord premises;
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

fn membership_step(operator: &Operator, facts: &Analysis) -> bool {
    let Operator::Probe(_, probe) = operator else {
        return false;
    };
    let key = (symbol_name(&probe.right), names(&probe.columns));
    matches!(probe.choice, Some(physical::Choice::First(_)))
        && facts.membership.contains(&key)
        && facts.filtered.get(&key.0).is_some_and(|filtered| {
            filtered
                .payload
                .columns
                .iter()
                .all(|column| key.1.contains(&symbol_name(column)))
        })
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
    // ST-Expand: SortedRun builds, lexicographic PrefixRun prefixes, recorded
    // resident sorted hosts, and binary Smallest intersections expose exact
    // ranges whose total length is known before expansion. Leveled hosts
    // enumerate (run, range) pairs from one immutable snapshot.
    // HashedRun bucket is only a candidate superset, so it cannot prove this
    // obligation even though its bucket length is available.
    let candidates = physical::candidate_prefixes(&program.physical, &facts.physical);
    let smallest_forces: BTreeSet<_> = program
        .physical
        .program
        .definitions
        .windows(2)
        .filter(|pair| {
            candidates
                .iter()
                .any(|candidate| candidate.reader == &pair[0].name)
        })
        .map(|pair| symbol_name(&pair[1].name))
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
            let range = candidates.iter().find(|candidate| candidate.reader == &definition.name)
                .is_some_and(|candidate| candidate.sides.iter().all(|(side, prefix)| {
                    facts.built.get(side).into_iter().flatten().any(|(levels, ty)| {
                        levels.len() == 2 && levels[0] == *prefix
                            && levels[1].len() == 1 && levels[1][0] == candidate.column
                            && matches!(ty, syn::Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == "PrefixRun"))
                    })
                }));
            if !range {
                return error(
                    definition,
                    "expand smallest requires a PrefixRun candidate build on both sides",
                );
            }
        } else {
            let keys = names(&probe.columns);
            let built_range = facts.built.get(&symbol_name(&probe.right)).into_iter().flatten()
                .any(|(levels, ty)| matches!(ty, syn::Type::Path(path)
                    if path.path.segments.last().is_some_and(|segment|
                        (segment.ident == "SortedRun" && levels.len() == 1 && levels[0] == keys)
                            || (segment.ident == "PrefixRun" && prefix_depth(levels, &keys).is_some()))));
            let resident_range = program.structures.entries.iter().any(|entry| {
                let Structure::Host { reader, keyed, .. } = &entry.structure else {
                    return false;
                };
                if entry.name != probe.right || reader.result() != Some(&definition.name) {
                    return false;
                }
                let resident = matches!(&keyed.ty, syn::Type::Path(path)
                    if path.path.segments.last().is_some_and(|segment| segment.ident == "FrozenDeviceIndex" || segment.ident == "LeveledDeviceIndex")
                        && path.path.segments.iter().rev().nth(1).is_some_and(|segment| segment.ident == "cq_gpu_runtime"));
                let levels = keyed.level_names();
                let mut columns = levels.concat();
                columns.sort();
                resident && columns == keys && !columns.is_empty()
                    && levels.iter().all(|level| !level.is_empty())
            });
            if !built_range && !resident_range {
                return error(
                    definition,
                    "expand requires a SortedRun or PrefixRun range-cursor build or an exact resident sorted host range",
                );
            }
        }
    }
    // ST-Stage: A positive, unique boundary ends a total segment before any
    // partial step, starting at a for producer, never a probe/merge cursor.
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
