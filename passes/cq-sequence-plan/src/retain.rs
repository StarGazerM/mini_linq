//! Retained small prefixes and segmented fan-out from recorded evidence
//! (P3.3 grading).
//!
//! A dispatch whose stream root fills less than one root tile runs its whole
//! fan-out in one block: H05's authored order starts from the one-row ASIA
//! region. The authored recipe R instead stores the joined Customer prefix,
//! distributes the remaining expansions over it and publishes offsets before
//! each fan-out (`tile 1024 items 4 store`). This pass makes those choices
//! from evidence alone: it retains (stores) the first expansion whose
//! evidence bound reaches the GPU machine's `retain_prefix_rows`, provided
//! another expansion follows it, then segments every later expansion whose
//! lookup does not cover a recorded key of its provider (a fan-out; keyed
//! lookups match at most one row and stay inline). Join order is unchanged;
//! both are the existing checked `store_level` and `segment` rewrites.
use std::collections::BTreeSet;

use cq_lang_sequence::{Machine, Program, Term};
use cq_syntax::symbol_name;

use crate::{Advice, bounds, decomposition};

/// The stream root reached from `term`, recording named expansions root first.
fn stream(program: &Program, term: &Term, path: &mut Vec<syn::Ident>) -> Option<String> {
    match term {
        Term::Rows(name) => Some(symbol_name(name)),
        Term::Ref(name) => {
            let definition = program
                .sequence
                .definitions
                .iter()
                .find(|definition| definition.name == *name)?;
            match &definition.term {
                Term::Store { .. } | Term::Rows(_) => Some(symbol_name(name)),
                inner => {
                    let root = stream(program, inner, path)?;
                    if expansion(inner) {
                        path.push(name.clone());
                    }
                    Some(root)
                }
            }
        }
        Term::Map(_, inner)
        | Term::Flatten(inner)
        | Term::Compact(_, inner)
        | Term::Canon(inner)
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Tile { inner, .. }
        | Term::Lookup { inner, .. } => stream(program, inner, path),
        Term::Store { .. } | Term::PartitionK { .. } | Term::Choose(_) | Term::Union(_) => None,
    }
}

fn expansion(term: &Term) -> bool {
    matches!(term, Term::Flatten(inner) if matches!(inner.as_ref(), Term::Lookup { .. }))
}

/// Formal rule GR-RetainPrefix: When a GPU dispatch streams a root whose evidence bound is below one root tile, retain the first expansion whose evidence bound reaches the machine's retain_prefix_rows if another expansion follows it.
/// Formal rule GR-SegmentFanout: After a retained prefix, segment every expansion whose lookup covers no recorded key of its provider, at the machine's distributing width times its items.
pub(crate) fn advise(program: &Program, machine: &Machine) -> Vec<Advice> {
    let (Some(gpu), Some(tile)) = (&machine.gpu, decomposition::width(&machine.root)) else {
        return Vec::new();
    };
    let bounds = bounds::of(program);
    let rows = |name: &str| bounds.get(name).and_then(|bound| bound.rows);
    let fan_out = |name: &syn::Ident| {
        let Some(definition) = program
            .sequence
            .definitions
            .iter()
            .find(|definition| definition.name == *name)
        else {
            return false;
        };
        let Term::Flatten(inner) = &definition.term else {
            return false;
        };
        let Term::Lookup { provider, keys, .. } = inner.as_ref() else {
            return false;
        };
        let keys: BTreeSet<_> = keys.iter().map(symbol_name).collect();
        !bounds
            .get(&symbol_name(provider))
            .is_some_and(|bound| bound.keys.iter().any(|key| key.is_subset(&keys)))
    };
    let mut chosen = BTreeSet::new();
    let mut advice = Vec::new();
    // Later expansions see longer chains; the longest decides each prefix.
    for definition in program.sequence.definitions.iter().rev() {
        if !expansion(&definition.term) {
            continue;
        }
        let mut path = Vec::new();
        let Some(root) = stream(program, &Term::Ref(definition.name.clone()), &mut path) else {
            continue;
        };
        if rows(&root).is_none_or(|rows| rows >= tile as u128) {
            continue;
        }
        for (index, name) in path.iter().enumerate() {
            let Some(bound) = rows(&symbol_name(name)) else {
                break;
            };
            if bound >= gpu.retain_prefix_rows as u128 {
                if index + 1 < path.len() && chosen.insert(symbol_name(name)) {
                    advice.push(Advice::Retain {
                        name: name.clone(),
                        rows: bound,
                    });
                    // The store consuming the rest of the chain is distributed
                    // over the retained prefix, one root tile per part.
                    let last = path.last().expect("nonempty chain");
                    if let Some(consumer) = program.sequence.definitions.iter().find(|store| {
                        let Term::Store { inner, .. } = &store.term else {
                            return false;
                        };
                        let mut consumed = Vec::new();
                        stream(program, inner, &mut consumed).is_some() && consumed.contains(last)
                    }) {
                        advice.push(Advice::Partition {
                            name: consumer.name.clone(),
                            n: tile,
                            items: None,
                        });
                    }
                    for later in &path[index + 1..] {
                        if fan_out(later) && chosen.insert(symbol_name(later)) {
                            advice.push(Advice::Segment {
                                name: later.clone(),
                                n: tile * machine.items,
                                items: (machine.items > 1).then_some(machine.items),
                            });
                        }
                    }
                }
                break;
            }
        }
    }
    advice
}

/// Check a recorded retention against the recorded evidence, then store it.
pub(crate) fn apply_in(
    checker: &mut cq_lang_sequence::contract::Checker,
    source: &Program,
    name: &syn::Ident,
    rows: u128,
) -> syn::Result<Program> {
    let bound = bounds::of(source)
        .get(&symbol_name(name))
        .and_then(|bound| bound.rows);
    if bound != Some(rows) {
        return Err(syn::Error::new_spanned(
            name,
            format!(
                "GR-RetainPrefix: recorded bound {rows} differs from the evidence bound {bound:?}"
            ),
        ));
    }
    crate::rewrites::store_level_in(checker, source, quote::quote!(#name))
}
