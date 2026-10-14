//! RA preservation and canonical first-use naming from language-owned demands.

use cq_lang_ix as ix;
use cq_lang_ra as ra;
use cq_lang_ra::syntax::symbol_name;

/// Check RA preservation and the IX-Join/Const/Other/New/Reuse derivation in order.
/// Demand names are fixed by the rules; no compiler decisions are replayed.
/// Formal rule CK-Index: Check unchanged RA and complete first-use descriptors with iN names.
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
    let demands = ix::demands(source, &info);
    for (position, demand) in demands.iter().enumerate() {
        let right = demand.relation;
        let definition = demand.definition;
        let descriptor = &demand.signature;
        // IX-New: the next demand is named iN, in first-use order.
        let expected_name = format!("i{position}");
        let Some(actual) = target.indexes.iter().nth(position) else {
            return Err(syn::Error::new_spanned(
                definition,
                format!(
                    "definition `{}`: missing demand `{expected_name}` for `{right}`",
                    definition
                ),
            ));
        };
        if &ix::signature(&actual.requirement) != descriptor {
            return Err(syn::Error::new_spanned(
                actual,
                format!(
                    "definition `{}`: demand `{}` must be the first-use demand for `{right}` on {:?} with {:?} supply",
                    definition, actual.name, descriptor.1, descriptor.2
                ),
            ));
        }
        if symbol_name(&actual.name) != expected_name {
            return Err(syn::Error::new_spanned(
                &actual.name,
                format!(
                    "definition `{}`: demand `{}` must be named `{expected_name}`",
                    definition, actual.name
                ),
            ));
        }
    }
    if let Some(extra) = target.indexes.iter().nth(demands.len()) {
        return Err(syn::Error::new_spanned(
            extra,
            format!(
                "demand `{}` for `{}`: no new source keyed read requires this entry",
                extra.name, extra.requirement.relation
            ),
        ));
    }
    Ok(())
}
