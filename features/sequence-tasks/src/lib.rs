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
    /// Rows per task-local key-routing chunk; contiguous tile sizes are planned.
    pub grain: usize,
    /// Logical partition count fixed before execution, independent of workers.
    pub partitions: usize,
    /// Published rows at which the planner records routed-provider construction.
    pub key_partition_threshold: usize,
}

impl Default for Config {
    fn default() -> Self {
        // A routing task fills about 1K rows per logical bucket, avoiding a
        // tiny allocation for every (1K-row chunk, partition) pair.
        Self { threads: None, grain: 64 * 1024, partitions: 64, key_partition_threshold: 64 * 1024 }
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
    assert!(config.key_partition_threshold > 0, "provider partition threshold must be positive");
    let mut machine = machine_with_width(config.partitions);
    machine.key_partition_threshold = config.key_partition_threshold;
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
            segment.ident == "HashIndex" || segment.ident == "Trie" || segment.ident == "KeySet")));
    catalog.providers.insert(0, cq_lang_sequence::ProviderToken {
        ty: syn::parse_quote!(::cq_storage::sharded::Index<_, _>),
        capabilities: vec![cq_lang_sequence::Capability::Exact, cq_lang_sequence::Capability::Membership],
        levels: levels.clone(),
        elements: None,
    });
    // Hash sharding retains exact probes, but not a shard's global ordering.
    // The synchronous pool join publishes native memory to host and task readers.
    use cq_lang_sequence::{Capability, ProviderToken};
    for (ty, capabilities) in [
        (
            syn::parse_quote!(::cq_storage::sharded::Sharded<::cq_storage::HashSet<_>>),
            vec![Capability::Membership],
        ),
        (
            syn::parse_quote!(::cq_storage::sharded::Sharded<::cq_storage::AdaptiveKeySet<_, _, _>>),
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
    source: &Program, catalog: &cq_lang_sequence::Catalog,
) -> syn::Result<Program> {
    // Instantiate supported authored families before checked selection. A
    // transient retype can invalidate obligations such as exact join filters.
    let mut native = catalog.clone();
    for entry in &source.structures.structures.entries {
        let ty = match &entry.structure {
            cq_lang_sequence::structures::Structure::Built { keyed, .. } => &keyed.ty,
            cq_lang_sequence::structures::Structure::KeySet { ty, .. } => ty,
            cq_lang_sequence::structures::Structure::JoinFilter { filter, .. } => &filter.ty,
            _ => continue,
        };
        if let Some(token) = catalog.providers.iter().find(|token|
                cq_sequence_plan::matches_type(&token.ty, ty))
        {
            let mut token = token.clone();
            token.ty = ty.clone();
            native.providers.push(token);
        }
    }
    let mut target = source.clone();
    target.structures =
        cq_sequence_rewrite_passes::select_from_catalog(&source.structures, &native)?;
    // A legal authored provider may supply stronger capabilities needed by an
    // authored tile (for example range cursors). Retarget only unavailable
    // families; do not downgrade an existing native representation.
    for original in &source.structures.structures.entries {
        // Replayable row buffers are not keyed range providers. Keep their
        // native row-collection ABI rather than selecting an index family.
        if let cq_lang_sequence::structures::Structure::Rows { ty, .. } = &original.structure {
            for selected in &mut target.structures.structures.entries {
                if selected.name == original.name
                    && let cq_lang_sequence::structures::Structure::Rows { ty: replacement, .. } = &mut selected.structure
                {
                    *replacement = ty.clone();
                }
            }
            continue;
        }
    }
    for (old, retained) in source.structures.physical.program.definitions.iter()
        .zip(&target.structures.physical.program.definitions) {
        if old.mode == retained.mode {
            continue;
        }
        // Catalog retention changes a host rename's mode, but Sequence was
        // lowered before selection. Its publication must become explicit too.
        if let Some(definition) = target.sequence.definitions.iter_mut()
            .find(|definition| definition.name == retained.name) {
            if !matches!(definition.term, Term::Store { .. }) {
                let inner = std::mem::replace(&mut definition.term, Term::Union(Vec::new()));
                definition.term = Term::Store { capacity: None, inner: Box::new(inner) };
            }
        } else {
            target.sequence.definitions.insert(0, cq_lang_sequence::Definition {
                name: retained.name.clone(),
                term: Term::Store { capacity: None, inner: Box::new(Term::Rows(retained.name.clone())) },
            });
            if let Some(graded) = &mut target.graded {
                graded.grades.push(cq_lang_sequence::Grade {
                    name: retained.name.clone(), level: vec![syn::parse_quote!(host)],
                });
            }
        }
    }
    cq_lang_sequence::check_rewrite(source, &target)?;
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
    // A distributing tile binds a symbolic one-part iterator. Streaming row
    // adapters compose through its paired flatten and named pipes. A Store or
    // non-distributive constructor joins the task buffers before proceeding.
    let mut partitions = Vec::new();
    let mut stored_sources = std::collections::BTreeMap::<String, syn::Ident>::new();
    let mut next_driver = 0usize;
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
                let binding = format_ident!("__cq_task_driver_{}", next_driver);
                next_driver += 1;
                let context = emitter.context();
                let host_result = input.result.as_ref().filter(|result|
                    inner_term(term).is_some_and(|inner| base_rows(inner, program))
                    && context.physical_analysis.hosts.contains_key(*result)
                    && !context.analysis.selections.contains_key(*result)
                    && context.join_filter_ids(result).is_none()
                );
                let positional = host_result.filter(|result| context.carries_positions(result));
                if let Some(result) = positional {
                    let host = cq_print::Context::positional_host(&context.physical_analysis.hosts[result]);
                    let ids = quote!(::cq_storage::PositionalRows::position_range(
                        #host, #binding * #n..(#binding + 1) * #n));
                    let rows = context.profile_stream(result, context.positional_rows(result, ids));
                    partitions.push(TaskTile {
                        binding,
                        prepare: quote!(),
                        parts: quote!((0..::cq_storage::PositionalRows::len(#host).div_ceil(#n)).into_par_iter()),
                    });
                    output.rows = quote!(::core::iter::once(#rows));
                } else if let Some(result) = host_result {
                    let host = &context.hosts[&context.physical_analysis.hosts[result]].value;
                    let rows = context.profile_stream(result, context.host_scan_from(result, Some(quote!(#binding))));
                    partitions.push(TaskTile {
                        binding,
                        prepare: quote! {
                            use ::mini_linq::__tasks::combine::TileRows as _;
                            let __cq_driver = ::mini_linq::__tasks::combine::TileSource(#host);
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
                        parts: quote!(__cq_driver.par_chunks(#n)),
                    });
                    output.rows = quote!(::core::iter::once(#binding.iter().cloned()));
                }
                output.partitioned = true;
            } else if let Term::PartitionK { keys, parts, inner } = term {
                let input = child.expect("checked key partition");
                let stored = match inner.as_ref() {
                    Term::Ref(name) => stored_sources.get(&cq_lang_sequence::structures::symbol_name(name)),
                    _ => None,
                };
                output.rows = keyed::partition(emitter, input, keys, *parts, config.grain, stored)?;
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
                let partial = emitter.is_partial_group(term);
                let tile = if partial {
                    partitions.iter().position(|tile: &TaskTile| uses_binding(&input.rows, &tile.binding))
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
            } else if matches!(term, Term::Store { .. } | Term::Canon(_) | Term::Group(_, _) | Term::Fold(_, _))
                || program.sequence.definitions.last().is_some_and(|definition|
                    std::ptr::eq(&definition.term, term))
            {
                let input = if matches!(term, Term::Store { .. } | Term::Canon(_) | Term::Group(_, _) | Term::Fold(_, _)) {
                    child.unwrap_or(output)
                } else {
                    &*output
                };
                if let Some(index) = partitions.iter().position(|tile: &TaskTile|
                    uses_binding(&input.rows, &tile.binding))
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
                        && program.sequence.definitions.last().is_some_and(|definition|
                            std::ptr::eq(&definition.term, term))
                        && let Some(result) = &output.result
                        && !analysis.structures.head_dedup.contains_key(result)
                    {
                        local = emitter.own_task_answer(result, local);
                    }
                    input.rows = join_tile(emitter, tile, local, nested);
                    output.rows = match term {
                        Term::Canon(_) => emitter.canon(input)?.rows,
                        Term::Group(name, _) => emitter.group(name, input)?.rows,
                        Term::Fold(monoid, _) => emitter.fold(monoid, input)?.rows,
                        _ => input.rows,
                    };
                }
            }
            let routed_provider = output.result.as_ref().is_some_and(|result|
                analysis.structures.built.get(result).is_some_and(|providers|
                    providers.iter().any(|(_, ty)| matches!(ty, syn::Type::Path(path)
                        if path.path.segments.last().is_some_and(|segment| segment.ident == "Sharded")))));
            if matches!(term, Term::Store { .. }) && output.depth == 0
                && base_rows(term, program)
                && output.result.as_ref().is_some_and(|result|
                    emitter.context().physical_analysis.hosts.contains_key(result)
                    && (routed_provider || analysis.structures.built.get(result).is_none_or(Vec::is_empty))
                    && !analysis.structures.selections.contains_key(result)
                    && emitter.context().join_filter_ids(result).is_none()
                    && matches!(&emitter.context().physical[result].operator,
                        cq_lang_sequence::structures::physical::Operator::Rename(rename)
                            if rename.constants.is_none()))
            {
                // An immutable base relation is already resident storage. The
                // checked Store publishes that borrow; it need not clone rows.
                output.rows = child.expect("checked base store").rows.clone();
                let scan = emitter.context().host_scan(output.result.as_ref().expect("base row provenance"));
                emitter.publish_length(owner, quote!({
                    let __cq_length_rows = #scan;
                    let (__cq_minimum, __cq_maximum) = __cq_length_rows.size_hint();
                    if __cq_maximum == ::core::option::Option::Some(__cq_minimum) {
                        __cq_minimum
                    } else {
                        __cq_length_rows.count()
                    }
                }));
                emitter.mark_target_published(owner);
            } else if output.depth == 0 && (partition_inputs.contains(owner) || routed_provider)
                && let Term::Store { inner, .. } = term
                && let Term::Ref(source) = inner.as_ref()
                && emitter.alias_store(owner, &cq_lang_sequence::structures::symbol_name(source))
            {
                output.rows = child.expect("checked store alias").rows.clone();
                if analysis.results.get(owner) == child.and_then(|value| value.result.as_ref())
                    && let Some(binding) = stored_sources.get(&cq_lang_sequence::structures::symbol_name(source)).cloned()
                {
                    stored_sources.insert(owner.to_owned(), binding);
                }
            } else if matches!(term, Term::Store { .. }) && output.depth == 0
                && (partition_inputs.contains(owner) || routed_provider)
            {
                let label = format!("{owner}.rows");
                emitter.context_mut().profile_label(
                    &label, "ROWS_BUILD", "", output.result.iter().cloned().collect(),
                );
                let rows = emitter.context().profile_inserting(&label, output.rows.clone());
                let build = emitter.context().profile_build(
                    &label, quote!((#rows).collect::<::std::vec::Vec<_>>()),
                );
                let binding = format_ident!("__cq_key_source_{}", owner);
                stored_sources.insert(owner.to_owned(), binding.clone());
                emitter.context_mut().statements.push(syn::parse_quote!(
                    let #binding = #build;
                ));
                emitter.context_mut().statements.push(syn::parse_quote!(
                    let _retain = ::cq_storage::retain_rows(&#binding);
                ));
                emitter.context_mut().bound = true;
                emitter.mark_target_published(owner);
                emitter.publish_length(owner, quote!(#binding.len()));
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
                emitter.context_mut().statements.push(syn::parse_quote!(
                    let _retain_parts = #binding.iter().map(::cq_storage::retain_rows).collect::<::std::vec::Vec<_>>();
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

fn inner_term(term: &Term) -> Option<&Term> {
    match term {
        Term::Tile { inner, .. } => Some(inner),
        _ => None,
    }
}

fn base_rows(term: &Term, program: &Program) -> bool {
    match term {
        Term::Rows(_) => true,
        Term::Store { inner, .. } => base_rows(inner, program),
        Term::Ref(name) => program.sequence.definitions.iter()
            .find(|definition| definition.name == *name)
            .is_some_and(|definition| base_rows(&definition.term, program)),
        _ => false,
    }
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
    let TaskTile { binding, prepare, parts } = tile;
    let workers = if nested {
        let local = task_scope(emitter, quote!((#rows).collect::<::std::vec::Vec<_>>()));
        quote!((#parts).map(|#binding| #local))
    } else {
        let local = task_scope(emitter, quote!({
            __cq_buffer.extend(#rows);
            __cq_buffer
        }));
        // A scheduler task may consume several logical tiles. Preserve their
        // order while growing one buffer, rather than allocating per tile.
        quote!((#parts).fold(::std::vec::Vec::new, |mut __cq_buffer, #binding| #local))
    };
    let joined = if nested {
        quote!(__cq_parts.into_iter().map(::std::vec::Vec::into_iter))
    } else {
        quote!(__cq_parts.into_iter().flatten())
    };
    quote!({
        use ::rayon::prelude::*;
        #prepare
        let __cq_task_context = ::cq_storage::profile_select! {
            plain { () }
            counters { ::cq_storage::counters::TaskContext::capture() }
        };
        let __cq_parts = (#workers).collect::<::std::vec::Vec<_>>();
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
