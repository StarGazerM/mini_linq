//! Sequence interpretation for an ordered, dynamically scheduled Rayon task pool.
//!
//! Generated expressions require `rayon` and `cq-storage` in the consumer. The
//! caller supplies facts checked against [`machine`]; translation never repeats
//! analysis. Host and task-local constructors reuse the sequential interpreter.
//! Pool chunks own their buffers and publish in chunk order at the join. Rayon
//! propagates a task panic to that join rather than publishing partial output.
//! Grouped folds join canonical valuations before parallel keyed aggregation.

use cq_lang_sequence::{Analysis, LevelNode, Machine, Program, Term, Tiling};
use quote::{format_ident, quote};

/// Pool scheduling parameters, independent of the checked partition width.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// `None` uses the calling Rayon's pool (including RAYON_NUM_THREADS).
    pub threads: Option<usize>,
    /// Contiguous driver rows owned by one task.
    pub grain: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self { threads: None, grain: 1024 }
    }
}

/// The host owns a separated dynamic pool of sequential tasks.
pub fn machine() -> Machine {
    machine_with(Config::default())
}

pub fn machine_with(config: Config) -> Machine {
    assert!(config.grain > 0, "task grain must be positive");
    assert!(config.threads != Some(0), "task thread count must be positive");
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
                width: Some(config.grain),
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

/// Translate checked partitions into joined, owned output.
// Formal rule T-Tasks: defer the entire partition-local iterator pipeline into
// the worker closure, then join before any global canonicalization/publication.
pub fn translate(program: &Program, analysis: &Analysis) -> syn::Result<syn::Expr> {
    translate_with(program, analysis, Config::default())
}

pub fn translate_with(
    program: &Program,
    analysis: &Analysis,
    config: Config,
) -> syn::Result<syn::Expr> {
    let mut tiles = std::collections::BTreeMap::new();
    for definition in &program.sequence.definitions {
        let name = syn::ext::IdentExt::unraw(&definition.name).to_string();
        let mut nodes = Vec::new();
        tile_nodes(&definition.term, &mut nodes);
        if let Some(kinds) = analysis.tilings.get(&name) {
            for (node, kind) in nodes.into_iter().zip(kinds) {
                tiles.insert(node, kind);
            }
        }
    }
    // A distributing tile binds a symbolic one-part iterator. Row adapters
    // compose beneath it unchanged; the paired flatten or pool store discharges
    // the binding into a Rayon closure.
    let mut partitions = Vec::new();
    let mut expression = cq_sequence_pull::translate_with(
        program,
        analysis,
        |term, _emitter, child, output, _, _| {
            if let Term::Tile { n, .. } = term
                && tiles.get(&(term as *const Term))
                    .is_some_and(|kind| **kind == Tiling::Distribute)
            {
                let input = child.expect("checked tile");
                if input.depth != 0 {
                    return Err(syn::Error::new_spanned(program, "Tasks distributing driver must be flat"));
                }
                let binding = format_ident!("__cq_task_driver_{}", partitions.len());
                partitions.push((binding.clone(), input.rows.clone(), *n));
                output.rows = quote!(::core::iter::once(#binding.iter().cloned()));
            } else if (matches!(term, Term::Flatten(_)) && output.depth == 0)
                || (matches!(term, Term::Store { .. }) && output.depth == 1)
            {
                if let Some((binding, source, grain)) = partitions.pop() {
                    let nested = output.depth == 1;
                    let rows = &output.rows;
                    let local = if nested {
                        quote!((#rows).flatten().collect::<::std::vec::Vec<_>>())
                    } else {
                        quote!((#rows).collect::<::std::vec::Vec<_>>())
                    };
                    let joined = if nested {
                        quote!(__cq_parts.into_iter().map(::std::vec::Vec::into_iter))
                    } else {
                        quote!(__cq_parts.into_iter().flatten())
                    };
                    output.rows = quote!({
                        use ::rayon::prelude::*;
                        let __cq_driver = (#source).collect::<::std::vec::Vec<_>>();
                        let __cq_parts = __cq_driver.par_chunks(#grain)
                            .map(|#binding| #local)
                            .collect::<::std::vec::Vec<_>>();
                        #joined
                    });
                }
            }
            Ok(())
        },
    )?;
    if !partitions.is_empty() {
        return Err(syn::Error::new_spanned(program, "Tasks partition lacks a joined flatten or pool store"));
    }
    if let Some(threads) = config.threads {
        expression = syn::parse_quote!({
            ::rayon::ThreadPoolBuilder::new().num_threads(#threads).build()
                .expect("create CQ task pool").install(|| #expression)
        });
    }
    Ok(syn::parse_quote!((#expression).into_iter()))
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
