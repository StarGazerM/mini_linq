use crate::{Analysis, BodyItem, Layout, Program};
use cq_datalog_support::{ensure, key_columns};
use cq_syntax::symbol_name;
use std::collections::BTreeSet;

/// Check physical grouping decisions against source rules, not emitted variants
/// or the scheduling compiler. Variant order and delta spelling are irrelevant.
pub(crate) fn check(program: &Program, analysis: &Analysis) -> syn::Result<()> {
    for layout in program.layouts.iter().chain(&program.delta_layouts) {
        let index = analysis
            .positions
            .get(&symbol_name(&layout.relation))
            .ok_or_else(|| {
                syn::Error::new_spanned(&layout.relation, "layout relation is not declared")
            })?;
        let arity = program.incremental.strata.source.declarations[*index].arity();
        let columns: BTreeSet<_> = layout.columns.iter().copied().collect();
        ensure(
            layout.columns.len() == arity
                && columns.len() == arity
                && columns.iter().copied().eq(0..arity),
            &layout.relation,
            "layout columns must be a complete column permutation",
        )?;
        ensure(
            layout.key <= arity,
            &layout.relation,
            "layout key exceeds relation arity",
        )?;
    }
    for layout in &program.delta_layouts {
        let index = analysis.positions[&symbol_name(&layout.relation)];
        ensure(
            program.incremental.strata.source.declarations[index]
                .value
                .is_none(),
            &layout.relation,
            "delta layout must name a base IDB relation",
        )?;
    }
    let mut expected = Vec::new();
    let mut expected_delta = Vec::new();
    for (rule, query) in program.incremental.strata.source.rules.iter().enumerate() {
        let changing = &analysis.changing_positions[rule];
        let drivers: Vec<_> = if changing.is_empty() {
            vec![None]
        } else {
            changing.iter().copied().map(Some).collect()
        };
        for driver in drivers {
            let mut bound = BTreeSet::new();
            for (position, item) in query.body.iter().enumerate() {
                let BodyItem::Positive { atom } = item else {
                    continue;
                };
                let (columns, key) = key_columns(atom, &bound);
                let layout = Layout {
                    relation: atom.relation.clone(),
                    columns,
                    key,
                };
                let expected = if Some(position) == driver {
                    if key == 0 && position == 0 {
                        bound.extend(atom.variables().map(symbol_name));
                        continue;
                    }
                    &mut expected_delta
                } else {
                    &mut expected
                };
                if !expected.contains(&layout) {
                    expected.push(layout);
                }
                bound.extend(atom.variables().map(symbol_name));
            }
        }
    }
    ensure(
        program.layouts == expected,
        program,
        "layouts must exactly match written-order positive join choices in stable rule order",
    )?;
    ensure(
        program.delta_layouts == expected_delta,
        program,
        "delta layouts must exactly match written-order delta probes in stable rule order",
    )
}
