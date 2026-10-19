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
                "share requires an earlier let payload build, eager semi and later same-key probe on its preserving chain",
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
        if name != share.first
            || !(b < s && s < p)
            || facts.physical.modes[&share.build] != ModeKind::Let
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
        let payload = facts.built.get(&share.build).into_iter().flatten().any(|(levels, ty)| {
            let syn::Type::Path(path) = ty else { return false };
            let Some(segment) = path.path.segments.last() else { return false };
            if levels.len() == 1 && levels[0] == share.keys
                && (segment.ident == "HashIndex" || segment.ident == "SortedRun")
            {
                return true;
            }
            candidate.is_some_and(|candidate| {
                segment.ident == "PrefixRun"
                    && levels.len() == 2
                    && levels[0] == share.keys
                    && levels[1] == [candidate.column.clone()]
                    && share.keys.len() == 1
                    && matches!(
                        ungroup_type(&facts.physical.rows[&share.build].types[&share.keys[0]]),
                        syn::Type::Path(path) if path.qself.is_none()
                            && ["i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize"]
                                .iter().any(|name| path.path.is_ident(name))
                    )
            })
        });
        if !payload
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
