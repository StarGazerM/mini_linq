use crate::{Analysis, BodyItem, Layout, Program, ensure};
use cq_syntax::{rust, symbol_name};
use std::collections::BTreeSet;

/// Check physical grouping decisions against source rules, not emitted variants
/// or the scheduling compiler. Variant order and delta spelling are irrelevant.
pub(crate) fn check(program: &Program, analysis: &Analysis) -> syn::Result<()> {
    if program.variants.is_none() {
        return Ok(());
    }
    for layout in &program.layouts {
        let index = analysis
            .positions
            .get(&symbol_name(&layout.relation))
            .ok_or_else(|| {
                syn::Error::new_spanned(&layout.relation, "layout relation is not declared")
            })?;
        let arity = program.declarations[*index].arity();
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
    let mut expected = Vec::new();
    for (rule, query) in program.rules.iter().enumerate() {
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
                if Some(position) == driver {
                    bound.extend(atom.variables().map(symbol_name));
                    continue;
                }
                let is_bound: Vec<_> = atom
                    .args
                    .iter()
                    .map(|arg| match rust::variable(arg) {
                        Some(name) => bound.contains(&symbol_name(name)),
                        None => !matches!(rust::unparen(arg), syn::Expr::Infer(_)),
                    })
                    .collect();
                let mut columns: Vec<_> = (0..atom.args.len())
                    .filter(|&column| is_bound[column])
                    .collect();
                let key = columns.len();
                columns.extend((0..atom.args.len()).filter(|&column| !is_bound[column]));
                let layout = Layout {
                    relation: atom.relation.clone(),
                    columns,
                    key,
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
    )
}
