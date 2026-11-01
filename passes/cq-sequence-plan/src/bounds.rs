//! Upper bounds on Physical result sizes from recorded counts and keys.
//!
//! A result's bound is a function of recorded evidence only; it is never a
//! guess. A probe whose keys cover a key of its provider matches at most one
//! provider row per stream row (the stream bound is kept, and so are the
//! stream's keys). A probe whose keys cover a key of its stream matches each
//! provider row with at most one stream row (the provider's bound and keys).
//! Any other join, a generator, or an unknown input has no bound.
use std::collections::{BTreeMap, BTreeSet};

use cq_lang_sequence::structures::physical::Operator;
use cq_lang_sequence::{EvidenceFact, Program};
use cq_syntax::symbol_name;

/// What the evidence establishes about one Physical result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Bound {
    pub rows: Option<u128>,
    /// Column sets on which the result has at most one row.
    pub keys: Vec<BTreeSet<String>>,
}

/// Formal rule BE-PrefixBound: Bound each Physical result from recorded counts and keys: renames by count, filters and semijoins by their stream, keyed probes by the side whose recorded key the probe covers; anything else is unbounded.
pub(crate) fn of(program: &Program) -> BTreeMap<String, Bound> {
    let mut bounds: BTreeMap<String, Bound> = BTreeMap::new();
    let get = |bounds: &BTreeMap<String, Bound>, name: &syn::Ident| {
        bounds.get(&symbol_name(name)).cloned().unwrap_or_default()
    };
    let covers = |bound: &Bound, columns: &BTreeSet<String>| {
        bound.keys.iter().any(|key| key.is_subset(columns))
    };
    for definition in &program.structures.physical.program.definitions {
        let bound = match &definition.operator {
            Operator::Unit(_) => Bound {
                rows: Some(1),
                keys: vec![BTreeSet::new()],
            },
            Operator::Rename(rename) => {
                let relation = symbol_name(&rename.relation);
                let mapping: BTreeMap<_, _> = rename
                    .mapping
                    .iter()
                    .map(|column| (symbol_name(&column.source), symbol_name(&column.target)))
                    .collect();
                Bound {
                    rows: program.evidence.count(&relation).map(|rows| rows as u128),
                    keys: program
                        .evidence
                        .facts
                        .iter()
                        .filter_map(|fact| match fact {
                            EvidenceFact::Key { subject, columns }
                                if symbol_name(subject) == relation =>
                            {
                                columns
                                    .iter()
                                    .map(|column| mapping.get(&symbol_name(column)).cloned())
                                    .collect::<Option<BTreeSet<_>>>()
                            }
                            _ => None,
                        })
                        .collect(),
                }
            }
            Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
                get(&bounds, &select.input)
            }
            Operator::Guard(guard) => get(&bounds, &guard.input),
            Operator::Bind(bind) if bind.for_token.is_none() => get(&bounds, &bind.input),
            Operator::Project(project) => {
                let input = get(&bounds, &project.input);
                let kept: BTreeSet<_> = project.attributes.iter().map(symbol_name).collect();
                Bound {
                    rows: input.rows,
                    keys: input
                        .keys
                        .into_iter()
                        .filter(|key| key.is_subset(&kept))
                        .collect(),
                }
            }
            Operator::Group(group) => Bound {
                rows: get(&bounds, &group.input).rows,
                keys: vec![group.keys.iter().map(symbol_name).collect()],
            },
            Operator::Semi(_, keyed) | Operator::Anti(_, keyed) => get(&bounds, &keyed.left),
            Operator::Probe(_, keyed) | Operator::Merge(_, keyed) => {
                let (stream, provider) = (get(&bounds, &keyed.left), get(&bounds, &keyed.right));
                let columns: BTreeSet<_> = keyed.columns.iter().map(symbol_name).collect();
                if covers(&provider, &columns) {
                    stream
                } else if covers(&stream, &columns) {
                    provider
                } else {
                    Bound::default()
                }
            }
            Operator::Bind(_) | Operator::Union(_) => Bound::default(),
        };
        bounds.insert(symbol_name(&definition.name), bound);
    }
    bounds
}
