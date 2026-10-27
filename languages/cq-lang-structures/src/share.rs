//! By-need sharing of a single immutable payload lookup.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Share {
    pub build: String,
    pub keys: Vec<String>,
    pub first: String,
    pub force: String,
}

pub(crate) fn forwarding(operator: &Operator) -> bool {
    matches!(
        operator,
        Operator::Rename(_)
            | Operator::Project(_)
            | Operator::Select(_)
            | Operator::Point(_, _)
            | Operator::Range(_, _)
            | Operator::Semi(_, _)
            | Operator::Anti(_, _)
    )
}

/// An explicitly retained immutable host read. Both demands must record the
/// same provider and level layout over the same host occurrence.
pub fn retained_host_read<'a>(
    program: &'a Program,
    build: &str,
    reader: &str,
    keys: &[String],
) -> Option<&'a syn::Type> {
    retained_host_keyed(program, build, reader, keys).map(|keyed| &keyed.ty)
}

fn retained_host_keyed<'a>(
    program: &'a Program,
    build: &str,
    reader: &str,
    keys: &[String],
) -> Option<&'a Keyed> {
    if keys.is_empty() {
        return None;
    }
    let mut wanted = keys.to_vec();
    wanted.sort();
    if wanted.windows(2).any(|pair| pair[0] == pair[1]) {
        return None;
    }
    program.structures.entries.iter().find_map(|entry| {
        let Structure::Host { keyed, reader: demand, .. } = &entry.structure else {
            return None;
        };
        let mut recorded = keyed.columns().map(symbol_name).collect::<Vec<_>>();
        recorded.sort();
        (symbol_name(&entry.name) == build
            && symbol_name(demand) == reader
            && recorded == wanted
            && keyed.levels.iter().all(|level| !level.columns.is_empty())
            && keyed.supports(ProviderCapability::Retain))
            .then_some(keyed.as_ref())
    })
}

pub(crate) fn check(program: &Program, facts: &Analysis) -> syn::Result<BTreeMap<String, Share>> {
    let definitions = &program.physical.program.definitions;
    let candidates = physical::candidate_prefixes(&program.physical, &facts.physical);
    let mut result = BTreeMap::new();
    // ST-Share: Immutable payload lookups preserve their keys until the force.
    // A recorded Smallest probe can force either candidate's directory prefix.
    for entry in &program.structures.entries {
        let Structure::Share {
            build,
            keys,
            first,
            force,
            ..
        } = &entry.structure
        else {
            continue;
        };
        let name = symbol_name(&entry.name);
        let share = Share {
            build: symbol_name(build),
            keys: names(&keys.columns),
            first: symbol_name(first),
            force: symbol_name(force),
        };
        let fail = || {
            syn::Error::new_spanned(
                entry,
                "share requires an earlier immutable payload build or matching retained snapshot hosts, eager semi and later same-key probe on its preserving chain",
            )
        };
        let position = |name: &str| {
            definitions
                .iter()
                .position(|d| symbol_name(&d.name) == name)
        };
        let (Some(b), Some(s), Some(p)) = (
            position(&share.build),
            position(&share.first),
            position(&share.force),
        ) else {
            return Err(fail());
        };
        let retained_host = retained_host_keyed(program, &share.build, &share.first, &share.keys)
            .zip(retained_host_keyed(program, &share.build, &share.force, &share.keys))
            .is_some_and(|(first, force)| {
                first.ty == force.ty && first.level_names() == force.level_names()
            })
            && facts.physical.hosts.contains_key(&share.build)
            && matches!(definitions[b].operator, Operator::Rename(_));
        if name != share.first
            || !(b < s && s < p)
            || (facts.physical.modes[&share.build] != ModeKind::Let && !retained_host)
        {
            return Err(fail());
        }
        let candidate = candidates.iter().find(|candidate| {
            symbol_name(candidate.reader) == share.force
                && candidate
                    .sides
                    .iter()
                    .any(|(build, prefix)| build == &share.build && prefix == &share.keys)
        });
        let payload = if let Some(candidate) = candidate {
            facts.candidate_on(&share.build, &share.keys, &candidate.column).is_some()
        } else {
            facts.selected_built(&share.build, &share.keys)
                .is_some_and(|((levels, _), depth)| depth == levels.len())
        };
        if !(payload || retained_host)
            || facts
                .membership
                .contains(&(share.build.clone(), share.keys.clone()))
        {
            return Err(fail());
        }
        let Operator::Semi(_, semi) = &definitions[s].operator else {
            return Err(fail());
        };
        let Operator::Probe(_, probe) = &definitions[p].operator else {
            return Err(fail());
        };
        if matches!(semi.choice, Some(physical::Choice::Lazy(_)))
            || symbol_name(&semi.right) != share.build
            || names(&semi.columns) != share.keys
            || (candidate.is_none()
                && (symbol_name(&probe.right) != share.build
                    || names(&probe.columns) != share.keys))
        {
            return Err(fail());
        }
        let mut current = symbol_name(&probe.left);
        while current != share.first {
            let Some(index) = position(&current) else {
                return Err(fail());
            };
            let operator = &definitions[index].operator;
            if !forwarding(operator)
                || share
                    .keys
                    .iter()
                    .any(|key| !facts.physical.headings[&current].contains(key))
            {
                return Err(fail());
            }
            let Some(input) = operator.input() else {
                return Err(fail());
            };
            current = symbol_name(input);
        }
        if result.insert(name, share).is_some() {
            return error(entry, "duplicate share decision");
        }
    }
    Ok(result)
}
