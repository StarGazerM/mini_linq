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

/// Check unchanged Physical, all read premises, built provider choices. Entry order is not a storage rule.
/// Formal rule CK-Structures: Check Physical preservation, independently derived storage rules.
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
    let facts = &info.physical;
    let definitions: BTreeMap<_, _> = source
        .program
        .definitions
        .iter()
        .map(|d| (symbol_name(&d.name), d))
        .collect();
    let mut demands = Demands::new();
    for (name, readers) in &facts.key_reads {
        for (reader, columns) in readers {
            if facts.modes[name] == ModeKind::Let {
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
            if facts.modes[name] == ModeKind::Let {
                demand(&mut demands, name, columns, ORDERED);
            }
        }
    }
    for (name, readers) in &facts.merge_reads {
        for (_, columns) in readers {
            let stream = facts.modes[name] == ModeKind::Pipe
                && matches!(&definitions[name].operator,
                Operator::Merge(_, k) if k.columns.iter().map(symbol_name).collect::<Key>() == columns.iter().cloned().collect());
            if !stream && facts.modes[name] == ModeKind::Let {
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
        let Structure::Built { keyed, .. } = &entry.structure;
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
                "no read requires a build; it is a host read or unused",
            ));
        };
        for prefix in &prefixes {
            if !reads.contains_key(prefix) {
                return Err(fail(&format!("no built read on {prefix:?}")));
            }
            if !discharged.insert((name.clone(), prefix.clone())) {
                return Err(fail(&format!("duplicate build for read on {prefix:?}")));
            }
        }
        let expected = {
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
                MEMBER => syn::parse_quote!(index_provider!(semi)),
                PAYLOAD => syn::parse_quote!(index_provider!(probe)),
                _ => syn::parse_quote!(index_provider!(ordered)),
            };
            for _ in 1..levels.len() {
                ty = syn::parse_quote!(index_provider!(trie #ty));
            }
            ty
        };
        if keyed.ty != expected {
            return Err(fail(&format!(
                "type must respect {}: {}",
                "read-kind provider slot",
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
