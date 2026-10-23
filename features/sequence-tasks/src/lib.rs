//! Sequence interpretation for an ordered, dynamically scheduled Rayon task pool.
//!
//! Generated expressions require `rayon` and `cq-storage` in the consumer. The
//! caller supplies facts checked against [`machine`]; translation never repeats
//! analysis. Host and task-local constructors reuse the sequential interpreter.
//! Pool chunks own their buffers and publish in chunk order at the join. Rayon
//! propagates a task panic to that join rather than publishing partial output.
//! Grouped folds reuse the host hash fold after publication; no task-pool
//! collective or per-chunk grouping may replace that global valuation domain.

use cq_lang_sequence::{Analysis, LevelNode, Machine, Program, Term, Tiling};
use cq_sequence_pull::{Emitter, Value};
use quote::quote;

/// The host owns a separated dynamic pool of sequential tasks.
pub fn machine() -> Machine {
    Machine {
        layout: None,
        name: syn::parse_quote!(tasks),
        items: 1,
        root: LevelNode {
            shared_bytes: None,
            name: syn::parse_quote!(host),
            collaborative: false,
            dynamic: false,
            width: None,
            children: vec![LevelNode {
                shared_bytes: None,
                name: syn::parse_quote!(pool),
                collaborative: false,
                dynamic: true,
                width: None,
                children: vec![LevelNode {
                    shared_bytes: None,
                    name: syn::parse_quote!(task),
                    collaborative: false,
                    dynamic: false,
                    width: None,
                    children: vec![],
                }],
            }],
        },
    }
}

/// Native runtime tokens are valid at host, pool publication and task levels.
///
/// Task chunks own their native values; no device codec or provider is implied.
pub fn catalog() -> cq_lang_sequence::Catalog {
    let mut catalog = cq_sequence_pull::catalog();
    let levels = vec![
        vec!["host".into()],
        vec!["host".into(), "pool".into()],
        vec!["host".into(), "pool".into(), "task".into()],
    ];
    for token in &mut catalog.providers {
        token.levels = levels.clone();
    }
    for token in &mut catalog.codecs {
        token.levels = levels.clone();
    }
    for token in &mut catalog.canons {
        token.levels = levels.clone();
    }
    catalog
}

/// Translate checked Sequence into an owned, deterministically concatenated Vec.
// Formal rule T-Tasks: fold the checked term using ordered pool partitions and
// local buffers; canonicalize parts before the global canonical boundary.
pub fn translate(program: &Program, analysis: &Analysis) -> syn::Result<syn::Expr> {
    let mut tiles = std::collections::BTreeMap::new();
    let mut publications = std::collections::BTreeSet::new();
    for definition in &program.sequence.definitions {
        let name = syn::ext::IdentExt::unraw(&definition.name).to_string();
        let mut nodes = Vec::new();
        tile_nodes(&definition.term, &mut nodes);
        if let Some(kinds) = analysis.tilings.get(&name) {
            for (node, kind) in nodes.into_iter().zip(kinds) {
                tiles.insert(node, kind);
            }
        }
        if matches!(definition.term, Term::Store { .. }) {
            publications.insert(&definition.term as *const Term);
        }
    }
    cq_sequence_pull::translate_with(
        program,
        analysis,
        |term, emitter, child, output, definition, _| {
            let node = term as *const Term;
            let level = if publications.contains(&node) {
                analysis.levels.get(definition)
            } else {
                analysis
                    .dispatches
                    .iter()
                    .find(|dispatch| dispatch.inner_stores.iter().any(|name| name == definition))
                    .map(|dispatch| &dispatch.producer_level)
                    .or_else(|| analysis.levels.get(definition))
            };
            if matches!(term, Term::Group(_, _))
                && level.is_some_and(|path| path.as_slice() != ["host"])
            {
                return Err(syn::Error::new_spanned(
                    program,
                    "BE-Group: grouped folds must execute at the host join",
                ));
            }
            let active = if matches!(term, Term::Tile { .. }) {
                tiles
                    .get(&node)
                    .is_some_and(|kind| **kind == Tiling::Distribute)
            } else {
                level.is_some_and(|path| path.last().is_some_and(|name| name == "pool"))
            };
            if !active {
                return Ok(());
            }
            if let Some(input) = child {
                let lifted = emitter.lift(input.clone(), input.depth, &mut |emitter, input| {
                    let mut result = input.clone();
                    match term {
                        Term::Tile { n, .. } => {
                            tile(*n, &input, &mut result);
                            result.depth += 1;
                        }
                        Term::Store { capacity, .. } => {
                            store(*capacity, &input, &mut result);
                            result.stored = true;
                        }
                        Term::Canon(_) => canon(emitter, &input, &mut result)?,
                        _ => return Ok(input),
                    }
                    Ok(result)
                })?;
                if matches!(
                    term,
                    Term::Tile { .. } | Term::Store { .. } | Term::Canon(_)
                ) {
                    *output = lifted;
                }
            }
            Ok(())
        },
    )
}

fn tile(n: usize, input: &Value, output: &mut Value) {
    let rows = &input.rows;
    output.rows = quote! {{
        use ::rayon::prelude::*;
        let __cq_task_input = (#rows).collect::<::std::vec::Vec<_>>();
        __cq_task_input.par_chunks(#n)
            .map(|__cq_chunk| __cq_chunk.to_vec())
            .collect::<::std::vec::Vec<_>>()
            .into_iter()
    }};
}

fn store(capacity: Option<usize>, input: &Value, output: &mut Value) {
    let rows = &input.rows;
    let capacity_check = capacity.map(|capacity| {
        quote! {
            ::core::assert!(__cq_task_input.len() <= #capacity, "Sequence store capacity exceeded");
        }
    });
    output.rows = quote! {{
        use ::rayon::prelude::*;
        let __cq_task_input = (#rows).collect::<::std::vec::Vec<_>>();
        #capacity_check
        let __cq_parts = __cq_task_input.into_par_iter()
            .chunks(1024)
            .collect::<::std::vec::Vec<_>>();
        __cq_parts.into_iter().flatten()
    }};
}

fn canon(emitter: &mut Emitter<'_>, input: &Value, output: &mut Value) -> syn::Result<()> {
    let rows = &input.rows;
    let mut part = input.clone();
    part.rows = quote! { __cq_chunk.iter().cloned() };
    part.stored = false;
    let local = emitter.canon(part)?.rows;
    let mut prefiltered = input.clone();
    prefiltered.rows = quote! {{
        use ::rayon::prelude::*;
        let __cq_task_input = (#rows).collect::<::std::vec::Vec<_>>();
        let __cq_parts = __cq_task_input.par_chunks(1024)
            .map(|__cq_chunk| (#local).collect::<::std::vec::Vec<_>>())
            .collect::<::std::vec::Vec<_>>();
        __cq_parts.into_iter().flatten()
    }};
    prefiltered.stored = false;
    *output = emitter.canon(prefiltered)?;
    Ok(())
}

fn tile_nodes(term: &Term, output: &mut Vec<*const Term>) {
    if matches!(term, Term::Tile { .. }) {
        output.push(term as *const Term);
    }
    match term {
        Term::Map(_, inner)
        | Term::Compact(_, inner)
        | Term::Flatten(inner)
        | Term::Fold(_, inner)
        | Term::Group(_, inner)
        | Term::Canon(inner)
        | Term::Store { inner, .. }
        | Term::Tile { inner, .. }
        | Term::Lookup { inner, .. } => tile_nodes(inner, output),
        Term::Choose(arms) => {
            for (_, arm) in arms {
                tile_nodes(arm, output);
            }
        }
        Term::Union(arms) => {
            for arm in arms {
                tile_nodes(arm, output);
            }
        }
        Term::Rows(_) | Term::Ref(_) => {}
    }
}
