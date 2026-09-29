//! RA preservation and canonical first-use naming, independently of construction.

use cq_lang_ix as ix;
use cq_lang_ra as ra;
use cq_lang_ra::syntax::symbol_name;

/// Check RA preservation and the IX-Join/Other/New/Reuse derivation in order.
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
    let mut demands = Vec::new();
    for definition in &source.plan.definitions {
        let (left, right) = match &definition.operator {
            // IX-Join: increasing logical ordinals in the right heading.
            ra::Operator::NaturalJoin(j) => (&j.left, &j.right),
            ra::Operator::Semijoin(j) => (&j.left, &j.right),
            // IX-Other: no demand is introduced by other operators.
            _ => continue,
        };
        let columns: Vec<usize> = info[&symbol_name(right)]
            .heading
            .iter()
            .enumerate()
            .filter_map(|(ordinal, column)| {
                info[&symbol_name(left)]
                    .heading
                    .contains(column)
                    .then_some(ordinal)
            })
            .collect();
        let descriptor = (symbol_name(right), columns);
        // IX-Reuse: only the first occurrence contributes a demand.
        if demands.contains(&descriptor) {
            continue;
        }
        // IX-New: the next demand is named iN, in first-use order.
        let position = demands.len();
        let expected_name = format!("i{position}");
        let Some(actual) = target.indexes.iter().nth(position) else {
            return Err(syn::Error::new_spanned(
                &definition.result,
                format!(
                    "definition `{}`: missing demand `{expected_name}` for `{right}`",
                    definition.result
                ),
            ));
        };
        if ix::signature(&actual.requirement) != descriptor {
            return Err(syn::Error::new_spanned(
                actual,
                format!(
                    "definition `{}`: demand `{}` must be the first-use demand for `{right}` on {:?}",
                    definition.result, actual.name, descriptor.1
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
        demands.push(descriptor);
    }
    if let Some(extra) = target.indexes.iter().nth(demands.len()) {
        return Err(syn::Error::new_spanned(
            extra,
            format!(
                "demand `{}` for `{}`: no new source join requires this entry",
                extra.name, extra.requirement.relation
            ),
        ));
    }
    Ok(())
}
