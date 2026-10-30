//! Shared occurrence-phase checker for automatic and advised Lookup/Expand plans.
use cq::symbol_name;
use cq_lang_cq as cq;
use cq_lang_vo as vo;
use std::collections::{BTreeMap, BTreeSet};

// Formal rule CK-Le: Check occurrence phases and column ownership independently of scheduling.
pub fn check(normal: &cq::Program, target: &vo::Program) -> syn::Result<()> {
    check_selected(normal, target, None)
}

/// Check explicitly selected lookup/expand occurrences; other reads obey WF-VO.
pub(crate) fn check_advice(
    normal: &cq::Program,
    target: &vo::Program,
    lookups: &BTreeSet<usize>,
) -> syn::Result<()> {
    check_selected(normal, target, Some(lookups))
}

fn check_selected(
    normal: &cq::Program,
    target: &vo::Program,
    selected: Option<&BTreeSet<usize>>,
) -> syn::Result<()> {
    if normal != &target.cq {
        return Err(syn::Error::new_spanned(
            target,
            "LE must preserve normalized CQ",
        ));
    }
    let analysis = vo::contract::check(target)?;
    fn scope(
        body: &cq::CommaList<cq::BodyItem>,
        steps: &[vo::Step],
        analysis: &vo::Analysis,
        context: &BTreeSet<String>,
        selected: Option<&BTreeSet<usize>>,
    ) -> syn::Result<()> {
        let mut written = context.clone();
        let mut ownership = BTreeMap::new();
        let written_outer = body.iter().position(|item| {
            matches!(item, cq::BodyItem::Positive { .. } | cq::BodyItem::Block(_))
        });
        for (clause, item) in body.iter().enumerate() {
            match item {
                cq::BodyItem::Positive { atom } => {
                    let columns: BTreeSet<_> = atom.variables().map(symbol_name).collect();
                    ownership.insert(
                        clause,
                        columns
                            .difference(&written)
                            .cloned()
                            .collect::<BTreeSet<_>>(),
                    );
                    written.extend(columns);
                }
                cq::BodyItem::Let(_) | cq::BodyItem::For(_) => written.extend(
                    item.rust_step()
                        .expect("Rust binding")
                        .bindings
                        .iter()
                        .map(symbol_name),
                ),
                cq::BodyItem::Block(block) => {
                    written.extend(block.analyze(&written).interface.into_keys())
                }
                _ => (),
            }
        }
        let first = steps.iter().find_map(|step| match step {
            vo::Step::Atom(a, _) => Some(a.clause()),
            vo::Step::Block(b, _) => Some(b.source.clause()),
            _ => None,
        });
        // Automatic Plan-Le keeps the written outer stream and written column
        // ownership. An advised schedule may choose its own root: its outer
        // stream is its first read, and WF-VO alone fixes which columns each
        // expansion introduces.
        let outer = if selected.is_some() {
            first
        } else {
            written_outer
        };
        if selected.is_none() && first != outer {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "LE must retain the written outer stream",
            ));
        }
        let mut phases: BTreeMap<usize, usize> = BTreeMap::new();
        for (index, step) in steps.iter().enumerate() {
            match step {
                vo::Step::Atom(atom, _) => {
                    if selected.is_some_and(|selected| !selected.contains(&atom.clause())) {
                        continue;
                    }
                    let phase = phases.entry(atom.clause()).or_default();
                    let stream = Some(atom.clause()) == outer;
                    let full = stream || *phase == 1;
                    if *phase >= if stream { 1 } else { 2 }
                        || (full && atom.variables().count() != atom.columns.len())
                        || (!full && !analysis.introduces[index].is_empty())
                        || (full
                            && selected.is_none()
                            && analysis.introduces[index]
                                .iter()
                                .cloned()
                                .collect::<BTreeSet<_>>()
                                != ownership[&atom.clause()])
                    {
                        return Err(syn::Error::new_spanned(
                            atom,
                            "LE requires a non-introducing lookup followed by one coarse expansion with written column ownership",
                        ));
                    }
                    *phase += 1;
                }
                vo::Step::Block(block, _) => {
                    let cq::BodyItem::Block(source) = &body[block.source.clause()] else {
                        unreachable!("checked VO block")
                    };
                    let bound = analysis.bound_before[index].iter().cloned().collect();
                    let facts = source.analyze(&bound);
                    let context = if facts.rust {
                        bound
                    } else {
                        facts.free.into_keys().collect()
                    };
                    for (branch, (source, facts)) in block.branches.iter().zip(
                        source
                            .branches
                            .iter()
                            .zip(&analysis.branches[&block.source.clause()]),
                    ) {
                        // Hand advice only names top-level occurrences; nested
                        // scopes keep the already checked ordinary schedule.
                        if selected.is_none() {
                            scope(&source.body, &branch.steps, facts, &context, None)?;
                        }
                    }
                }
                _ => (),
            }
        }
        for (&clause, atom) in &analysis.atoms {
            if body[clause].atom().is_none()
                || selected.is_some_and(|selected| !selected.contains(&clause))
            {
                continue;
            }
            if phases.get(&clause).copied() != Some(if Some(clause) == outer { 1 } else { 2 }) {
                return Err(syn::Error::new_spanned(
                    atom,
                    "LE occurrence is missing its lookup or expansion",
                ));
            }
        }
        // WF-VO independently establishes all first-use and Rust dependency obligations,
        // full correlated rows, block interfaces, and fallback ancestry.
        Ok(())
    }
    scope(
        &normal.query.body,
        &target.valuation.steps,
        &analysis,
        &BTreeSet::new(),
        selected,
    )
}
