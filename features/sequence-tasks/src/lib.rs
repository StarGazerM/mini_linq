//! Sequence interpretation for an ordered, dynamically scheduled Rayon task pool.
//!
//! Generated expressions require `rayon` and `cq-storage` in the consumer. The
//! caller supplies facts checked against [`machine`]; translation never repeats
//! analysis. Host and task-local constructors reuse the sequential interpreter.
//! Pool chunks own their buffers and publish in chunk order at the join. Rayon
//! propagates a task panic to that join rather than publishing partial output.
//! Key-disjoint partitions support local canon and keyed folds; global monoids
//! combine task-local partials at the host, never as a pool collective.
//!
//! Profiling retains the same tile/partition/Group interpretation. Operator
//! spans accumulate all worker invocations (and can overlap); `TASKS_BUILD_WALL`
//! measures host setup/publication and `TASKS_QUERY_WALL` final joined answer
//! evaluation. These disjoint wall phases include upstream work and are not an
//! exclusive index-insertion split. Deferred builds belong to the phase that
//! demands them. Runtime task contexts merge primitive worker costs at joins.

use cq_lang_sequence::{Analysis, LevelNode, Machine, Program, Term, Tiling};
use quote::{format_ident, quote};

/// Pool scheduling parameters, independent of the checked partition width.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// `None` uses the calling Rayon's pool (including RAYON_NUM_THREADS).
    pub threads: Option<usize>,
    /// Contiguous driver rows owned by one task.
    pub grain: usize,
    /// Logical partition count fixed before execution, independent of workers.
    pub partitions: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self { threads: None, grain: 1024, partitions: 64 }
    }
}

pub mod combine;
mod keyed;

/// The host owns a separated dynamic pool with 64 logical partitions.
///
/// This fixed planning width is independent of the Rayon worker count.
pub fn machine() -> Machine {
    machine_with(Config::default())
}

pub fn machine_with(config: Config) -> Machine {
    assert!(config.grain > 0, "task grain must be positive");
    assert!(config.threads != Some(0), "task thread count must be positive");
    machine_with_width(config.partitions)
}

/// Build a task machine with a fixed positive logical partition count.
pub fn machine_with_width(parts: usize) -> Machine {
    assert!(parts > 0, "task machine partition count must be positive");
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
                width: Some(parts),
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
    // Shared reads must not capture the serial lazy Rc/RefCell providers.
    catalog.providers.retain(|token| !matches!(&token.ty, syn::Type::Path(path)
        if path.path.segments.last().is_some_and(|segment|
            segment.ident == "HashIndex" || segment.ident == "Trie")));
    catalog.providers.insert(0, cq_lang_sequence::ProviderToken {
        ty: syn::parse_quote!(::cq_storage::sharded::Index<_, _>),
        capabilities: vec![cq_lang_sequence::Capability::Exact, cq_lang_sequence::Capability::Membership],
        levels: levels.clone(),
        elements: None,
    });
    // Hash sharding retains exact probes, but not a shard's global ordering.
    // Providers are published at the pool join and read by sequential tasks.
    let task = vec![vec!["host".into(), "pool".into(), "task".into()]];
    use cq_lang_sequence::{Capability, ProviderToken};
    for (ty, capabilities) in [
        (
            syn::parse_quote!(::cq_storage::sharded::Sharded<::cq_storage::HashSet<_>>),
            vec![Capability::Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::sharded::Sharded<::std::collections::HashSet<_>>),
            vec![Capability::Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::sharded::Sharded<::std::collections::HashSet<_, _>>),
            vec![Capability::Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::sharded::Sharded<::cq_storage::sharded::Index<_, _>>),
            vec![Capability::Exact, Capability::Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::sharded::Sharded<::cq_storage::SortedRun<_, _>>),
            vec![Capability::Exact, Capability::Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::sharded::Sharded<::cq_storage::SortedRun<_, _, _>>),
            vec![Capability::Exact, Capability::Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::sharded::Sharded<::std::collections::BTreeMap<_, ::std::vec::Vec<_>>>),
            vec![Capability::Exact, Capability::Membership],
        ),
    ] {
        let ty: syn::Type = ty;
        // A projection router changes partition placement, not capabilities.
        let mut routed_ty: syn::Type = ty.clone();
        if let syn::Type::Path(path) = &mut routed_ty {
            if let syn::PathArguments::AngleBracketed(args) =
                &mut path.path.segments.last_mut().unwrap().arguments
            {
                args.args.push(syn::parse_quote!(_));
            }
        }
        catalog.providers.push(ProviderToken {
            ty: routed_ty,
            capabilities: capabilities.clone(),
            levels: task.clone(),
            elements: None,
        });
        catalog.providers.push(ProviderToken {
            ty,
            capabilities,
            levels: task.clone(),
            elements: None,
        });
    }
    for token in &mut catalog.codecs {
        token.levels = levels.clone();
    }
    for token in &mut catalog.canons {
        token.levels = levels.clone();
    }
    catalog
}

/// Select native shareable representations through checked Structures rewrites.
pub fn select_providers(
    source: &Program, catalog: &cq_lang_sequence::Catalog,
) -> syn::Result<Program> {
    let mut target = source.clone();
    target.structures =
        cq_sequence_rewrite_passes::select_from_catalog(&source.structures, catalog)?;
    // A legal authored provider may supply stronger capabilities needed by an
    // authored tile (for example range cursors). Retarget only unavailable
    // families; do not downgrade an existing native representation.
    for (original, selected) in source.structures.structures.entries.iter()
        .zip(&mut target.structures.structures.entries)
    {
        if let (
            cq_lang_sequence::structures::Structure::Built { keyed: original, .. },
            cq_lang_sequence::structures::Structure::Built { keyed: selected, .. },
        ) = (&original.structure, &mut selected.structure)
            && catalog.providers.iter().any(|token|
                cq_sequence_plan::matches_type(&token.ty, &original.ty))
        {
            selected.ty = original.ty.clone();
        }
    }
    cq_lang_sequence::structures::check_rewrite(&source.structures, &target.structures)?;
    Ok(target)
}

/// Translate checked partitions into joined, owned output.
// Formal rule T-Tasks: defer the entire partition-local iterator pipeline into
// the worker closure, then join before any global canonicalization/publication.
pub fn translate(program: &Program, analysis: &Analysis) -> syn::Result<syn::Expr> {
    translate_with(program, analysis, Config::default())
}

/// Instrument the same checked Tasks interpretation, including worker work.
pub fn translate_profile(program: &Program, analysis: &Analysis) -> syn::Result<syn::Expr> {
    translate_profile_with(program, analysis, Config::default())
}

/// Profile a target with explicit scheduling parameters.
pub fn translate_profile_with(
    program: &Program,
    analysis: &Analysis,
    config: Config,
) -> syn::Result<syn::Expr> {
    translate_mode(program, analysis, config, true)
}

pub fn translate_with(
    program: &Program,
    analysis: &Analysis,
    config: Config,
) -> syn::Result<syn::Expr> {
    translate_mode(program, analysis, config, false)
}

fn translate_mode(
    program: &Program,
    analysis: &Analysis,
    config: Config,
    profile: bool,
) -> syn::Result<syn::Expr> {
    let mut tiles = std::collections::BTreeMap::new();
    let mut partition_inputs = std::collections::BTreeSet::new();
    for definition in &program.sequence.definitions {
        keyed::ancestors(&definition.term, &mut partition_inputs);
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
    let translate = if profile {
        cq_sequence_pull::translate_profile_with
    } else {
        cq_sequence_pull::translate_with
    };
    let mut expression = translate(
        program,
        analysis,
        |term: &Term, emitter: &mut cq_sequence_pull::Emitter<'_>,
         child: Option<&cq_sequence_pull::Value>, output: &mut cq_sequence_pull::Value,
         owner: &str, _: usize| {
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
            } else if let Term::PartitionK { keys, parts, .. } = term {
                let input = child.expect("checked key partition");
                output.rows = keyed::partition(emitter, input, keys, *parts, config.grain)?;
            } else if matches!(term, Term::Canon(_))
                && child.is_some_and(|input| input.depth == 1)
            {
                output.rows = keyed::canon(emitter, child.unwrap())?;
            } else if let Term::Group(name, _) = term
                && child.is_some_and(|input| input.depth == 1)
            {
                let input = child.unwrap();
                let result = cq_lang_sequence::structures::symbol_name(name);
                let global = matches!(
                    &emitter.context().physical[&result].operator,
                    cq_lang_sequence::structures::physical::Operator::Group(group)
                        if group.keys.is_empty()
                );
                output.rows = if global {
                    keyed::global(emitter, input, name, partitions.pop())?
                } else {
                    keyed::group(emitter, input, name)?
                };
                output.depth = usize::from(!global);
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
                    let local = task_scope(emitter, local);
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
            if matches!(term, Term::Store { .. }) && partition_inputs.contains(owner) {
                let label = format!("{owner}.rows");
                emitter.context_mut().profile_label(
                    &label, "ROWS_BUILD", "", output.result.iter().cloned().collect(),
                );
                let rows = emitter.context().profile_inserting(&label, output.rows.clone());
                let build = emitter.context().profile_build(
                    &label, quote!((#rows).collect::<::std::vec::Vec<_>>()),
                );
                let binding = format_ident!("__cq_key_source_{}", owner);
                emitter.context_mut().statements.push(syn::parse_quote!(
                    let #binding = #build;
                ));
                emitter.context_mut().bound = true;
                emitter.mark_target_published(owner);
                output.rows = quote!(#binding.iter().cloned());
            } else if matches!(term, Term::Store { .. }) && output.depth == 1
                && let Some(keys) = keyed::partition_keys(term, program)
                && let Some(result) = output.result.clone()
                && analysis.structures.built.get(&result).is_some_and(|providers|
                    providers.iter().any(|(_, ty)| matches!(ty, syn::Type::Path(path)
                        if path.path.segments.last().is_some_and(|segment| segment.ident == "Sharded"))))
            {
                let label = format!("{owner}.parts");
                emitter.context_mut().profile_label(
                    &label, "PARTITION_BUILD", "", vec![result.clone()],
                );
                let rows = &output.rows;
                let part = emitter.context().profile_inserting(&label, quote!(__cq_part));
                let build = emitter.context().profile_build(
                    &label,
                    quote!((#rows).map(|__cq_part|
                        (#part).collect::<::std::vec::Vec<_>>())
                        .collect::<::std::vec::Vec<_>>()),
                );
                let binding = format_ident!("__cq_key_parts_{}", owner);
                emitter.context_mut().statements.push(syn::parse_quote!(
                    let #binding = #build;
                ));
                let keys = keys.iter().map(cq_lang_sequence::structures::symbol_name).collect::<Vec<_>>();
                let repr = emitter.context_mut().materialize_partitioned(
                    &result, quote!(#binding), &keys,
                )?;
                emitter.context_mut().reprs.insert(result, repr);
                emitter.context_mut().bound = true;
                emitter.mark_target_published(owner);
                output.rows = quote!(#binding.iter().cloned().map(::std::vec::Vec::into_iter));
            }
            Ok(())
        },
    )?;
    if !partitions.is_empty() {
        return Err(syn::Error::new_spanned(program, "Tasks partition lacks a joined flatten or pool store"));
    }
    if let Some(threads) = config.threads {
        expression = if profile {
            syn::parse_quote!({
                let __cq_task_context = ::cq_storage::counters::TaskContext::capture();
                ::rayon::ThreadPoolBuilder::new().num_threads(#threads).build()
                    .expect("create CQ task pool")
                    .install(|| __cq_task_context.run(|| #expression))
            })
        } else {
            syn::parse_quote!({
                ::rayon::ThreadPoolBuilder::new().num_threads(#threads).build()
                    .expect("create CQ task pool").install(|| #expression)
            })
        };
    }
    Ok(syn::parse_quote!((#expression).into_iter()))
}

/// Capture the caller's primitive-counter epoch before entering Rayon. Operator
/// handles already share their own state; primitive counters stay task-local
/// on the hot path and merge once when this synchronous task returns.
pub(crate) fn task_scope(
    emitter: &cq_sequence_pull::Emitter<'_>,
    body: proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    if emitter.context().profile.is_some() {
        let profile = emitter.context().profile_handle();
        quote!(#profile.task(|| #body))
    } else {
        body
    }
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
        | Term::PartitionK { inner, .. }
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
