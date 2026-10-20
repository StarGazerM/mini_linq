//! Logical erasure and admissibility obligations, independent of lowering decisions.

use cq_lang_ix as ix;
use cq_lang_physical::{self as physical, Operator, syntax::symbol_name};
use std::collections::{BTreeMap, BTreeSet};

/// Check source/target contracts, logical erasure, IX demands, and recorded decisions.
/// Materialization is free whenever the language premises hold.
/// Formal rule CK-Physical: Check local contracts, RA erasure, IX demands and decision admissibility.
pub fn check(source: &ix::Program, target: &physical::Program) -> syn::Result<()> {
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
        let heading: BTreeSet<String> = facts.headings[&name].iter().cloned().collect();
        let inherited = match &definition.operator {
            Operator::Union(_) => None,
            Operator::Anti(_, k) => keys.get(&symbol_name(&k.left)).cloned(),
            Operator::Unit(_) => Some(BTreeSet::new()),
            Operator::Rename(rename) => target
                .program
                .host_is_set(&rename.relation)
                .then(|| heading.clone()),
            Operator::Probe(_, k) | Operator::Merge(_, k) => keys
                .get(&symbol_name(&k.left))
                .zip(keys.get(&symbol_name(&k.right)))
                .map(|(l, r)| l.union(r).cloned().collect()),
            Operator::Semi(_, k) => keys.get(&symbol_name(&k.left)).cloned(),
            Operator::Select(s) => keys.get(&symbol_name(&s.input)).cloned(),
            Operator::Point(_, s) | Operator::Range(_, s) => {
                keys.get(&symbol_name(&s.input)).cloned()
            }
            Operator::Bind(b) if b.for_token.is_none() => keys.get(&symbol_name(&b.input)).cloned(),
            Operator::Bind(_) => None,
            Operator::Guard(g) => keys.get(&symbol_name(&g.input)).cloned(),
            Operator::Project(p) => keys
                .get(&symbol_name(&p.input))
                .filter(|k| k.is_subset(&heading))
                .cloned(),
        };
        if let Some(key) = inherited.or_else(|| definition.canon.is_some().then_some(heading)) {
            keys.insert(name, key);
        }
    }
    let returned = symbol_name(&target.program.returned);
    if !keys.contains_key(&returned) && target.program.return_canon.is_none() {
        return Err(syn::Error::new_spanned(
            &target.program.returned,
            format!("return `{returned}`: not canonical by flag or plan key inference"),
        ));
    }
    Ok(())
}
