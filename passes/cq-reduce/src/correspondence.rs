//! Reduction erasure and obligations, without the reducer's schedule builder.

use super::{Options, Pushdown, Reductions, erase, without_guards};
use cq_lang_vo::{self as vo, Reduction, Step, syntax::rust};
use std::collections::{BTreeMap, BTreeSet};

/// Check the source domain, unchanged query/steps, and reduction obligations.
/// A full tree reducer obeys dependencies, but need not use breadth-first order.
pub fn check(source: &vo::Program, target: &vo::Program, options: &Options) -> syn::Result<()> {
    let source_info = vo::contract::check(source)?;
    for step in &source.valuation.steps {
        if let Step::Reduce(r, _) = step {
            return Err(syn::Error::new_spanned(
                r,
                format!(
                    "clause #{} (`{}`): source already carries reductions",
                    r.clause(),
                    r.relation
                ),
            ));
        }
    }
    let tree = if options.semijoins == Reductions::Tree {
        Some(
            vo::cq::hypergraph::Analysis::of(&source.cq)
                .join_tree
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        &source.cq.name,
                        format!(
                            "query `{}`: Tree reduction requires an acyclic query",
                            source.cq.name
                        ),
                    )
                })?,
        )
    } else {
        None
    };
    vo::contract::check(target)?;
    if target.cq != source.cq {
        return Err(syn::Error::new_spanned(
            &target.cq.name,
            format!(
                "query `{}`: reductions must preserve source CQ `{}`",
                target.cq.name, source.cq.name
            ),
        ));
    }
    let (steps, guards) = erase(target);
    let retained = without_guards(&source.valuation.steps, &guards);
    if steps != retained {
        let position = steps
            .iter()
            .zip(&retained)
            .position(|(a, b)| a != b)
            .unwrap_or(steps.len().min(retained.len()));
        let construct = steps.get(position).or_else(|| retained.get(position));
        return Err(syn::Error::new_spanned(
            &target.cq.name,
            format!(
                "query `{}`: reduction erasure changes source step {position}: {}",
                source.cq.name,
                construct
                    .map(|s| match s {
                        Step::Atom(a, _) => format!("clause #{} (`{}`)", a.clause(), a.relation),
                        Step::Guard(_, _) => "guard clause".into(),
                        Step::Let(_, _) => "binding clause".into(),
                        Step::Reduce(r, _) => format!("reduction of clause #{}", r.clause()),
                    })
                    .unwrap_or_default()
            ),
        ));
    }
    let mut edges = BTreeMap::new();
    let parents: BTreeMap<usize, usize> = tree
        .into_iter()
        .flatten()
        .filter_map(|(child, parent)| parent.map(|p| (child, p)))
        .collect();
    let mut downward = false;
    for (position, step) in target.valuation.steps.iter().enumerate() {
        let Step::Reduce(r, _) = step else { continue };
        let clause = r.clause();
        let fail = |message: &str| {
            syn::Error::new_spanned(r, format!("clause #{clause} (`{}`): {message}", r.relation))
        };
        match &r.reduction {
            Reduction::Guard(filter) => {
                if options.guards == Pushdown::None {
                    return Err(fail("guards=None forbids `reduce ... if`"));
                }
                let free = rust::free_variables(&filter.condition);
                let variables: BTreeSet<_> = source_info.atoms[&clause]
                    .variables()
                    .map(vo::symbol_name)
                    .collect();
                if free.opaque || !free.names.is_subset(&variables) {
                    return Err(fail(
                        "guards=Atoms requires a guard over this atom clause's variables",
                    ));
                }
            }
            Reduction::With(_, support) => {
                if options.semijoins == Reductions::None {
                    return Err(fail("semijoins=None forbids `reduce ... with`"));
                }
                let other = support.clause();
                let up = parents.get(&other) == Some(&clause);
                let down = parents.get(&clause) == Some(&other);
                if !up && !down {
                    return Err(fail(&format!(
                        "semijoin with clause #{other} is not a join-tree edge"
                    )));
                }
                if edges.insert((clause, other), position).is_some() {
                    return Err(fail(&format!(
                        "duplicate join-tree reduction with clause #{other}"
                    )));
                }
                if up && downward {
                    return Err(fail(
                        "upward child-to-parent reductions must precede every downward reduction",
                    ));
                }
                downward |= down;
            }
        }
    }
    for (&child, &parent) in &parents {
        for edge in [(parent, child), (child, parent)] {
            if !edges.contains_key(&edge) {
                return Err(syn::Error::new_spanned(
                    &source_info.atoms[&edge.0],
                    format!(
                        "clause #{}: missing full-reducer edge with clause #{}",
                        edge.0, edge.1
                    ),
                ));
            }
        }
    }
    for (&child, &parent) in &parents {
        if let Some(&grandparent) = parents.get(&parent) {
            if edges[&(parent, child)] > edges[&(grandparent, parent)] {
                return Err(syn::Error::new_spanned(
                    &source_info.atoms[&parent],
                    format!("clause #{parent}: upward reduction must follow child clause #{child}"),
                ));
            }
            if edges[&(parent, grandparent)] > edges[&(child, parent)] {
                return Err(syn::Error::new_spanned(
                    &source_info.atoms[&child],
                    format!(
                        "clause #{child}: downward reduction must follow parent clause #{parent}"
                    ),
                ));
            }
        }
    }
    Ok(())
}
