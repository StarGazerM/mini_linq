//! Record the exact Boolean semi-naive cover without storage decisions.
use cq_datalog_support::{Fresh, ensure};
use cq_lang_datalog_incremental::{Delta, Program, Variant, contract};
use cq_lang_datalog_strata as strata;
use cq_syntax::symbol_name;
use std::collections::BTreeMap;
use strata::source::BodyItem;
/// CPU scheduling preserves written order; device scheduling may lead with delta.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DriverPolicy {
    #[default]
    WrittenOrder,
    DeltaFirst,
}
pub fn compile(source: &strata::Program) -> syn::Result<Program> {
    compile_with_driver(source, DriverPolicy::WrittenOrder)
}

pub fn compile_with_driver(source: &strata::Program, driver_policy: DriverPolicy) -> syn::Result<Program> {
    let analysis = strata::contract::check(source)?;
    let mut fresh = Fresh::new(source);
    let mut deltas = Vec::new();
    let mut names = BTreeMap::new();
    for declaration in source
        .source
        .declarations
        .iter()
        .filter(|d| d.value.is_none())
    {
        let name = fresh.id("delta");
        names.insert(symbol_name(&declaration.name), name.clone());
        deltas.push(Delta {
            relation: declaration.name.clone(),
            name,
        });
    }
    let mut variants = Vec::new();
    for (rule, query) in source.source.rules.iter().enumerate() {
        let positions = &analysis.changing_positions[rule];
        let first = query.body.iter().position(|item| matches!(item, BodyItem::Positive { .. }));
        if positions.is_empty() {
            variants.push(Variant {
                rule,
                delta: None,
                driver: first,
                query: query.clone(),
            });
        }
        for &position in positions {
            let mut query = query.clone();
            let BodyItem::Positive { atom } = &mut query.body[position] else {
                unreachable!("checked changing position")
            };
            atom.relation = names[&symbol_name(&atom.relation)].clone();
            let driver = if driver_policy == DriverPolicy::DeltaFirst
                && first.is_some_and(|first| query.body.iter().skip(first).take(position - first + 1)
                    .all(|item| matches!(item, BodyItem::Positive { .. })))
            {
                Some(position)
            } else {
                first
            };
            if let (Some(first), Some(driver)) = (first, driver) {
                if first != driver {
                    let mut body: Vec<_> = query.body.into_iter().collect();
                    let item = body.remove(driver);
                    body.insert(first, item);
                    query.body = body.into_iter().collect();
                }
            }
            variants.push(Variant {
                rule,
                delta: Some(position),
                driver,
                query,
            });
        }
    }
    let target = Program {
        strata: source.clone(),
        variants,
        deltas,
    };
    check(source, &target)?;
    Ok(target)
}
/// Independently check cover, erasure, fresh schemas, and the embedded Strata source.
pub fn check(source: &strata::Program, target: &Program) -> syn::Result<()> {
    contract::check(target)?;
    ensure(
        source == &target.strata,
        target,
        "incremental changed strata",
    )
}
