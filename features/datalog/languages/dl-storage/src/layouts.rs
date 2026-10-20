use crate::{Analysis, BodyItem, Negative, Program};
use cq_datalog_support::{columns, ensure, filters};
use cq_syntax::{rust, symbol_name};
use std::collections::{BTreeMap, BTreeSet};

/// Check declared representations and each recorded occurrence binding.
/// Per-layout maintenance covers even unused extra layouts.
pub(crate) fn check(program: &Program, analysis: &Analysis) -> syn::Result<()> {
    for (layout, maintenance) in program
        .layouts
        .iter()
        .map(|layout| (layout, "admitted_rows"))
        .chain(
            program
                .delta_layouts
                .iter()
                .map(|layout| (layout, "replace_each_run")),
        )
    {
        ensure(
            layout.maintenance == maintenance,
            &layout.maintenance,
            "layout maintenance differs from its full or delta lifetime",
        )?;
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
    let mut reads = BTreeMap::new();
    for read in &program.reads {
        ensure(
            reads.insert((read.variant, read.position), read).is_none(),
            program,
            "duplicate storage read binding",
        )?;
    }
    let mut negations = Vec::new();
    for (variant_index, variant) in program.incremental.variants.iter().enumerate() {
        let query = &program.incremental.strata.source.rules[variant.rule];
        let head = analysis.positions[&symbol_name(&query.head.relation)];
        let component = analysis.component[head].expect("checked IDB component");
        let mut bound = BTreeSet::new();
        for (position, item) in query.body.iter().enumerate() {
            match item {
                BodyItem::Positive { atom } => {
                    let read = reads.remove(&(variant_index, position)).ok_or_else(|| {
                        syn::Error::new_spanned(atom, "storage omitted a read binding")
                    })?;
                    let is_delta = variant.delta == Some(position);
                    if read.kind == "direct" {
                        ensure(
                            is_delta
                                && read.index == analysis.positions[&symbol_name(&atom.relation)]
                                && read.columns.iter().copied().eq(0..atom.args.len()),
                            atom,
                            "direct read must scan its delta relation in declaration order",
                        )?;
                    } else {
                        ensure(
                            if is_delta {
                                read.kind == "delta"
                            } else {
                                read.kind == "full"
                            },
                            atom,
                            "read kind differs from its recorded variant occurrence",
                        )?;
                        let layouts = if is_delta {
                            &program.delta_layouts
                        } else {
                            &program.layouts
                        };
                        let layout = layouts.get(read.index).ok_or_else(|| {
                            syn::Error::new_spanned(atom, "read names an undeclared layout")
                        })?;
                        ensure(
                            layout.relation == atom.relation && read.columns == layout.columns,
                            atom,
                            "read relation or column permutation differs from its layout",
                        )?;
                        for &column in &layout.columns[..layout.key] {
                            let arg = &atom.args[column];
                            let available = match rust::variable(arg) {
                                Some(id) => bound.contains(&symbol_name(id)),
                                None => !matches!(rust::unparen(arg), syn::Expr::Infer(_)),
                            };
                            ensure(
                                available,
                                arg,
                                "layout key column must be bound before this occurrence or constant",
                            )?;
                        }
                    }
                    bound.extend(atom.variables().map(symbol_name));
                }
                BodyItem::Let(binding) => {
                    bound.extend(rust::bindings(&binding.pattern).iter().map(symbol_name));
                }
                BodyItem::For(binding) => {
                    bound.extend(rust::bindings(&binding.pattern).iter().map(symbol_name));
                }
                BodyItem::Negation(negative) => {
                    let atom = &negative.atom;
                    let entry = Negative {
                        component,
                        relation: analysis.positions[&symbol_name(&atom.relation)],
                        columns: columns(atom),
                        filters: filters(atom),
                    };
                    if !negations.contains(&entry) {
                        negations.push(entry);
                    }
                }
                _ => {}
            }
        }
    }
    ensure(
        reads.is_empty(),
        program,
        "storage read binding does not name a positive occurrence",
    )?;
    ensure(
        program.negations == negations,
        program,
        "storage negative projection coverage changed",
    )
}
