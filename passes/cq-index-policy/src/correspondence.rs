//! RA preservation, read coverage, and canonical first-use requirement naming.

use cq_lang_ix as ix;
use cq_lang_ra as ra;
use cq_lang_ra::syntax::symbol_name;

/// Check unchanged RA, coverage of every keyed read, and absence of unused demands.
/// Each recorded descriptor is matched against the read's relation, keys and supply;
/// the producer's demand traversal is not replayed. Names follow first use.
/// Formal rule CK-Index: Check read coverage, no extras, and iN names in first-use order.
pub fn check(source: &ra::Program, target: &ix::Program) -> syn::Result<()> {
    let info = ra::contract::check(source)?;
    ix::contract::check(target)?;
    if target.ra != *source {
        let construct = source
            .plan
            .definitions
            .iter()
            .zip(&target.ra.plan.definitions)
            .find(|(s, t)| s != t)
            .map(|(s, _)| &s.result)
            .unwrap_or(&source.plan.output.input);
        return Err(syn::Error::new_spanned(
            construct,
            format!("result `{construct}`: IX must preserve the source RA program"),
        ));
    }
    let reads = ix::read_requirements(target, &info)?;
    let mut seen = std::collections::BTreeSet::new();
    for definition in &source.plan.definitions {
        let Some(required) = reads.get(&symbol_name(&definition.result)) else {
            continue;
        };
        if !seen.insert(symbol_name(&required.name)) {
            continue;
        }
        let position = seen.len() - 1;
        // IX-New: name each newly covered requirement iN in first-use order.
        let expected_name = format!("i{position}");
        let actual = &target.indexes[position];
        if actual.name != required.name {
            return Err(syn::Error::new_spanned(
                actual,
                format!(
                    "definition `{}`: demand `{}` for `{}` is not in first-use order",
                    definition.result, actual.name, actual.requirement.relation
                ),
            ));
        }
        if symbol_name(&actual.name) != expected_name {
            return Err(syn::Error::new_spanned(
                &actual.name,
                format!(
                    "definition `{}`: demand `{}` must be named `{expected_name}`",
                    definition.result, actual.name
                ),
            ));
        }
    }
    Ok(())
}
