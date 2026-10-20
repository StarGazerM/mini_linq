//! Record the exact Boolean semi-naive cover without storage decisions.
use cq_datalog_support::{Fresh, ensure};
use cq_lang_datalog_incremental::{Delta, Program, Variant, contract};
use cq_lang_datalog_strata as strata;
use cq_syntax::symbol_name;
use std::collections::BTreeMap;
use strata::source::BodyItem;
pub fn compile(source: &strata::Program) -> syn::Result<Program> {
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
        if positions.is_empty() {
            variants.push(Variant {
                rule,
                delta: None,
                query: query.clone(),
            });
        }
        for &position in positions {
            let mut query = query.clone();
            let BodyItem::Positive { atom } = &mut query.body[position] else {
                unreachable!("checked changing position")
            };
            atom.relation = names[&symbol_name(&atom.relation)].clone();
            variants.push(Variant {
                rule,
                delta: Some(position),
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
/// Independently check cover, erasure, fresh schemas, and preservation.
pub fn check(source: &strata::Program, target: &Program) -> syn::Result<()> {
    strata::contract::check(source)?;
    contract::check(target)?;
    ensure(
        source == &target.strata,
        target,
        "incremental changed strata",
    )
}
