//! Conditional planning bounds on Physical result sizes.
//!
//! Numeric bounds depend on authored estimates, not certified cardinalities;
//! they must not justify truncation or unchecked memory access. A probe whose
//! keys cover a derived key of its provider matches at most one provider row
//! per stream row, preserving the stream bound and keys. If the keys cover a
//! derived key of the stream, the provider bound and keys are preserved.
//! Filters, scalar binds and projections preserve row bounds; projection
//! preserves only surviving keys. Groups preserve the input row bound and
//! introduce their grouping key. Unit has one row and the empty key.
use std::collections::{BTreeMap, BTreeSet};

use crate::structures::physical::Operator;
use crate::Program;
use cq_syntax::symbol_name;

/// Conditional size advice and independently derived uniqueness facts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bound {
    /// Estimated upper bound, never a capacity or truncation certificate.
    pub rows: Option<u128>,
    /// Column sets on which the result has at most one row.
    pub keys: Vec<BTreeSet<String>>,
    /// BE-PrefixEstimate: a nonsemantic row estimate that also uses recorded
    /// filtered counts; geometry advice reads it, never capacities.
    pub estimate: Option<u128>,
}

/// Formal rule BE-PrefixEstimate: A recorded count for the result itself, else the relation count at a rename; filters, scalar binds, projections and semijoins keep their input's estimate; a key-covered probe scales the matched side's estimate by the keyed side's estimate/bound fraction; generators and Union are unknown.
fn estimate(
    definition: &crate::structures::physical::Definition,
    program: &Program,
    get: &dyn Fn(&syn::Ident) -> Bound,
    covers: &dyn Fn(&Bound, &crate::structures::physical::Keyed) -> bool,
) -> Option<u128> {
    if let Some(rows) = program.evidence.count(&symbol_name(&definition.name)) {
        return Some(rows as u128);
    }
    // The fraction of a keyed side's bound that its estimate keeps.
    let scaled = |matched: Option<u128>, keyed: &Bound| {
        let matched = matched?;
        Some(match (keyed.estimate, keyed.rows) {
            (Some(estimate), Some(rows)) if rows > 0 => {
                matched.saturating_mul(estimate).div_ceil(rows)
            }
            _ => matched,
        })
    };
    match &definition.operator {
        Operator::Unit(_) => Some(1),
        Operator::Rename(rename) => program
            .evidence
            .count(&symbol_name(&rename.relation))
            .map(|rows| rows as u128),
        Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
            get(&select.input).estimate
        }
        Operator::Guard(guard) => get(&guard.input).estimate,
        Operator::Bind(bind) if bind.for_token.is_none() => get(&bind.input).estimate,
        Operator::Project(project) => get(&project.input).estimate,
        Operator::Group(group) if group.keys.is_empty() => Some(1),
        Operator::Group(group) => get(&group.input).estimate,
        Operator::Semi(_, keyed) | Operator::Anti(_, keyed) => get(&keyed.left).estimate,
        Operator::Probe(_, keyed) | Operator::Merge(_, keyed) => {
            let (stream, provider) = (get(&keyed.left), get(&keyed.right));
            if covers(&provider, keyed) {
                scaled(stream.estimate, &provider)
            } else if covers(&stream, keyed) {
                scaled(provider.estimate, &stream)
            } else {
                None
            }
        }
        Operator::Bind(_) | Operator::Union(_) => None,
    }
}

fn covers_keyed(bound: &Bound, keyed: &crate::structures::physical::Keyed) -> bool {
    let columns: BTreeSet<_> = keyed.columns.iter().map(symbol_name).collect();
    bound.keys.iter().any(|key| key.is_subset(&columns))
}

/// Formal rule BE-PrefixBound: Record count estimates and full-heading input keys through Unit, Rename, filters, scalar Bind, Project, Group, Semi/Anti and keyed Probe/Merge; generators and Union have no bound.
pub(crate) fn record(physical: &crate::structures::physical::Program, program: &Program) -> BTreeMap<String, Bound> {
    let mut bounds: BTreeMap<String, Bound> = BTreeMap::new();
    let get = |bounds: &BTreeMap<String, Bound>, name: &syn::Ident| {
        bounds.get(&symbol_name(name)).cloned().unwrap_or_default()
    };
    let covers = |bound: &Bound, columns: &BTreeSet<String>| {
        bound.keys.iter().any(|key| key.is_subset(columns))
    };
    for definition in &physical.program.definitions {
        let mut bound = match &definition.operator {
            Operator::Unit(_) => Bound {
                rows: Some(1),
                keys: vec![BTreeSet::new()],
                estimate: None,
            },
            Operator::Rename(rename) => {
                let relation = symbol_name(&rename.relation);
                Bound {
                    rows: program.evidence.count(&relation).map(|rows| rows as u128),
                    keys: vec![rename.mapping.iter().map(|column| symbol_name(&column.target)).collect()],
                    estimate: None,
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
                    estimate: None,
                }
            }
            Operator::Group(group) => Bound {
                rows: if group.keys.is_empty() {
                    Some(1)
                } else {
                    get(&bounds, &group.input).rows
                },
                keys: vec![group.keys.iter().map(symbol_name).collect()],
                estimate: None,
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
        bound.estimate = estimate(
            definition,
            program,
            &|name| get(&bounds, name),
            &covers_keyed,
        );
        bounds.insert(symbol_name(&definition.name), bound);
    }
    bounds
}

/// Check BE-PrefixBound locally against evidence and preceding recorded facts.
/// A record may omit keys or weaken an upper bound. At a keyed join either
/// justified side is legal; this does not replay `record`'s preferred side.
pub fn check(physical: &crate::structures::physical::Program, program: &Program, bounds: &BTreeMap<String, Bound>) -> syn::Result<()> {
    fn weakens(recorded: &Bound, premise: &Bound) -> bool {
        recorded
            .rows
            .is_none_or(|rows| premise.rows.is_some_and(|limit| limit <= rows))
            && recorded.keys.iter().all(|key| premise.keys.contains(key))
    }
    let definitions = &physical.program.definitions;
    if bounds.len() != definitions.len() {
        return Err(syn::Error::new_spanned(
            program,
            "BE-PrefixBound: incomplete result coverage",
        ));
    }
    let mut preceding = BTreeMap::<String, &Bound>::new();
    let unknown = Bound::default();
    for definition in definitions {
        let name = symbol_name(&definition.name);
        let Some(bound) = bounds.get(&name) else {
            return Err(syn::Error::new_spanned(
                &definition.name,
                "BE-PrefixBound: missing result",
            ));
        };
        let get = |name: &syn::Ident| {
            preceding
                .get(&symbol_name(name))
                .copied()
                .unwrap_or(&unknown)
        };
        let covers =
            |bound: &Bound, columns: &syn::punctuated::Punctuated<syn::Ident, syn::Token![,]>| {
                bound.keys.iter().any(|key| {
                    key.iter().all(|column| {
                        columns
                            .iter()
                            .any(|candidate| symbol_name(candidate) == *column)
                    })
                })
            };
        let legal = match &definition.operator {
            Operator::Unit(_) => weakens(
                bound,
                &Bound {
                    rows: Some(1),
                    keys: vec![BTreeSet::new()],
                    estimate: None,
                },
            ),
            Operator::Rename(rename) => {
                let relation = symbol_name(&rename.relation);
                bound.rows.is_none_or(|rows| {
                    program
                        .evidence
                        .count(&relation)
                        .is_some_and(|limit| limit as u128 <= rows)
                }) && bound.keys.iter().all(|key| {
                    *key == rename.mapping.iter().map(|column| symbol_name(&column.target)).collect()
                })
            }
            Operator::Select(select) | Operator::Point(_, select) | Operator::Range(_, select) => {
                weakens(bound, get(&select.input))
            }
            Operator::Guard(guard) => weakens(bound, get(&guard.input)),
            Operator::Bind(bind) if bind.for_token.is_none() => weakens(bound, get(&bind.input)),
            Operator::Project(project) => {
                weakens(bound, get(&project.input))
                    && bound.keys.iter().all(|key| {
                        key.iter().all(|column| {
                            project
                                .attributes
                                .iter()
                                .any(|kept| symbol_name(kept) == *column)
                        })
                    })
            }
            Operator::Group(group) => weakens(
                bound,
                &Bound {
                    rows: if group.keys.is_empty() {
                        Some(1)
                    } else {
                        get(&group.input).rows
                    },
                    keys: vec![group.keys.iter().map(symbol_name).collect()],
                    estimate: None,
                },
            ),
            Operator::Semi(_, keyed) | Operator::Anti(_, keyed) => weakens(bound, get(&keyed.left)),
            Operator::Probe(_, keyed) | Operator::Merge(_, keyed) => {
                let (left, right) = (get(&keyed.left), get(&keyed.right));
                weakens(bound, &unknown)
                    || (covers(right, &keyed.columns) && weakens(bound, left))
                    || (covers(left, &keyed.columns) && weakens(bound, right))
            }
            Operator::Bind(_) | Operator::Union(_) => weakens(bound, &unknown),
        };
        if !legal {
            return Err(syn::Error::new_spanned(
                &definition.name,
                "BE-PrefixBound: recorded rows or keys lack an evidence/transfer premise",
            ));
        }
        // BE-PrefixEstimate: unknown, or exactly the transfer of the preceding records.
        if bound.estimate.is_some()
            && bound.estimate
                != estimate(definition, program, &|name| get(name).clone(), &covers_keyed)
        {
            return Err(syn::Error::new_spanned(
                &definition.name,
                "BE-PrefixEstimate: recorded estimate is not its evidence/transfer estimate",
            ));
        }
        preceding.insert(name, bound);
    }
    Ok(())
}
