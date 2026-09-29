//! Physical preservation and storage rule derivation independent of selection.

use cq_lang_physical::{self as physical, ModeKind, Operator, policy::Policy, syntax::symbol_name};
use cq_lang_structures::{self as structures, Structure};
use std::collections::{BTreeMap, BTreeSet};

type Key = BTreeSet<String>;
// Strength of a read premise: membership < payload lookup < ordered lookup.
const MEMBER: u8 = 0;
const PAYLOAD: u8 = 1;
const ORDERED: u8 = 2;
type Demands = BTreeMap<String, BTreeMap<Key, u8>>;

/// Check unchanged Physical, all read premises, exact host facts, and built
/// advice/defaults. Entry order is not a storage rule.
pub fn check(
    source: &physical::Program,
    target: &structures::Program,
    policy: &Policy,
) -> syn::Result<()> {
    physical::contract::check(source)?;
    let info = structures::contract::check(target)?;
    if target.physical != *source {
        let construct = source
            .program
            .definitions
            .iter()
            .zip(&target.physical.program.definitions)
            .find(|(a, b)| a != b)
            .map(|(a, _)| &a.name)
            .unwrap_or(&source.program.returned);
        return Err(syn::Error::new_spanned(
            construct,
            format!("result `{construct}`: Structures must preserve the source Physical program"),
        ));
    }
    let mut configured = BTreeSet::new();
    for host in &policy.hosts {
        let name = symbol_name(&host.name);
        let fail = |message: &str| {
            syn::Error::new_spanned(&host.name, format!("relation `{name}`: {message}"))
        };
        let Some(actual) = info.host_keys.get(&name) else {
            return Err(fail("extra policy host"));
        };
        if !configured.insert(name.clone()) {
            return Err(fail("duplicate policy host"));
        }
        let expected = host.key.as_ref().map(|k| structures::KeyFact {
            columns: k
                .columns
                .iter()
                .map(symbol_name)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            ordered: k.ordered.is_some(),
        });
        if let Some(k) = &host.key {
            if k.columns.len() != expected.as_ref().unwrap().columns.len() {
                return Err(fail("duplicate policy key column"));
            }
        }
        // Host key tuple order is part of the representation fact. The
        // language analysis sorts keys for read coverage, so also compare
        // the written columns used by realization to construct host keys.
        let written = target
            .structures
            .entries
            .iter()
            .find_map(|entry| match &entry.structure {
                Structure::Host(h) if symbol_name(&entry.name) == name => Some(
                    h.key
                        .as_ref()
                        .map(|k| k.columns.iter().map(symbol_name).collect::<Vec<_>>()),
                ),
                _ => None,
            });
        let requested = host
            .key
            .as_ref()
            .map(|k| k.columns.iter().map(symbol_name).collect::<Vec<_>>());
        if *actual != expected
            || written != Some(requested)
            || info.host_canon.contains(&name) != host.canon.is_some()
        {
            return Err(fail(
                "Structures host facts differ from policy (key, order, or canon)",
            ));
        }
    }
    for name in info.host_keys.keys() {
        if !configured.contains(name) {
            return Err(syn::Error::new_spanned(
                policy,
                format!("relation `{name}`: missing policy host"),
            ));
        }
    }
    let facts = &info.physical;
    let definitions: BTreeMap<_, _> = source
        .program
        .definitions
        .iter()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    let mut advice = BTreeMap::new();
    for input in source.inputs() {
        if let Some(attr) = input.index_advice()? {
            advice.insert(symbol_name(&input.name), attr.parse_args::<syn::Type>()?);
        }
    }
    let advised = |name: &str| facts.hosts.get(name).and_then(|h| advice.get(h));
    let mut demands = Demands::new();
    for (name, readers) in &facts.key_reads {
        for (reader, columns) in readers {
            if info.host_key_for(name, columns).is_none() {
                let strength = if matches!(definitions[reader].operator, Operator::Semi(..)) {
                    MEMBER
                } else {
                    PAYLOAD
                };
                demand(&mut demands, name, columns, strength);
            }
        }
    }
    for (name, readers) in &facts.range_reads {
        for (_, column) in readers {
            let columns = std::slice::from_ref(column);
            if !info.host_key_for(name, columns).is_some_and(|k| k.ordered) {
                demand(&mut demands, name, columns, ORDERED);
            }
        }
    }
    for (name, readers) in &facts.merge_reads {
        for (_, columns) in readers {
            let stream = facts.modes[name] == ModeKind::Pipe
                && matches!(&definitions[name].operator,
                Operator::Merge(_, k) if k.columns.iter().map(symbol_name).collect::<Key>() == columns.iter().cloned().collect());
            if !stream && !info.host_key_for(name, columns).is_some_and(|k| k.ordered) {
                demand(&mut demands, name, columns, ORDERED);
            }
        }
    }
    let mut intersections: BTreeMap<String, Key> = BTreeMap::new();
    if policy.smallest_intersections() {
        // Independently derive the local intersection rule: a single new
        // column, immediately checked, completing both materialized headings.
        for pair in source.program.definitions.windows(2) {
            let (Operator::Probe(_, a), Operator::Probe(_, b) | Operator::Semi(_, b)) =
                (&pair[0].operator, &pair[1].operator)
            else {
                continue;
            };
            if facts.modes[&symbol_name(&pair[0].name)] != ModeKind::Pipe
                || pair[0].canon.is_some()
                || b.left != pair[0].name
            {
                continue;
            }
            let left: Key = facts.headings[&symbol_name(&a.left)]
                .iter()
                .cloned()
                .collect();
            let ah: Key = facts.headings[&symbol_name(&a.right)]
                .iter()
                .cloned()
                .collect();
            let bh: Key = facts.headings[&symbol_name(&b.right)]
                .iter()
                .cloned()
                .collect();
            let added: Vec<_> = ah.difference(&left).cloned().collect();
            let [variable] = added.as_slice() else {
                continue;
            };
            let ak: Key = a.columns.iter().map(symbol_name).collect();
            let bk: Key = b.columns.iter().map(symbol_name).collect();
            if ak.is_empty()
                || bk.len() < 2
                || !bk.contains(variable)
                || ah.len() != ak.len() + 1
                || bh != bk
                || !bh.difference(&left).all(|c| c == variable)
            {
                continue;
            }
            let sides = [
                (symbol_name(&a.right), ak),
                (
                    symbol_name(&b.right),
                    bk.into_iter().filter(|c| c != variable).collect(),
                ),
            ];
            let eligible = sides.iter().all(|(name, prefix)| {
                let full: Key = facts.headings[name].iter().cloned().collect();
                facts.modes[name] == ModeKind::Let
                    && advised(name).is_none()
                    && facts.key_reads[name]
                        .iter()
                        .all(|(_, c)| info.host_key_for(name, c).is_none())
                    && demands
                        .get(name)
                        .into_iter()
                        .flat_map(|d| d.iter())
                        .all(|(k, n)| *n != ORDERED && (k == prefix || *k == full))
                    && intersections.get(name).is_none_or(|old| old == prefix)
            });
            if eligible {
                for (name, prefix) in sides {
                    demand(
                        &mut demands,
                        &name,
                        &prefix.iter().cloned().collect::<Vec<_>>(),
                        MEMBER,
                    );
                    demand(&mut demands, &name, &facts.headings[&name], MEMBER);
                    intersections.insert(name, prefix);
                }
            }
        }
    }
    // Decide whether the read premises mandate a chain, as a relation on
    // sets: strict inclusion, only membership before the maximal key, and
    // no ordered reads. This does not call the selector's chain builder.
    let mut chains: BTreeMap<String, Vec<Key>> = BTreeMap::new();
    for (name, reads) in &demands {
        if advised(name).is_some() {
            continue;
        }
        if let Some(prefix) = intersections.get(name) {
            chains.insert(
                name.clone(),
                vec![
                    prefix.clone(),
                    facts.headings[name].iter().cloned().collect(),
                ],
            );
            continue;
        }
        let mut keys: Vec<_> = reads.keys().cloned().collect();
        keys.sort_by_key(Key::len);
        if keys.len() > 1
            && keys
                .windows(2)
                .all(|p| p[0] != p[1] && p[0].is_subset(&p[1]) && reads[&p[0]] == MEMBER)
            && reads[keys.last().unwrap()] != ORDERED
        {
            chains.insert(name.clone(), keys);
        }
    }
    let mut discharged: BTreeSet<(String, Key)> = BTreeSet::new();
    for entry in &target.structures.entries {
        let Structure::Built { keyed, .. } = &entry.structure else {
            continue;
        };
        let name = symbol_name(&entry.name);
        let fail = |message: &str| {
            syn::Error::new_spanned(&entry.name, format!("built result `{name}`: {message}"))
        };
        let levels = keyed.level_names();
        let mut union = Key::new();
        let prefixes: Vec<_> = levels
            .iter()
            .map(|level| {
                union.extend(level.iter().cloned());
                union.clone()
            })
            .collect();
        let Some(reads) = demands.get(&name) else {
            return Err(fail(
                "no read requires a build; a host key already answers it or it is unused",
            ));
        };
        for prefix in &prefixes {
            if !reads.contains_key(prefix) {
                return Err(fail(&format!(
                    "no unanswered read on {prefix:?}; do not build a read already answered by a host key"
                )));
            }
            if !discharged.insert((name.clone(), prefix.clone())) {
                return Err(fail(&format!("duplicate build for read on {prefix:?}")));
            }
        }
        let expected = if let Some(ty) = advised(&name) {
            if levels.len() != 1 {
                return Err(fail("index advice requires one exact-key entry per read"));
            }
            ty.clone()
        } else {
            if let Some(chain) = chains.get(&name) {
                if *chain != prefixes {
                    return Err(fail(
                        "chained reads require Trie levels at successive key differences",
                    ));
                }
            } else if levels.len() != 1 {
                return Err(fail("these reads do not admit chained Trie levels"));
            }
            let mut ty: syn::Type = match reads[prefixes.last().unwrap()] {
                MEMBER => syn::parse_quote!(::std::collections::HashSet<_>),
                PAYLOAD => syn::parse_quote!(::cq_storage::HashIndex<_, _>),
                _ => syn::parse_quote!(::std::collections::BTreeMap<_, ::std::vec::Vec<_>>),
            };
            for _ in 1..levels.len() {
                ty = syn::parse_quote!(::cq_storage::Trie<'_, _, #ty>);
            }
            ty
        };
        if keyed.ty != expected {
            return Err(fail(&format!(
                "type must respect {}: {}",
                if advised(&name).is_some() {
                    "#[index(T)] advice"
                } else {
                    "read-kind default"
                },
                quote::quote!(#expected)
            )));
        }
    }
    for (name, reads) in demands {
        for key in reads.keys() {
            if !discharged.contains(&(name.clone(), key.clone())) {
                return Err(syn::Error::new_spanned(
                    &definitions[&name].name,
                    format!("result `{name}`: missing selected structure for read on {key:?}"),
                ));
            }
        }
    }
    Ok(())
}

fn demand(demands: &mut Demands, result: &str, columns: &[String], strength: u8) {
    demands
        .entry(result.into())
        .or_default()
        .entry(columns.iter().cloned().collect())
        .and_modify(|n| *n = (*n).max(strength))
        .or_insert(strength);
}
