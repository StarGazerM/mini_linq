//! Sequence interpretation for an ordered, dynamically scheduled Rayon task pool.
//!
//! Generated expressions require `rayon`, `cq-storage` and this crate (its
//! [`combine`] runtime) in the consumer. The
//! caller supplies facts checked against [`machine`]; translation never repeats
//! analysis. Host and task-local constructors reuse the sequential interpreter.
//! Shared indexed dispatch keeps Rayon plumbing out of generated pipelines.
//! Pool chunks publish owned buffers in order; task panics propagate to the join.
//! Key-disjoint partitions support local canon and keyed folds; global monoids
//! combine task-local partials at the host, never as a pool collective.
//!
//! Profiling retains the same tile/partition/Group interpretation. Operator
//! spans accumulate all worker invocations (and can overlap); `TASKS_BUILD_WALL`
//! measures host setup/publication and `TASKS_QUERY_WALL` final joined answer
//! evaluation. These disjoint wall phases include upstream work and are not an
//! exclusive index-insertion split. Deferred builds belong to the phase that
//! demands them. Runtime task contexts merge primitive worker costs at joins.

use cq_lang_sequence::{Analysis, LevelNode, Machine, Program, Publication, Term, Tiling};
use quote::{format_ident, quote};

/// Pool scheduling parameters, independent of the checked partition width.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// `None` uses the calling Rayon's pool (including RAYON_NUM_THREADS).
    pub threads: Option<usize>,
    /// Rows per task-local key-routing chunk; contiguous tile sizes are planned.
    pub grain: usize,
    /// Logical partition count fixed before execution, independent of workers.
    pub partitions: usize,
    /// Published rows at which the planner records routed-provider construction.
    pub key_partition_threshold: usize,
    /// Rows at which cache-local provider partitions also apply with one worker.
    /// The effective cutoff is at least `key_partition_threshold`.
    pub key_partition_cache_threshold: usize,
}

impl Default for Config {
    fn default() -> Self {
        // A routing task fills about 1K rows per logical bucket, avoiding a
        // tiny allocation for every (1K-row chunk, partition) pair.
        Self {
            threads: None,
            grain: 64 * 1024,
            partitions: 64,
            key_partition_threshold: 64 * 1024,
            // Measured row cutoff, not a universal cache-capacity guarantee.
            key_partition_cache_threshold: 6 * 1024 * 1024,
        }
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
    assert!(
        config.threads != Some(0),
        "task thread count must be positive"
    );
    assert!(
        config.key_partition_threshold > 0,
        "provider partition threshold must be positive"
    );
    assert!(
        config.key_partition_cache_threshold > 0,
        "provider partition cache threshold must be positive"
    );
    let mut machine = machine_with_width(config.partitions);
    machine.key_partition_threshold = config.key_partition_threshold;
    machine.key_partition_cache_threshold = config.key_partition_cache_threshold;
    machine
}

/// Build a task machine with a fixed positive logical partition count.
pub fn machine_with_width(parts: usize) -> Machine {
    assert!(parts > 0, "task machine partition count must be positive");
    Machine {
        layout: None,
        name: syn::parse_quote!(tasks),
        items: 1,
        key_partition_threshold: Config::default().key_partition_threshold,
        key_partition_cache_threshold: Config::default().key_partition_cache_threshold,
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
        // Lazy Rc/RefCell providers remain legal at host, but cannot be
        // captured by shared task readers. Selection upgrades them when its
        // checked retention/retyping rewrite is applicable.
        if !matches!(&token.ty, syn::Type::Path(path)
            if path.path.segments.last().is_some_and(|segment|
                segment.ident == "HashIndex" || segment.ident == "Trie" || segment.ident == "KeySet"))
        {
            token.levels = levels.clone();
        }
    }
    catalog.providers.insert(
        0,
        cq_lang_sequence::ProviderToken {
            ty: syn::parse_quote!(::cq_storage::sharded::Index<_, _>),
            capabilities: vec![
                cq_lang_sequence::Capability::Exact,
                cq_lang_sequence::Capability::Membership,
            ],
            levels: levels.clone(),
            elements: None,
        },
    );
    // Hash sharding retains exact probes, but not a shard's global ordering.
    // The synchronous pool join publishes native memory to host and task readers.
    use cq_lang_sequence::{Capability, ProviderToken};
    for (ty, capabilities) in [
        (
            syn::parse_quote!(::cq_storage::sharded::Sharded<::cq_storage::HashSet<_>>),
            vec![Capability::Membership],
        ),
        (
            syn::parse_quote!(
                ::cq_storage::sharded::Sharded<::cq_storage::AdaptiveKeySet<_, _, _>>
            ),
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
            syn::parse_quote!(
                ::cq_storage::sharded::Sharded<::std::collections::BTreeMap<_, ::std::vec::Vec<_>>>
            ),
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
            levels: levels.clone(),
            elements: None,
        });
        catalog.providers.push(ProviderToken {
            ty,
            capabilities,
            levels: levels.clone(),
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
    source: &Program,
    catalog: &cq_lang_sequence::Catalog,
) -> syn::Result<Program> {
    let analysis = cq_lang_sequence::contract::check(source)?;
    let mut admitted: std::collections::BTreeSet<_> = source
        .structures
        .structures
        .entries
        .iter()
        .map(|entry| cq_lang_sequence::structures::symbol_name(&entry.name))
        .collect();
    for obligation in &analysis.obligations {
        let cq_lang_sequence::Obligation::Provider {
            provider,
            capability,
            ty,
            ..
        } = obligation
        else {
            continue;
        };
        if !catalog.providers.iter().any(|token| {
            cq_sequence_plan::matches_type(&token.ty, ty)
                && token.capabilities.contains(capability)
                && !token.levels.is_empty()
                && token.elements.as_ref().is_none_or(|accepted| {
                    analysis.structures.layouts[provider].iter().all(|column| {
                        accepted.iter().any(|pattern| {
                            cq_sequence_plan::matches_type(
                                pattern,
                                &analysis.structures.physical.rows[provider].types[column],
                            )
                        })
                    })
                })
        }) {
            admitted.remove(provider);
        }
    }
    // Instantiate supported authored families before checked selection. A
    // transient retype can invalidate obligations such as exact join filters.
    let mut native = catalog.clone();
    native
        .providers
        .retain(|token| token.levels.iter().any(|level| level.len() > 1));
    for entry in &source.structures.structures.entries {
        let ty = match &entry.structure {
            cq_lang_sequence::structures::Structure::Built { keyed, .. }
            | cq_lang_sequence::structures::Structure::Host { keyed, .. } => &keyed.ty,
            cq_lang_sequence::structures::Structure::KeySet { ty, .. }
            | cq_lang_sequence::structures::Structure::Selection { ty, .. } => ty,
            cq_lang_sequence::structures::Structure::JoinFilter { filter, .. } => &filter.ty,
            _ => continue,
        };
        if let Some(token) = catalog.providers.iter().find(|token| {
            token.levels.iter().any(|level| level.len() > 1)
                && cq_sequence_plan::matches_type(&token.ty, ty)
        }) {
            let mut token = token.clone();
            token.ty = ty.clone();
            native.providers.push(token);
        }
    }
    let mut target = source.clone();
    target.structures = cq_sequence_rewrite_passes::select_from_catalog_preserving(
        &source.structures,
        &native,
        &admitted,
    )?;
    for (old, retained) in source
        .structures
        .physical
        .program
        .definitions
        .iter()
        .zip(&target.structures.physical.program.definitions)
    {
        if old.mode == retained.mode {
            continue;
        }
        // Catalog retention changes a host rename's mode, but Sequence was
        // lowered before selection. Its publication must become explicit too.
        if let Some(definition) = target
            .sequence
            .definitions
            .iter_mut()
            .find(|definition| definition.name == retained.name)
        {
            if !matches!(definition.term, Term::Store { .. }) {
                let inner = std::mem::replace(&mut definition.term, Term::Union(Vec::new()));
                definition.term = Term::Store {
                    capacity: None,
                    inner: Box::new(inner),
                };
            }
        } else {
            target.sequence.definitions.insert(
                0,
                cq_lang_sequence::Definition {
                    name: retained.name.clone(),
                    term: Term::Store {
                        capacity: None,
                        inner: Box::new(Term::Rows(retained.name.clone())),
                    },
                },
            );
            if let Some(graded) = &mut target.graded {
                graded.grades.push(cq_lang_sequence::Grade {
                    name: retained.name.clone(),
                    level: vec![syn::parse_quote!(host)],
                });
            }
        }
    }
    cq_lang_sequence::check_rewrite(source, &target)?;
    Ok(target)
}

/// Translate checked partitions into joined, owned output.
// Formal rule T-Tasks: shared indexed task dispatch publishes ordered task-local
// buffers before monoidal reductions and local then global canonicalization.
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
    let mut host_tiles = std::collections::BTreeSet::new();
    for definition in &program.sequence.definitions {
        let name = syn::ext::IdentExt::unraw(&definition.name).to_string();
        let mut nodes = Vec::new();
        tile_nodes(&definition.term, &mut nodes);
        if let Some(kinds) = analysis.tilings.get(&name) {
            for (index, (node, kind)) in nodes.into_iter().zip(kinds).enumerate() {
                tiles.insert(node, kind);
                if analysis
                    .host_tiles
                    .get(&name)
                    .and_then(|inputs| inputs.get(index))
                    .is_some_and(Option::is_some)
                {
                    host_tiles.insert(node);
                }
            }
        }
    }
    // A distributing tile binds a symbolic one-part iterator. Streaming row
    // adapters compose through its paired flatten and named pipes. A Store or
    // non-distributive constructor joins the task buffers before proceeding.
    let mut partitions = Vec::new();
    let mut stored_sources = std::collections::BTreeMap::<String, proc_macro2::TokenStream>::new();
    let mut deferred_sources =
        std::collections::BTreeMap::<String, proc_macro2::TokenStream>::new();
    let mut provider_covers =
        std::collections::BTreeMap::<*const Term, keyed::ProviderCover>::new();
    let mut named_covers = std::collections::BTreeMap::<String, keyed::ProviderCover>::new();
    let mut immutable_sources = std::collections::BTreeSet::<String>::new();
    let mut next_driver = 0usize;
    let mut needs_workers = false;
    let translate = if profile {
        cq_sequence_pull::translate_profile_with
    } else {
        cq_sequence_pull::translate_with
    };
    let mut expression = translate(
        program,
        analysis,
        syn::parse_quote!(__cq_task_workers),
        |term: &Term,
         emitter: &mut cq_sequence_pull::Emitter<'_>,
         child: Option<&cq_sequence_pull::Value>,
         output: &mut cq_sequence_pull::Value,
         owner: &str,
         _: usize| {
            if let Term::Choose(arms) = term {
                needs_workers |= arms
                    .iter()
                    .any(|(guard, _)| matches!(guard, cq_lang_sequence::Guard::WorkersLt(_)));
            }
            if let Term::Tile { n, .. } = term
                && tiles
                    .get(&(term as *const Term))
                    .is_some_and(|kind| **kind == Tiling::Distribute)
            {
                let input = child.expect("checked tile");
                if input.depth != 0 {
                    return Err(syn::Error::new_spanned(
                        program,
                        "Tasks distributing driver must be flat",
                    ));
                }
                let binding = format_ident!("__cq_task_driver_{}", next_driver);
                next_driver += 1;
                let context = emitter.context();
                let host_result = input.result.as_ref().filter(|result| {
                    host_tiles.contains(&(term as *const Term))
                        && context.physical_analysis.hosts.contains_key(*result)
                        && !context.analysis.selections.contains_key(*result)
                        && matches!(&context.physical[*result].operator,
                        cq_lang_sequence::structures::physical::Operator::Rename(rename)
                            if !context.carries_positions(result) || rename.constants.is_none())
                });
                let positional = host_result.filter(|result| {
                    context.carries_positions(result) || context.join_filter_ids(result).is_some()
                });
                if let Some(result) = positional {
                    let host = cq_print::Context::positional_host(
                        &context.physical_analysis.hosts[result],
                    );
                    let ids = quote!(::cq_storage::PositionalRows::position_range(
                        #host, #binding * #n..(#binding + 1) * #n));
                    let ids = context
                        .join_filter_ids_from(result, ids.clone())
                        .unwrap_or(ids);
                    let rows = context.profile_stream(result, context.positional_rows(result, ids));
                    partitions.push(TaskTile {
                        binding,
                        prepare: quote!(),
                        parts: quote!(::cq_sequence_tasks::combine::IndexedTiles::new(
                            ::cq_storage::PositionalRows::len(#host).div_ceil(#n), |i| i)),
                    });
                    output.rows = quote!(::core::iter::once(#rows));
                } else if let Some(result) = host_result {
                    let host = &context.hosts[&context.physical_analysis.hosts[result]].value;
                    let rows = context.profile_stream(
                        result,
                        context.host_scan_from(result, Some(quote!(#binding))),
                    );
                    partitions.push(TaskTile {
                        binding,
                        prepare: quote! {
                            use ::cq_sequence_tasks::combine::TileRows as _;
                            let __cq_driver = ::cq_sequence_tasks::combine::TileSource(#host);
                            let __cq_driver_ref = &&__cq_driver;
                        },
                        parts: quote!(__cq_driver_ref.tile_rows(#n)),
                    });
                    output.rows = quote!(::core::iter::once(#rows));
                } else {
                    let source = &input.rows;
                    partitions.push(TaskTile {
                        binding: binding.clone(),
                        prepare: quote!(let __cq_driver = (#source).collect::<::std::vec::Vec<_>>();),
                        parts: quote!(::cq_sequence_tasks::combine::IndexedTiles::new(
                            __cq_driver.len().div_ceil(#n),
                            |i| &__cq_driver[i * #n..((i + 1) * #n).min(__cq_driver.len())])),
                    });
                    output.rows = quote!(::core::iter::once(#binding.iter().cloned()));
                }
                output.partitioned = true;
            } else if let Term::PartitionK { keys, parts, inner } = term {
                let input = child.expect("checked key partition");
                let stored = match inner.as_ref() {
                    Term::Ref(name) => {
                        stored_sources.get(&cq_lang_sequence::structures::symbol_name(name))
                    }
                    _ => None,
                };
                let borrowed_host = match inner.as_ref() {
                    Term::Ref(name) => match analysis
                        .publications
                        .get(&cq_lang_sequence::structures::symbol_name(name))
                    {
                        Some(Publication::BorrowHost { result, .. })
                            if input.result.as_ref() == Some(result) =>
                        {
                            Some(result.as_str())
                        }
                        _ => None,
                    },
                    _ => None,
                };
                output.rows = keyed::partition(
                    emitter,
                    input,
                    keys,
                    *parts,
                    config.grain,
                    stored,
                    borrowed_host,
                )?;
            } else if matches!(term, Term::Canon(_)) && child.is_some_and(|input| input.depth == 1)
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
                let partial = emitter.is_partial_group(term);
                let tile = if partial {
                    partitions
                        .iter()
                        .position(|tile: &TaskTile| uses_binding(&input.rows, &tile.binding))
                        .map(|index| partitions.remove(index))
                } else {
                    None
                };
                output.rows = if partial && global {
                    keyed::global(emitter, input, name, tile)?
                } else if partial {
                    keyed::partial(emitter, input, name, tile, config.partitions)?
                } else {
                    keyed::group(emitter, input, name)?
                };
                output.depth = usize::from(!partial);
                output.partitioned = !partial && output.partitioned;
            } else if matches!(
                term,
                Term::Store { .. } | Term::Canon(_) | Term::Group(_, _) | Term::Fold(_, _)
            ) || program
                .sequence
                .definitions
                .last()
                .is_some_and(|definition| std::ptr::eq(&definition.term, term))
            {
                let input = if matches!(
                    term,
                    Term::Store { .. } | Term::Canon(_) | Term::Group(_, _) | Term::Fold(_, _)
                ) {
                    child.unwrap_or(output)
                } else {
                    &*output
                };
                if let Some(index) = partitions
                    .iter()
                    .position(|tile: &TaskTile| uses_binding(&input.rows, &tile.binding))
                {
                    let tile = partitions.remove(index);
                    let mut input = input.clone();
                    input.partitioned = false;
                    output.partitioned = false;
                    let nested = input.depth > 0;
                    let mut local = input.rows.clone();
                    if nested {
                        local = quote!((#local).flatten());
                    }
                    if matches!(term, Term::Store { .. })
                        && !nested
                        && program
                            .sequence
                            .definitions
                            .last()
                            .is_some_and(|definition| std::ptr::eq(&definition.term, term))
                        && let Some(result) = &output.result
                        && !analysis.structures.head_dedup.contains_key(result)
                    {
                        local = emitter.own_task_answer(result, local);
                    }
                    input.rows = join_tile(emitter, tile, local, nested);
                    output.rows = match term {
                        Term::Canon(_) => {
                            let depth = input.depth;
                            emitter
                                .lift(input, depth, &mut |emitter, rows| emitter.canon(rows))?
                                .rows
                        }
                        Term::Group(name, _) => {
                            let depth = input.depth;
                            emitter
                                .lift(input, depth, &mut |emitter, rows| emitter.group(name, rows))?
                                .rows
                        }
                        Term::Fold(monoid, _) => {
                            let depth = input.depth;
                            emitter
                                .lift(input, depth, &mut |emitter, rows| {
                                    emitter.fold(monoid, rows)
                                })?
                                .rows
                        }
                        _ => input.rows,
                    };
                }
            }
            if matches!(term, Term::Flatten(_))
                && output.depth == 0
                && !partitions
                    .iter()
                    .any(|tile| uses_binding(&output.rows, &tile.binding))
            {
                // A completed key cover is no longer a worker-local slice.
                // Symbolic distributing tiles remain partitioned until joined.
                output.partitioned = false;
            }
            // Preserve the recorded cover before Pull unifies Choose arms by
            // collecting rows. Only immutable flat publications may be replayed;
            // all alternatives must retain the same source and carrier.
            let cover = match term {
                Term::PartitionK { parts, inner, .. } => {
                    if let Term::Ref(name) = inner.as_ref()
                        && immutable_sources
                            .contains(&cq_lang_sequence::structures::symbol_name(name))
                        && let Some(input) = child.filter(|input| input.depth == 0)
                    {
                        Some(keyed::ProviderCover {
                            source: cq_lang_sequence::structures::symbol_name(name),
                            rows: input.rows.clone(),
                            result: input.result.clone(),
                            choice: keyed::CoverChoice::Partition {
                                parts: *parts,
                                rows: output.rows.clone(),
                            },
                        })
                    } else {
                        None
                    }
                }
                Term::Ref(name) => named_covers
                    .get(&cq_lang_sequence::structures::symbol_name(name))
                    .filter(|cover| cover.result == output.result)
                    .cloned(),
                Term::Choose(arms) => {
                    let covers = arms
                        .iter()
                        .map(|(_, arm)| provider_covers.get(&(arm as *const Term)))
                        .collect::<Option<Vec<_>>>();
                    if let Some(covers) = covers
                        && let Some(first) = covers.first()
                        && covers.iter().all(|cover| {
                            cover.source == first.source && cover.result == first.result
                        })
                    {
                        let mut cover = (*first).clone();
                        cover.choice = keyed::CoverChoice::Choose(
                            arms.iter()
                                .zip(covers)
                                .map(|((guard, _), cover)| {
                                    Ok((emitter.runtime_guard(guard)?, cover.choice.clone()))
                                })
                                .collect::<syn::Result<_>>()?,
                        );
                        Some(cover)
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(cover) = cover {
                provider_covers.insert(term as *const Term, cover.clone());
                if program
                    .sequence
                    .definitions
                    .iter()
                    .any(|definition| std::ptr::eq(&definition.term, term))
                    && analysis.results.get(owner) == cover.result.as_ref()
                {
                    named_covers.insert(owner.to_owned(), cover);
                }
            }
            let publication = analysis.publications.get(owner).filter(|_| {
                program
                    .sequence
                    .definitions
                    .iter()
                    .any(|definition| std::ptr::eq(&definition.term, term))
            });
            if matches!(term, Term::Store { .. })
                && output.depth == 0
                && let Some(Publication::BorrowHost { result, .. }) = publication
            {
                // An immutable base relation is already resident storage. The
                // checked Store publishes that borrow; it need not clone rows.
                output.rows = child.expect("checked base store").rows.clone();
                immutable_sources.insert(owner.to_owned());
                let scan = emitter.context().host_scan(result);
                emitter.publish_length(
                    owner,
                    quote!({
                        let __cq_length_rows = #scan;
                        let (__cq_minimum, __cq_maximum) = __cq_length_rows.size_hint();
                        if __cq_maximum == ::core::option::Option::Some(__cq_minimum) {
                            __cq_minimum
                        } else {
                            __cq_length_rows.count()
                        }
                    }),
                );
                emitter.mark_target_published(owner);
            } else if matches!(term, Term::Store { .. })
                && output.depth == 0
                && let Some(Publication::Alias { source }) = publication
            {
                assert!(
                    emitter.alias_store(owner, source),
                    "checked publication alias"
                );
                output.rows = child.expect("checked store alias").rows.clone();
                if immutable_sources.contains(source) {
                    immutable_sources.insert(owner.to_owned());
                }
                if analysis.results.get(owner) == child.and_then(|value| value.result.as_ref())
                    && let Some(binding) = stored_sources.get(source).cloned()
                {
                    stored_sources.insert(owner.to_owned(), binding);
                }
            } else if matches!(term, Term::Store { .. })
                && output.depth == 0
                && matches!(
                    publication,
                    Some(Publication::OwnRows { staged: true } | Publication::DeferredRows { .. })
                )
            {
                let label = format!("{owner}.rows");
                emitter.context_mut().profile_label(
                    &label,
                    "ROWS_BUILD",
                    "",
                    output.result.iter().cloned().collect(),
                );
                let binding = format_ident!("__cq_key_source_{}", owner);
                let source = output.rows.clone();
                let factory = format_ident!("__cq_key_factory_{}", owner);
                let staged = format_ident!("__cq_key_staged_{}", owner);
                let deferred = matches!(publication, Some(Publication::DeferredRows { .. }));
                let rows = emitter.context().profile_inserting(
                    &label,
                    if deferred {
                        quote!(#factory())
                    } else {
                        source.clone()
                    },
                );
                let build = emitter
                    .context()
                    .profile_build(&label, quote!((#rows).collect::<::std::vec::Vec<_>>()));
                let stored = if deferred {
                    // The checked single consumer may bypass this allocation,
                    // but a length guard or routed arm owns exactly one copy.
                    // Keep retention inside the initializer: merely publishing
                    // the source must neither evaluate it nor charge a copy.
                    emitter.context_mut().statements.push(syn::parse_quote!(
                        let #factory = || { #source };
                    ));
                    emitter.context_mut().statements.push(syn::parse_quote!(
                        let #binding = ::std::sync::OnceLock::<(
                            ::std::vec::Vec<_>, ::cq_storage::Retention
                        )>::new();
                    ));
                    emitter.context_mut().statements.push(syn::parse_quote!(
                        let #staged = || #binding.get_or_init(|| {
                            let __cq_rows = #build;
                            let __cq_retention = ::cq_storage::retain_rows(&__cq_rows);
                            (__cq_rows, __cq_retention)
                        });
                    ));
                    deferred_sources.insert(
                        owner.to_owned(),
                        quote!({
                            let __cq_staged = #binding.get();
                            __cq_staged.is_none().then(|| #factory()).into_iter().flatten()
                                .chain(__cq_staged.into_iter()
                                    .flat_map(|(__cq_rows, _)| __cq_rows.iter().cloned()))
                        }),
                    );
                    quote!(#staged().0)
                } else {
                    emitter.context_mut().statements.push(syn::parse_quote!(
                        let #binding = #build;
                    ));
                    emitter.context_mut().statements.push(syn::parse_quote!(
                        let _retain = ::cq_storage::retain_rows(&#binding);
                    ));
                    quote!(#binding)
                };
                // A publication may rehead an equivalent Physical result.
                // Reuse the raw slice only when its carrier is unchanged.
                if analysis.results.get(owner) == output.result.as_ref() {
                    stored_sources.insert(owner.to_owned(), stored.clone());
                }
                emitter.context_mut().bound = true;
                emitter.mark_target_published(owner);
                emitter.publish_length(owner, quote!((#stored).len()));
                output.rows = quote!((#stored).iter().cloned());
                immutable_sources.insert(owner.to_owned());
            } else if matches!(term, Term::Store { .. })
                && output.depth == 1
                && let Some(Publication::Parts { keys }) = publication
                && let Some(result) = output.result.clone()
            {
                let label = format!("{owner}.parts");
                emitter.context_mut().profile_label(
                    &label,
                    "PARTITION_BUILD",
                    "",
                    vec![result.clone()],
                );
                let cover = match term {
                    Term::Store { inner, .. } => provider_covers
                        .get(&(inner.as_ref() as *const Term))
                        .filter(|cover| cover.result == output.result),
                    _ => None,
                };
                let binding = format_ident!("__cq_key_parts_{}", owner);
                let repr = if let Some(cover) = cover {
                    let source = match analysis.publications.get(&cover.source) {
                        Some(Publication::DeferredRows { consumer }) if consumer == owner => {
                            &deferred_sources[&cover.source]
                        }
                        _ => &cover.rows,
                    };
                    let buffers = cover.choice.buffers(emitter, &label);
                    let build = emitter.context().profile_build(
                        &label,
                        quote!(::cq_storage::sharded::ProviderParts::new(|| #source, #buffers)),
                    );
                    emitter.context_mut().statements.push(syn::parse_quote!(
                        let #binding = #build;
                    ));
                    emitter.context_mut().statements.push(syn::parse_quote!(
                        let _retain_parts = #binding.routed().into_iter().flatten()
                            .map(::cq_storage::retain_rows).collect::<::std::vec::Vec<_>>();
                    ));
                    output.rows = quote!((0..#binding.part_count()).map(|__cq_index|
                        #binding.part(__cq_index)));
                    emitter.context_mut().materialize_provider_parts(
                        &result,
                        quote!(#binding),
                        keys,
                        &analysis.routed_providers[&result],
                    )
                } else {
                    let rows = &output.rows;
                    let part = emitter
                        .context()
                        .profile_inserting(&label, quote!(__cq_part));
                    let build = emitter.context().profile_build(
                        &label,
                        quote!((#rows).map(|__cq_part|
                            (#part).collect::<::std::vec::Vec<_>>())
                            .collect::<::std::vec::Vec<_>>()),
                    );
                    emitter.context_mut().statements.push(syn::parse_quote!(
                        let #binding = #build;
                    ));
                    emitter.context_mut().statements.push(syn::parse_quote!(
                        let _retain_parts = #binding.iter().map(::cq_storage::retain_rows)
                            .collect::<::std::vec::Vec<_>>();
                    ));
                    output.rows = quote!(#binding.iter().cloned().map(::std::vec::Vec::into_iter));
                    emitter.context_mut().materialize_partitioned(
                        &result,
                        quote!(#binding),
                        keys,
                        &analysis.routed_providers[&result],
                    )?
                };
                emitter.context_mut().reprs.insert(result, repr);
                emitter.context_mut().bound = true;
                emitter.mark_target_published(owner);
            }
            Ok(())
        },
    )?;
    if needs_workers {
        // Evaluate in the executing pool, including an explicitly installed
        // Config::threads pool below, once for all recorded worker guards.
        expression = syn::parse_quote!({
            let __cq_task_workers = ::rayon::current_num_threads();
            #expression
        });
    }
    if !partitions.is_empty() {
        return Err(syn::Error::new_spanned(
            program,
            "Tasks partition lacks a joined flatten or pool store",
        ));
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
                let __cq_task_context = ::cq_storage::profile_select! {
                    plain { () }
                    counters { ::cq_storage::counters::TaskContext::capture() }
                };
                ::rayon::ThreadPoolBuilder::new().num_threads(#threads).build()
                    .expect("create CQ task pool").install(|| ::cq_storage::profile_select! {
                        plain { #expression }
                        counters { __cq_task_context.run(|| #expression) }
                    })
            })
        };
    }
    Ok(syn::parse_quote!((#expression).into_iter()))
}

pub(crate) struct TaskTile {
    binding: syn::Ident,
    prepare: proc_macro2::TokenStream,
    parts: proc_macro2::TokenStream,
}

fn uses_binding(tokens: &proc_macro2::TokenStream, binding: &syn::Ident) -> bool {
    tokens.clone().into_iter().any(|token| match token {
        proc_macro2::TokenTree::Ident(ident) => ident == *binding,
        proc_macro2::TokenTree::Group(group) => uses_binding(&group.stream(), binding),
        _ => false,
    })
}

fn join_tile(
    emitter: &cq_sequence_pull::Emitter<'_>,
    tile: TaskTile,
    rows: proc_macro2::TokenStream,
    nested: bool,
) -> proc_macro2::TokenStream {
    let TaskTile {
        binding,
        prepare,
        parts,
    } = tile;
    // Shared indexed dispatch keeps Rayon plumbing independent of generated
    // tile sources and row pipelines; row iteration remains statically typed.
    let workers = if nested {
        let local = task_scope(emitter, quote!((#rows).collect::<::std::vec::Vec<_>>()));
        quote!(::cq_sequence_tasks::combine::map_tiles(#parts, &|#binding| #local))
    } else {
        let local = task_scope(emitter, quote!(__cq_buffer.extend(#rows)));
        // A scheduler task may consume several logical tiles. Preserve their
        // order while growing one buffer, rather than allocating per tile.
        quote!(::cq_sequence_tasks::combine::fold_tiles(
            #parts,
            &|__cq_buffer: &mut ::std::vec::Vec<_>, #binding| #local,
        ))
    };
    let joined = if nested {
        quote!(__cq_parts.into_iter().map(::std::vec::Vec::into_iter))
    } else {
        quote!(__cq_parts.into_iter().flatten())
    };
    quote!({
        #prepare
        let __cq_task_context = ::cq_storage::profile_select! {
            plain { () }
            counters { ::cq_storage::counters::TaskContext::capture() }
        };
        let __cq_parts = #workers;
        #joined
    })
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
        quote!(::cq_storage::profile_select! {
            plain { #body }
            counters { __cq_task_context.run(|| #body) }
        })
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
