//! Logical erasure and admissibility obligations, independent of lowering decisions.

use cq_lang_ix as ix;
use cq_lang_physical::{self as physical, syntax::symbol_name};
use std::collections::BTreeMap;

/// Check source/target contracts, logical erasure, IX demands, and recorded decisions.
/// Materialization is free whenever the language premises hold.
/// Formal rule CK-Physical: Check local contracts, RA erasure, IX demands and decision admissibility.
/// Formal rule CK-Return: Require an inferred key, return canon, or a canonical consumer.
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
    let mut keys: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for definition in &target.program.definitions {
        let name = symbol_name(&definition.name);
        let heading = &facts.headings[&name];
        let inherited = physical::inherited_key(
            &definition.operator,
            heading,
            &target.program.host_sets,
            |input| keys.get(&symbol_name(input)).cloned(),
        );
        if let Some(key) = inherited.or_else(|| definition.canon.is_some().then(|| heading.clone()))
        {
            keys.insert(name, key);
        }
    }
    let returned = symbol_name(&target.program.returned);
    if !keys.contains_key(&returned)
        && target.program.return_canon.is_none()
        && !facts.consumer_canonical
    {
        return Err(syn::Error::new_spanned(
            &target.program.returned,
            format!("return `{returned}`: not canonical by flag or plan key inference"),
        ));
    }
    Ok(())
}
