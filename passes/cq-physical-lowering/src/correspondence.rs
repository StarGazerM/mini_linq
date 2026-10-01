//! Logical erasure and policy obligations, independent of lowering decisions.

use cq_lang_ix as ix;
use cq_lang_physical::{self as physical, Operator, policy::Policy, syntax::symbol_name};
use std::collections::{BTreeMap, BTreeSet};

/// Check source/target contracts, logical erasure, IX demands, and policy.
/// Materialization is free whenever the language and policy premises hold.
/// Formal rule CK-Physical: Check local contracts, RA erasure, IX demands and independent policy premises.
pub fn check(source: &ix::Program, target: &physical::Program, policy: &Policy) -> syn::Result<()> {
    ix::contract::check(source)?;
    let facts = physical::contract::check(target)?;
    let erased = super::erase(target);
    if erased != source.ra {
        let construct = source
            .ra
            .plan
            .definitions
            .iter()
            .zip(&erased.plan.definitions)
            .find(|(a, b)| a != b)
            .map(|(a, _)| &a.result)
            .unwrap_or(&target.program.returned);
        return Err(syn::Error::new_spanned(
            construct,
            format!("result `{construct}`: physical erasure differs from the source RA program"),
        ));
    }
    super::check_demands(source, target)?;
    let mut keys: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for definition in &target.program.definitions {
        let name = symbol_name(&definition.name);
        let fail = |message: &str| {
            syn::Error::new_spanned(&definition.name, format!("definition `{name}`: {message}"))
        };
        let heading: BTreeSet<String> = facts.headings[&name].iter().cloned().collect();
        let inherited = match &definition.operator {
            Operator::Unit(_) => Some(BTreeSet::new()),
            Operator::Rename(_) => None,
            Operator::Probe(_, k) | Operator::Merge(_, k) => {
                let merge = policy.merge_joins() && !k.columns.is_empty();
                if merge != matches!(definition.operator, Operator::Merge(..)) {
                    return Err(fail(if merge {
                        "join policy requires merge"
                    } else {
                        "join policy requires probe (including Cartesian joins)"
                    }));
                }
                keys.get(&symbol_name(&k.left))
                    .zip(keys.get(&symbol_name(&k.right)))
                    .map(|(l, r)| l.union(r).cloned().collect())
            }
            Operator::Semi(_, k) => keys.get(&symbol_name(&k.left)).cloned(),
            Operator::Select(s) => {
                if s.condition.constant_keys().is_some() {
                    return Err(fail("literal selection requires a const point read"));
                }
                keys.get(&symbol_name(&s.input)).cloned()
            }
            Operator::Point(_, s) => keys.get(&symbol_name(&s.input)).cloned(),
            Operator::Bind(b) => keys.get(&symbol_name(&b.input)).cloned(),
            Operator::Guard(g) => keys.get(&symbol_name(&g.input)).cloned(),
            Operator::Project(p) => keys
                .get(&symbol_name(&p.input))
                .filter(|k| k.is_subset(&heading))
                .cloned(),
        };
        if definition.canon.is_some()
            && definition.name != target.program.returned
            && (!policy.canon_projections() || !matches!(definition.operator, Operator::Project(_)))
        {
            return Err(fail(
                "canon is allowed only at return or a project=canon projection",
            ));
        }
        if policy.canon_projections()
            && matches!(definition.operator, Operator::Project(_))
            && inherited.is_none()
            && definition.canon.is_none()
        {
            return Err(fail(
                "project=canon requires canon on a key-dropping projection",
            ));
        }
        if let Some(key) = inherited.or_else(|| definition.canon.is_some().then_some(heading)) {
            keys.insert(name, key);
        }
    }
    let returned = symbol_name(&target.program.returned);
    if !keys.contains_key(&returned) {
        return Err(syn::Error::new_spanned(
            &target.program.returned,
            format!("return `{returned}`: not canonical by flag or plan key inference"),
        ));
    }
    Ok(())
}
