//! Checked device Datalog with opaque, per-hole MiniLinq pipelines.
//! Schedule advice belongs to the selected source-to-storage pass. Storage advice
//! belongs to ordinary Structures passes such as `retype`; the bridge routes
//! arguments unchanged and records selector identities from checked storage.
//! Every rule/key hole supplies its static NVTX identity to `fused_retry_scope`,
//! so runtime retry estimates are isolated by hole without changing its kernel.
use cq_lang_datalog_round::{Program as RoundProgram, RoundContext};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{ToTokens, quote};
use std::collections::{BTreeMap, BTreeSet};

mod routing;
mod schedule_pass;

#[proc_macro]
pub fn datalog(input: TokenStream) -> TokenStream {
    expand(input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Consume schedule advice and lower source Datalog to checked storage IR.
#[proc_macro]
pub fn schedule(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_source::Program, cq_lang_datalog_storage::Program>(
        input.into(),
        schedule_pass::compile,
    )
    .into()
}

#[doc(hidden)]
#[proc_macro]
pub fn __scheduled(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_datalog_storage::Program, syn::Expr>(input.into(), |source, arguments| {
        syn::parse2(start_holes(source, syn::parse2(arguments)?)?)
    })
    .into()
}

#[doc(hidden)]
#[proc_macro]
pub fn __resident_reads(input: TokenStream) -> TokenStream {
    cq_stage::rewrite::<cq_lang_physical::Program>(input.into(), |source, arguments| {
        use syn::parse::Parser;
        let names = syn::punctuated::Punctuated::<syn::Ident, syn::Token![,]>::parse_terminated
            .parse2(arguments)?
            .into_iter()
            .collect::<Vec<_>>();
        build_reads_excluding(source, &names)
    })
    .into()
}

#[doc(hidden)]
#[proc_macro]
pub fn __collect_hole(input: TokenStream) -> TokenStream {
    cq_stage::pass::<cq_lang_structures::Program, syn::Expr>(input.into(), |source, arguments| {
        let mut state: routing::State = syn::parse2(arguments)?;
        state.collected.push(source.clone());
        syn::parse2(advance(state)?)
    })
    .into()
}

/// Keep explicitly certified frozen/leveled inputs as host reads; build all other
/// eligible keyed inputs using the ordinary checked physical rewrite.
fn build_reads_excluding(
    source: &cq_lang_physical::Program,
    names: &[syn::Ident],
) -> syn::Result<cq_lang_physical::Program> {
    let facts = cq_lang_physical::contract::check(source)?;
    let mut target = cq_rewrite_passes::build_host_reads(source, Tokens::new())?;
    for (before, after) in source
        .program
        .definitions
        .iter()
        .zip(&mut target.program.definitions)
    {
        if facts
            .hosts
            .get(&before.name.to_string())
            .is_some_and(|host| names.iter().any(|name| name == host))
        {
            after.mode = cq_lang_physical::Mode::Pipe(Default::default());
        }
    }
    cq_lang_physical::check_rewrite(source, &target)?;
    Ok(target)
}

type FrozenRuns = Vec<Vec<Vec<usize>>>;

/// Minimum chain cover of the actual demand sets (shared with
/// `cq_rewrites::sorted_indexes`). Each chain becomes one run; each
/// successive difference becomes a recorded lexicographic trie level.
fn demand_chains(demands: &BTreeSet<Vec<usize>>) -> FrozenRuns {
    // Frozen runs realize every prefix read themselves: any demand extends.
    cq_rewrite_passes::chain_cover(demands, |_| true)
}

/// Independently check exact demand coverage by the recorded run prefixes.
fn check_demand_chains(demands: &BTreeSet<Vec<usize>>, runs: &FrozenRuns) -> syn::Result<()> {
    cq_rewrite_passes::check_chain_cover(demands, runs)
        .map_err(|detail| syn::Error::new(proc_macro2::Span::call_site(), format!("CQ_GPU_FROZEN_LEVELS: {detail}")))
}

#[cfg(test)]
fn checked_holes(fork: &RoundProgram) -> syn::Result<Vec<cq_lang_structures::Program>> {
    fork.holes
        .iter()
        .enumerate()
        .map(|(hole_index, hole)| {
            let frozen: Vec<_> = fork
                .context
                .strata
                .iter()
                .filter(|stratum| {
                    stratum
                        .seeds
                        .iter()
                        .chain(&stratum.steps)
                        .any(|rule| rule.hole == hole_index)
                })
                .flat_map(|stratum| stratum.frozen_layouts.iter().chain(&stratum.leveled_layouts))
                .map(|&index| fork.context.layouts[index].name.clone())
                .collect();
            let planned = cq_planners_plan::compile(&hole.cq, cq_planners_plan::Planner::Le)?;
            let indexed = cq_index_policy::compile(&planned)?;
            let physical = cq_physical_lowering::compile(&indexed)?;
            let physical = build_reads_excluding(&physical, &frozen)?;
            cq_structure_selection::compile(&physical)
        })
        .collect()
}

fn frozen_runs(
    fork: &RoundProgram,
    holes: &[cq_lang_structures::Program],
) -> syn::Result<Vec<BTreeMap<usize, FrozenRuns>>> {
    fork.context
        .strata
        .iter()
        .map(|stratum| {
            let mut demands: BTreeMap<usize, BTreeSet<Vec<usize>>> = stratum
                .frozen_layouts
                .iter()
                .chain(&stratum.leveled_layouts)
                .map(|&index| (index, BTreeSet::new()))
                .collect();
            for rule in stratum.seeds.iter().chain(&stratum.steps) {
                let hole = &holes[rule.hole];
                let facts = cq_lang_structures::contract::check(hole)?;
                for entry in &hole.structures.entries {
                    let cq_lang_structures::Structure::Host { reader, keyed, .. } =
                        &entry.structure
                    else {
                        continue;
                    };
                    if reader == "scan" {
                        continue;
                    }
                    let result = entry.name.to_string();
                    let host = &facts.physical.hosts[&result];
                    let Some((&index, keys)) = demands
                        .iter_mut()
                        .find(|(index, _)| fork.context.layouts[**index].name == host)
                    else {
                        continue;
                    };
                    let layout = &fork.context.layouts[index];
                    let declaration = &fork.context.relations[layout.relation].declaration;
                    let mut slots = keyed
                        .level_names()
                        .into_iter()
                        .flatten()
                        .map(|key| {
                            let original = &facts.physical.renames[&result][&key];
                            layout
                                .columns
                                .iter()
                                .position(|&column| declaration.columns[column].name == original)
                                .expect("checked host rename maps to a declared column")
                        })
                        .collect::<Vec<_>>();
                    slots.sort_unstable();
                    keys.insert(slots);
                }
            }
            demands
                .into_iter()
                .map(|(index, keys)| {
                    let runs = demand_chains(&keys);
                    check_demand_chains(&keys, &runs)?;
                    Ok((index, runs))
                })
                .collect()
        })
        .collect()
}

fn expand(input: Tokens) -> syn::Result<Tokens> {
    let routing::Input {
        schedule,
        advice,
        source,
    } = syn::parse2(input)?;
    let callback: cq_stage::Invocation =
        syn::parse2(quote!(::cq_gpu_datalog::__scheduled(#advice)))?;
    cq_stage::Pipeline {
        passes: vec![schedule, callback],
    }
    .start(source.to_token_stream())
}

fn start_holes(source: &cq_lang_datalog_storage::Program, advice: routing::Advice) -> syn::Result<Tokens> {
    cq_lang_datalog_storage::contract::check(source)?;
    if source.incremental.strata.source.observer.is_some() {
        return Err(syn::Error::new_spanned(
            source,
            "CQ_GPU_DATALOG_OBSERVER: device Datalog does not execute Rust observers",
        ));
    }
    if source.policy.insertion != "device_set_all_columns"
        || source.policy.negation_keys != "device_set_owned"
    {
        return Err(syn::Error::new_spanned(
            source,
            "CQ_GPU_DATALOG_POLICY: requires device_set_all_columns/device_set_owned; CPU HashSet schedules cannot be reinterpreted",
        ));
    }
    let leveled_ratio = advice.leveled_ratio;
    let mut fork = cq_datalog_fork::compile_with_ratio(source, leveled_ratio)?;
    let pipelines = routing::resolve(source, &fork, advice)?;
    let names = routing::context_names(&mut fork.context, None);
    let mut pending = Vec::new();
    for (hole_index, hole) in fork.holes.iter().enumerate() {
        let config = &pipelines[hole_index];
        let frozen = fork
            .context
            .strata
            .iter()
            .filter(|stratum| {
                stratum
                    .seeds
                    .iter()
                    .chain(&stratum.steps)
                    .any(|rule| rule.hole == hole_index)
            })
            .flat_map(|stratum| stratum.frozen_layouts.iter().chain(&stratum.leveled_layouts))
            .map(|&index| &fork.context.layouts[index].name)
            .collect::<Vec<_>>();
        let mut passes = config.surface.clone();
        passes.extend([
            config.plan.clone().unwrap(),
            config.ix.clone().unwrap(),
            config.lower.clone().unwrap(),
        ]);
        passes.extend(config.physical.clone());
        passes.push(syn::parse2(
            quote!(::cq_gpu_datalog::__resident_reads(#(#frozen),*)),
        )?);
        passes.push(config.select.clone().unwrap());
        pending.push(routing::Pending {
            prefix: cq_stage::Pipeline { passes },
            source: hole.cq.to_token_stream(),
        });
    }
    pending.reverse();
    advance(routing::State {
        source: source.clone(),
        round: fork,
        names,
        pipelines,
        collected: Vec::new(),
        pending,
    })
}

fn advance(mut state: routing::State) -> syn::Result<Tokens> {
    if let Some(mut pending) = state.pending.pop() {
        pending.prefix.passes.push(syn::parse2(
            quote!(::cq_gpu_datalog::__collect_hole(#state)),
        )?);
        return pending.prefix.start(pending.source);
    }
    let mut fork = state.round;
    if state.collected.len() != fork.holes.len() || state.pipelines.len() != fork.holes.len() {
        return Err(syn::Error::new_spanned(
            &state.source,
            "CQ_GPU_DATALOG_HOLES: incomplete checked hole continuation",
        ));
    }
    routing::context_names(&mut fork.context, Some(&state.names));
    cq_datalog_fork::check(&state.source, &fork)?;
    render(&state.source, fork, state.collected, state.pipelines)
}

fn snapshots(context: &RoundContext, queries: Tokens, frozen: &[usize]) -> Tokens {
    let relations = context.relations.iter().map(|relation| {
        let state = &relation.seen;
        let all = &relation.all;
        let delta = relation
            .delta_host
            .as_ref()
            .map(|delta| quote!(let #delta = #state.delta();));
        quote!(let #all = #state.full(); #delta)
    });
    let layouts = context.layouts.iter().enumerate()
        .filter(|(index, _)| !frozen.contains(index)).map(|(_, layout)| (layout, false))
        .chain(context.delta_layouts.iter().map(|layout| (layout, true)))
        .filter(|(layout, _)| mentions(queries.clone(), &layout.name))
        .map(|(layout, delta)| {
            let relation = &context.relations[layout.relation];
            let input = if delta { relation.delta_host.as_ref().unwrap() } else { &relation.all };
            let host = &layout.host;
            let columns = &layout.columns;
            let types = columns.iter().map(|&i| &relation.declaration.columns[i].ty);
            quote! {
                let #host: ::cq_gpu_runtime::DeviceColumns<'_, (#(#types,)*), ::cq_gpu_runtime::Plain> =
                    ::cq_gpu_runtime::with_runtime_for(Some(#input.pool().clone()), |runtime| {
                        let mut execution = runtime.execution();
                        let input = execution.bind_columns(#input)?;
                        execution.into_columns(input, &[#(#columns),*])
                    }).expect("GPU Datalog layout projection failed");
            }
        });
    quote!(#(#relations)* #(#layouts)*)
}

fn render(
    source: &cq_lang_datalog_storage::Program,
    fork: RoundProgram,
    checked: Vec<cq_lang_structures::Program>,
    pipelines: Vec<cq_stage::Configuration>,
) -> syn::Result<Tokens> {
    let context = &fork.context;
    let literals = string_literals(source.incremental.strata.source.rules.iter().map(ToTokens::to_token_stream).collect());
    let schedule = syn::LitStr::new(
        &source.to_token_stream().to_string(),
        proc_macro2::Span::call_site(),
    );
    let variants = &source.incremental.variants;
    let selectors = variants
        .iter()
        .enumerate()
        .map(|(index, variant)| {
            let selector = routing::Selector {
                rule: variant.rule,
                delta: variant.delta,
            }
            .to_token_stream();
            syn::LitStr::new(
                &format!("variant {index}: hole({selector})"),
                proc_macro2::Span::call_site(),
            )
        })
        .collect::<Vec<_>>();
    let index_runs = frozen_runs(&fork, &checked)?;
    let anchor = context
        .relations
        .iter()
        .find(|r| r.declaration.value.is_some())
        .ok_or_else(|| {
            syn::Error::new(
                proc_macro2::Span::call_site(),
                "CQ_GPU_DATALOG_CONTEXT: at least one resident EDB is required (it may be empty)",
            )
        })?;
    let anchor_state = &anchor.seen;
    let anchor_host = &anchor.all;
    // Reuse source-fresh context names; observers are rejected by this bridge.
    let dictionary = &context.round_name;
    let pool = &context.observer_name;
    let edbs: Vec<_> = context
        .relations
        .iter()
        .filter(|relation| relation.declaration.value.is_some())
        .collect();
    let hosts = edbs.iter().map(|relation| {
        let host = &relation.all;
        let expression = &relation.declaration.value.as_ref().unwrap().expression;
        let types = relation.declaration.columns.iter().map(|column| &column.ty);
        quote! {
            let #host: &::cq_gpu_runtime::DeviceColumns<'_, (#(#types,)*), _> = &(#expression);
        }
    });
    let merge = edbs.iter().map(|relation| {
        let host = &relation.all;
        quote! {
            assert!(::std::sync::Arc::ptr_eq(#host.pool(), &#pool),
                "CQ_GPU_CONTEXT: Datalog EDBs must share one pool, stream and context");
            #dictionary.merge(#host.dictionary())
                .expect("CQ_GPU_DICTIONARY_DOMAIN: Datalog EDB dictionaries must have compatible prefixes");
        }
    });
    let dictionary_setup = quote! {
        #(#hosts)*
        let #pool = #anchor_host.pool().clone();
        let mut #dictionary = ::cq_gpu_runtime::Dictionary::default();
        #(#merge)*
        #(#dictionary.intern(#literals).expect("GPU Datalog string literal interning failed");)*
        let #dictionary = ::std::sync::Arc::new(#dictionary);
    };
    let mut bindings = Vec::new();
    for relation in context
        .relations
        .iter()
        .filter(|r| r.declaration.value.is_some())
    {
        let state = &relation.seen;
        let host = &relation.all;
        let types = relation.declaration.columns.iter().map(|c| &c.ty);
        bindings.push(quote! {
            let mut #state: ::cq_gpu_runtime::datalog::Relation<'_, (#(#types,)*)> =
                ::cq_gpu_runtime::datalog::Relation::from_columns(
                    &#host.with_dictionary(#dictionary.clone())
                        .expect("GPU Datalog common dictionary binding failed"))
                    .expect("GPU Datalog requires resident DeviceColumns EDBs with compatible context and dictionary");
        });
    }
    for relation in context
        .relations
        .iter()
        .filter(|r| r.declaration.value.is_none())
    {
        let state = &relation.seen;
        let types = relation.declaration.columns.iter().map(|c| &c.ty);
        let width = relation.declaration.columns.len();
        bindings.push(quote! {
            let mut #state: ::cq_gpu_runtime::datalog::Relation<'_, (#(#types,)*)> =
                ::cq_gpu_runtime::datalog::Relation::empty(
                    #pool.clone(), #dictionary.clone(), #width)
                    .expect("GPU Datalog empty relation failed");
        });
    }
    let holes = checked
        .iter()
        .enumerate()
        .map(|(hole_index, hole)| -> syn::Result<Tokens> {
            let mut frozen = Vec::new();
            let mut leveled = Vec::new();
            for (scc, stratum) in context.strata.iter().enumerate() {
                if !stratum
                    .seeds
                    .iter()
                    .chain(&stratum.steps)
                    .any(|rule| rule.hole == hole_index)
                {
                    continue;
                }
                for (&index, runs) in &index_runs[scc] {
                    let name = &context.layouts[index].name;
                    if !hole.physical.inputs().any(|input| input.name == *name) {
                        continue;
                    }
                    for run in runs {
                        let levels = run.iter().map(|level| quote!([#(#level),*]));
                        if stratum.leveled_layouts.contains(&index) {
                            leveled.push(quote!(#name => [#(#levels),*]));
                        } else {
                            frozen.push(quote!(#name => [#(#levels),*]));
                        }
                    }
                }
            }
            let hole = hole.to_token_stream();
            let keys: Vec<_> = context
                .strata
                .iter()
                .flat_map(|s| &s.keys)
                .filter(|key| mentions(hole.clone(), &key.name))
                .map(|key| {
                    let name = &key.name;
                    let types = key.columns.iter().map(|&column| {
                        &context.relations[key.relation].declaration.columns[column].ty
                    });
                    quote!(#name:(#(#types,)*))
                })
                .collect();
            let config = &pipelines[hole_index];
            let mut print = config.print.clone().expect("validated device printer");
            let supplied = std::mem::take(&mut print.arguments);
            // Prefix bridge context so an opaque caller's trailing comma stays
            // valid. Only the printer interprets options and rejects duplicates.
            print.arguments = quote!(completion(round),
                datalog_dictionary(#dictionary), datalog_pool(#pool), datalog_keys(#(#keys),*),
                #supplied);
            let mut passes = vec![
                syn::parse2(quote!(::cq_gpu::frozen_hosts(#(#frozen),*)))?,
                syn::parse2(quote!(::cq_gpu::leveled_hosts(#(#leveled),*)))?,
            ];
            passes.extend(config.structures.clone());
            passes.push(
                config
                    .represent
                    .clone()
                    .expect("resolved representation pass"),
            );
            passes.push(config.stage.clone().expect("resolved staging pass"));
            passes.push(print);
            let continuation = cq_stage::Pipeline { passes };
            continuation.start(hole)
        })
        .collect::<syn::Result<Vec<_>>>()?;
    let key_snapshots = snapshots(context, Tokens::new(), &[]);
    let strata = context.strata.iter().enumerate().map(|(scc, stratum)| {
        let scc_label = syn::LitCStr::new(
            &std::ffi::CString::new(format!("scc/{scc}")).unwrap(),
            proc_macro2::Span::call_site(),
        );
        let frozen = stratum.frozen_layouts.iter().map(|&index| {
            let layout = &context.layouts[index];
            let relation = &context.relations[layout.relation];
            let input = &relation.all;
            let host = &layout.host;
            let columns = &layout.columns;
            let types = columns.iter().map(|&i| &relation.declaration.columns[i].ty);
            let runs = &index_runs[scc][&index];
            let run_tokens = runs.iter().map(|run| {
                let levels = run.iter().map(|level| quote!(&[#(#level),*] as &[usize]));
                quote!(&[#(#levels),*] as &[&[usize]])
            });
            let certificate = syn::LitStr::new(&format!(
                "layout {} relation {} columns {:?} key {} realization MaintainedFrozenDeviceIndex lifetime stratum {} levels {:?}",
                layout.name, relation.declaration.name, columns, layout.key, scc, runs,
            ), proc_macro2::Span::call_site());
            quote! {
                let #host = {
                    const __CQ_DATALOG_FROZEN_LAYOUT_IR: &str = #certificate;
                    let mut execution = __cq_datalog_runtime.execution();
                    let input = execution.bind_columns(#input)?;
                    let projected: ::cq_gpu_runtime::DeviceColumns<'_, (#(#types,)*), ::cq_gpu_runtime::Plain> =
                        execution.into_columns(input, &[#(#columns),*])?;
                    ::cq_gpu_runtime::FrozenDeviceIndex::new(
                        __cq_datalog_runtime, &projected, &[#(#run_tokens),*])?
                };
            }
        });
        let leveled = stratum.leveled_layouts.iter().map(|&index| {
            let layout = &context.layouts[index];
            let relation = &context.relations[layout.relation];
            let host = &layout.host;
            let columns = &layout.columns;
            let types = columns.iter().map(|&i| &relation.declaration.columns[i].ty);
            let runs = &index_runs[scc][&index];
            let ratio = context.leveled_ratio;
            let run_tokens = runs.iter().map(|run| {
                let levels = run.iter().map(|level| quote!(&[#(#level),*] as &[usize]));
                quote!(&[#(#levels),*] as &[&[usize]])
            });
            let certificate = syn::LitStr::new(&format!(
                "layout {} relation {} columns {:?} key {} realization {:?} lifetime scc {} ratio {} levels {:?}",
                layout.name, relation.declaration.name, columns, layout.key,
                context.changing_layout_realization, scc, ratio, runs,
            ), proc_macro2::Span::call_site());
            quote! {
                let mut #host: ::cq_gpu_runtime::LeveledDeviceIndex<'_, '_, (#(#types,)*), ::cq_gpu_runtime::Plain> = {
                    const __CQ_DATALOG_LEVELED_LAYOUT_IR: &str = #certificate;
                    ::cq_gpu_runtime::LeveledDeviceIndex::new(
                        __cq_datalog_runtime, &[#(#run_tokens),*], #ratio)?
                };
            }
        });
        let keys = stratum.keys.iter().map(|key| {
            let name = &key.name;
            let hole = &holes[key.hole];
            let label_text = format!("hole/key/{name}");
            let retry_label = syn::LitStr::new(&label_text, proc_macro2::Span::call_site());
            let label = syn::LitCStr::new(
                &std::ffi::CString::new(label_text).unwrap(),
                proc_macro2::Span::call_site(),
            );
            let relation = &context.relations[key.relation].declaration.name;
            let certificate = syn::LitStr::new(&format!(
                "key {} relation {} columns {:?} representation DeviceSetOwned realization FrozenDeviceMembership lifetime stratum {}",
                name, relation, key.columns, scc,
            ), proc_macro2::Span::call_site());
            let constructor = if key.columns.is_empty() {
                quote!(::cq_gpu_runtime::FrozenDeviceMembership::new_exists)
            } else {
                quote!(::cq_gpu_runtime::FrozenDeviceMembership::new)
            };
            quote!(let #name = {
                let _cq_hole = ::cq_gpu_runtime::nvtx::range(#label);
                let _cq_retry = ::cq_gpu_runtime::fused_retry_scope(#retry_label);
                const __CQ_DATALOG_FROZEN_KEY_IR: &str = #certificate;
                let columns = #hole;
                #constructor(__cq_datalog_runtime, &columns)?
            };)
        });
        let phase = |rules: &[cq_lang_datalog_round::RuleHole], phase: &str| {
            let round_label = syn::LitCStr::new(
                &std::ffi::CString::new(format!("round/{phase}")).unwrap(),
                proc_macro2::Span::call_site(),
            );
            let queries: Tokens = rules.iter()
                .map(|rule| {
                    let query = &fork.holes[rule.hole].cq.query;
                    let scan_layout = rule.scan.as_ref().and_then(|scan| match scan.source {
                        cq_lang_datalog_round::InputSource::Layout(index) => Some(&context.layouts[index].name),
                        cq_lang_datalog_round::InputSource::Delta(_) => None,
                    });
                    quote!(#query #scan_layout)
                })
                .collect();
            let maintained = stratum.frozen_layouts.iter().chain(&stratum.leveled_layouts)
                .copied().collect::<Vec<_>>();
            let snapshots = snapshots(context, queries, &maintained);
            let candidates = stratum.members.iter().map(|&index| {
                let relation = &context.relations[index];
                let next = &relation.next;
                let types = relation.declaration.columns.iter().map(|c| &c.ty);
                quote!(let mut #next: ::std::vec::Vec<::cq_gpu_runtime::DeviceColumns<'_, (#(#types,)*), ::cq_gpu_runtime::Plain>> = ::std::vec::Vec::new();)
            });
            let calls = rules.iter().map(|rule| {
                let next = &context.relations[rule.head].next;
                let hole = &holes[rule.hole];
                let name = &context.relations[rule.head].declaration.name;
                let scan = rule.scan.as_ref().map(|scan| {
                    let binding = &scan.binding;
                    match scan.source {
                        cq_lang_datalog_round::InputSource::Delta(index) => {
                            let host = context.relations[index].delta_host.as_ref().unwrap();
                            quote!(let #binding = #host;)
                        }
                        cq_lang_datalog_round::InputSource::Layout(index) => {
                            let host = &context.layouts[index].host;
                            quote!(let #binding = &#host;)
                        }
                    }
                });
                let label_text = format!("hole/{name}/variant-{}", rule.variant);
                let retry_label = syn::LitStr::new(&label_text, proc_macro2::Span::call_site());
                let label = syn::LitCStr::new(
                    &std::ffi::CString::new(label_text).unwrap(),
                    proc_macro2::Span::call_site(),
                );
                quote!({
                    let _cq_hole = ::cq_gpu_runtime::nvtx::range(#label);
                    let _cq_retry = ::cq_gpu_runtime::fused_retry_scope(#retry_label);
                    #scan
                    #next.push(#hole);
                })
            });
            let prepare = stratum.members.iter().map(|&index| {
                let relation = &context.relations[index];
                let state = &relation.seen;
                let next = &relation.next;
                quote!(let #next = #state.prepare(&#next, &mut __cq_status).expect("GPU Datalog admission failed");)
            });
            let finish = stratum.members.iter().map(|&index| {
                let next = &context.relations[index].next;
                quote!(let #next = #next.finish(&__cq_checked).expect("GPU Datalog full snapshot failed");)
            });
            let publish = stratum.members.iter().map(|&index| {
                let relation = &context.relations[index];
                let state = &relation.seen;
                let next = &relation.next;
                quote!(#state = #next;)
            });
            let insert = stratum.leveled_layouts.iter().map(|&index| {
                let layout = &context.layouts[index];
                let relation = &context.relations[layout.relation];
                let state = &relation.seen;
                let host = &layout.host;
                let columns = &layout.columns;
                let types = columns.iter().map(|&i| &relation.declaration.columns[i].ty);
                quote! {
                    {
                        let delta = #state.delta();
                        let mut execution = __cq_datalog_runtime.execution();
                        let input = execution.bind_columns(delta)?;
                        let projected: ::cq_gpu_runtime::DeviceColumns<'_, (#(#types,)*), ::cq_gpu_runtime::Plain> =
                            execution.into_columns(input, &[#(#columns),*])?;
                        #host.insert(&projected, __cq_checked.rows(delta)? as u64)?;
                    }
                }
            });
            quote! {{
                let _cq_round = ::cq_gpu_runtime::nvtx::range(#round_label);
                let mut __cq_status = ::cq_gpu_runtime::datalog::RoundStatus::new(#anchor_state.full().pool().clone());
                #(#candidates)*
                { #snapshots #(#calls)* }
                #(#prepare)*
                let __cq_checked = __cq_status.check().expect("GPU Datalog convergence failed");
                #(#finish)*
                #(#publish)*
                #(#insert)*
                __cq_checked.any()
            }}
        };
        let seed = phase(&stratum.seeds, "seed");
        let step = phase(&stratum.steps, "step");
        quote! {
            {
                let _cq_scc = ::cq_gpu_runtime::nvtx::range(#scc_label);
                // Lower-stratum snapshots and projected negative keys stay frozen.
                #key_snapshots
                #(#keys)*
                #(#frozen)*
                #(#leveled)*
                let mut __cq_continue = #seed;
                while __cq_continue {
                    __cq_continue = #step;
                }
            }
        }
    });
    let outputs: Vec<_> = context
        .outputs
        .iter()
        .map(|output| {
            let state = &context.relations[output.relation].seen;
            quote!(#state.full().clone())
        })
        .collect();
    let result = if outputs.len() == 1 {
        outputs[0].clone()
    } else {
        quote!((#(#outputs,)*))
    };
    Ok(quote!({
        const __CQ_DATALOG_SCHEDULE_IR: &str = #schedule;
        const __CQ_DATALOG_HOLE_SELECTORS: &[&str] = &[#(#selectors),*];
        let _cq_program = ::cq_gpu_runtime::nvtx::range(c"program");
        #dictionary_setup #(#bindings)*
        ::cq_gpu_runtime::with_runtime_for(Some(#pool.clone()), |__cq_datalog_runtime| {
            let _cq_fixed_point = ::cq_gpu_runtime::nvtx::range(c"fixed-point");
            #(#strata)*
            __cq_datalog_runtime.execution().finish()?;
            Ok(#result)
        }).expect("GPU Datalog completion failed")
    }))
}

fn mentions(tokens: Tokens, name: &syn::Ident) -> bool {
    tokens.into_iter().any(|token| match token {
        proc_macro2::TokenTree::Ident(id) => id == *name,
        proc_macro2::TokenTree::Group(group) => mentions(group.stream(), name),
        _ => false,
    })
}

/// Intern only authored rule string literals, in stable source order. These
/// include heads, positive selections, guards, and negative constant filters.
fn string_literals(tokens: Tokens) -> Vec<syn::LitStr> {
    fn collect(
        tokens: Tokens,
        seen: &mut std::collections::BTreeSet<String>,
        literals: &mut Vec<syn::LitStr>,
    ) {
        for token in tokens {
            match token {
                proc_macro2::TokenTree::Group(group) => collect(group.stream(), seen, literals),
                proc_macro2::TokenTree::Literal(literal) => {
                    if let Ok(literal) = syn::parse2::<syn::LitStr>(literal.into_token_stream()) {
                        if seen.insert(literal.value()) {
                            literals.push(literal);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut literals = Vec::new();
    collect(
        tokens,
        &mut std::collections::BTreeSet::new(),
        &mut literals,
    );
    literals
}

#[cfg(test)]
mod tests {
    use super::*;

    // Host-only rendering harness. Production executes the caller's macros
    // through the public continuation protocol rather than this fixed fixture.
    fn expand(input: Tokens) -> syn::Result<Tokens> {
        let input: routing::Input = syn::parse2(input)?;
        let source = schedule_pass::compile(&input.source, input.schedule.arguments)?;
        let fork = cq_datalog_fork::compile_with_ratio(&source, input.advice.leveled_ratio)?;
        let pipelines = routing::resolve(&source, &fork, input.advice)?;
        let checked = checked_holes(&fork)?;
        render(&source, fork, checked, pipelines)
    }

    fn program() -> Tokens {
        quote! {
            struct Reachability;
            relation Edge(x:i64,y:i64) = (edges);
            relation Reach(x:i64,y:i64);
            output Reach;
            Reach(*x,*y) :- Edge(x,y);
            Reach(*x,*z) :- Reach(x,y), Edge(y,z);
        }
    }

    fn policy_pipeline(policy: &syn::Ident) -> Tokens {
        let mut config = routing::defaults();
        config.structures.last_mut().unwrap().arguments = quote!(dedup => ::cq_gpu_runtime::DeviceSet<_, ::cq_gpu_runtime::Plain, ::cq_gpu_runtime::#policy>);
        routing::config_tokens(&config)
    }

    #[test]
    fn scheduling_and_storage_advice_are_routed_to_their_consumers() {
        let source = program();
        let pipeline = policy_pipeline(&syn::parse_quote!(Fused));
        let emitted = expand(quote! {
            schedule = ::cq_gpu_datalog::schedule(delta_first, hoist_invariants);
            #pipeline
            #source
        })
        .unwrap()
        .to_string();
        assert_eq!(emitted.matches("cq_gpu_runtime :: Fused").count(), 2);
        assert!(emitted.contains("__CQ_DATALOG_HOLE_SELECTORS"));
        assert!(emitted.contains("hole(rule 1 , delta 0)"));

        let opaque = super::expand(quote! {
            schedule = ::custom::schedule(unknown { schedule tokens });
            pipeline {
                plan = ::custom::plan(skip(Edge with Reach));
                select = ::custom::select(positional(Edge));
            }
            #source
        })
        .unwrap()
        .to_string();
        assert!(opaque.contains("unknown { schedule tokens }"));
        assert!(opaque.contains("skip (Edge with Reach)"));
        assert!(opaque.contains("positional (Edge)"));
    }

    #[test]
    fn per_hole_override_preserves_default_and_rejects_invalid_selectors() {
        let source = program();
        let fused = policy_pipeline(&syn::parse_quote!(Fused));
        let emitted = expand(quote! {
            hole(rule 1, delta 0) #fused
            #source
        })
        .unwrap()
        .to_string();
        assert_eq!(emitted.matches("cq_gpu_runtime :: Fused").count(), 1);
        assert_eq!(emitted.matches("cq_gpu_runtime :: Compacting").count(), 1);
        for selector in [
            quote!(rule 0, delta 0),
            quote!(rule 1, delta 1),
            quote!(rule 99, seed),
        ] {
            let error = expand(quote!(hole(#selector) #fused #source)).unwrap_err();
            assert!(error.to_string().contains("HOLE_SELECTOR"));
        }
        let error = expand(quote!(
            hole(rule 1, delta 0) #fused
            hole(rule 1, delta 0) #fused
            #source
        ))
        .unwrap_err();
        assert!(error.to_string().contains("ambiguous duplicate"));
        // An explicitly empty Structures queue is routed as written. Whether
        // its IR is device-realizable belongs to the selected consuming printer.
        let emitted = expand(quote!(
            hole(rule 1, delta 0) pipeline { structures = []; }
            #source
        ))
        .unwrap()
        .to_string();
        assert_eq!(emitted.matches("cq_gpu :: prefix_runs").count(), 1);
        assert_eq!(emitted.matches("cq_rewrites :: expand_lookups").count(), 1);
    }

    #[test]
    fn only_the_schedule_pass_validates_schedule_arguments() {
        let source: cq_lang_datalog_source::Program = syn::parse2(program()).unwrap();
        assert!(
            schedule_pass::compile(&source, quote!(unknown))
                .unwrap_err()
                .to_string()
                .contains("SCHEDULE_ARGUMENT")
        );
        assert!(schedule_pass::compile(&source, quote!(delta_first, delta_first)).is_err());
        let written = schedule_pass::compile(&source, Tokens::new()).unwrap();
        assert_eq!(written.incremental.variants[1].driver, Some(0));
        let mut config = routing::defaults();
        let opaque = quote!(this is not a dedup argument { but the router must not parse it });
        config.structures.last_mut().unwrap().arguments = opaque.clone();
        routing::validate(&config).unwrap();
        assert_eq!(
            config.structures.last().unwrap().arguments.to_string(),
            opaque.to_string()
        );
    }

    #[test]
    fn opaque_printer_trailing_comma_survives_context_injection() {
        let source = program();
        let generated = expand(quote! {
            pipeline { print = ::cq_gpu::print(tile(256,4),); }
            #source
        }).unwrap().to_string().split_whitespace().collect::<String>();
        assert!(generated.contains("tile(256,4),)"));
        assert!(!generated.contains(",,"));
    }

    #[test]
    fn string_domain_uses_stable_authored_literal_order() {
        let literals = string_literals(quote! {
            T(*x, "a") :- E(x), !M(x, r#"b"#), #[total] if *x == "a";
            U(*x, "c") :- E(x);
        });
        assert_eq!(
            literals.iter().map(syn::LitStr::value).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        let emitted = expand(quote! {
            struct Literals;
            relation E(x:i64) = (input);
            relation T(x:i64,tag:&'static str);
            output T;
            T(*x,"a") :- E(x);
            T(*x,"b") :- E(x);
        })
        .unwrap()
        .to_string();
        assert_eq!(emitted.matches("datalog_dictionary").count(), 2);
        assert_eq!(emitted.matches("datalog_pool").count(), 2);
        assert!(
            emitted.find(". intern").unwrap() < emitted.find("Relation :: from_columns").unwrap()
        );
    }

    #[test]
    fn every_hole_uses_gpu_printer_and_resident_publication() {
        let emitted = expand(quote! {
            struct Reachability;
            relation Edge(x:i64,y:i64) = (edges);
            relation Reach(x:i64,y:i64);
            output Reach;
            Reach(*x,*y) :- Edge(x,y);
            Reach(*x,*z) :- Reach(x,y), Edge(y,z);
        })
        .unwrap()
        .to_string();
        assert_eq!(emitted.matches("cq_gpu :: print").count(), 2);
        assert!(emitted.contains("prepare"));
        assert_eq!(emitted.matches("completion (round)").count(), 2);
        assert!(!emitted.contains("output (retry)"));
        assert_eq!(emitted.matches("fused_retry_scope").count(), 2);
        assert!(emitted.contains("fused_retry_scope (\"hole/Reach/variant-0\")"));
        assert!(emitted.contains("fused_retry_scope (\"hole/Reach/variant-1\")"));
        assert_eq!(emitted.matches("cq_rewrites :: hashed_runs").count(), 2);
        assert_eq!(emitted.matches("cq_gpu_runtime :: Compacting").count(), 2);
        assert!(emitted.contains("__CQ_DATALOG_HOLE_SELECTORS"));
        assert_eq!(emitted.matches("expand_lookups (256)").count(), 2);
        assert_eq!(emitted.matches("tile (256 , 4)").count(), 2);
        assert_eq!(emitted.matches("frontier (8)").count(), 2);
        assert_eq!(emitted.matches("directory (rank)").count(), 2);
        assert_eq!(emitted.matches("FrozenDeviceIndex :: new").count(), 1);
        assert!(
            emitted.find("FrozenDeviceIndex :: new").unwrap()
                < emitted.find("while __cq_continue").unwrap()
        );
        assert!(emitted.contains("MaintainedFrozenDeviceIndex lifetime stratum 0"));
        assert!(emitted.find(". check").unwrap() < emitted.find(". finish").unwrap());
        assert!(emitted.contains("RoundStatus"));
        assert!(!emitted.contains("to_host"));
        assert_eq!(emitted.matches("retype (dedup =>").count(), 2);
    }

    #[test]
    fn changing_keyed_layouts_are_maintained_once_after_admission() {
        let emitted = expand(quote! {
            leveled_ratio(8);
            struct Closure;
            relation Edge(x:i64,y:i64) = (edges);
            relation Reach(x:i64,y:i64);
            output Reach;
            Reach(*x,*y) :- Edge(x,y);
            Reach(*x,*z) :- Reach(x,y), Reach(y,z);
        }).unwrap().to_string();
        assert!(emitted.contains("MaintainedLeveledDeviceIndex lifetime scc 0 ratio 8"));
        assert_eq!(emitted.matches("LeveledDeviceIndex :: new").count(), 1);
        assert!(emitted.contains("cq_gpu :: leveled_hosts"));
        assert!(emitted.find("LeveledDeviceIndex :: new").unwrap()
            < emitted.find("while __cq_continue").unwrap());
        assert!(emitted.find(". check").unwrap() < emitted.find(". insert").unwrap());
        assert!(emitted.contains("__cq_checked . rows (delta)"));
        assert!(!emitted.contains("checked_len"));
        for ratio in [quote!(leveled_ratio(0);), quote!(leveled_ratio(1);)] {
            let source = program();
            assert!(expand(quote!(#ratio #source)).is_err());
        }
    }

    #[test]
    fn frozen_demand_chains_cover_exact_checked_prefixes() {
        let demands = BTreeSet::from([vec![1, 2], vec![0, 1, 2]]);
        let runs = demand_chains(&demands);
        assert_eq!(runs, vec![vec![vec![1, 2], vec![0]]]);
        check_demand_chains(&demands, &runs).unwrap();
        assert!(check_demand_chains(&demands, &vec![vec![vec![0, 1, 2]]]).is_err());
        assert!(check_demand_chains(&demands, &vec![vec![vec![1, 2], vec![0, 2]]]).is_err());
        let branching = BTreeSet::from([vec![0], vec![1], vec![0, 1], vec![0, 2]]);
        let runs = demand_chains(&branching);
        assert_eq!(
            runs.len(),
            2,
            "minimum chain cover avoids redundant index builds"
        );
        check_demand_chains(&branching, &runs).unwrap();
        let mut wrong = runs.clone();
        wrong.push(runs[0].clone());
        assert!(check_demand_chains(&branching, &wrong).is_err());
    }

    #[test]
    fn le_frozen_semi_and_probe_share_one_recorded_trie() {
        let source: cq_lang_datalog_source::Program = syn::parse_quote! {
            struct StackShape;
            relation Seed(a:i64,b:i64,c:i64,d:i64,e:i64,f:i64) = (seed);
            relation Prior(ea:i64,block:i64,r:i64,p:i64) = (prior);
            relation Live(block:i64,r:i64,p:i64,ur:i64,up:i64,ea:i64) = (live);
            relation R(a:i64,b:i64,c:i64,d:i64,e:i64,f:i64);
            output R;
            R(*a,*b,*c,*d,*e,*f) :- Seed(a,b,c,d,e,f);
            R(*a,*b,*c,*next,*ur,*up) :- Prior(used,block,r,p), R(a,b,c,used,r,p), Live(block,r,p,ur,up,next);
        };
        let source = cq_datalog_stratify::compile(&source).unwrap();
        let source = cq_datalog_incremental::compile_with_driver(
            &source,
            cq_datalog_incremental::DriverPolicy::DeltaFirst,
        )
        .unwrap();
        let source = cq_datalog_storage::compile_with_representation(
            &source,
            cq_datalog_storage::Representation::DeviceSet,
        )
        .unwrap();
        let fork = cq_datalog_fork::compile(&source).unwrap();
        let holes = checked_holes(&fork).unwrap();
        let runs = frozen_runs(&fork, &holes).unwrap();
        let live = fork
            .context
            .layouts
            .iter()
            .position(|layout| fork.context.relations[layout.relation].declaration.name == "Live")
            .unwrap();
        assert_eq!(runs[0][&live], vec![vec![vec![1, 2], vec![0]]]);
        let step = fork.context.strata[0].steps[0].hole;
        let text = holes[step]
            .to_token_stream()
            .to_string()
            .split_whitespace()
            .collect::<String>();
        assert!(text.contains("semieager[p,r]"), "{text}");
        assert!(text.contains("probefirst[block,p,r]"), "{text}");
    }

    #[test]
    fn frozen_negative_memberships_are_built_before_rounds() {
        let emitted = expand(quote! {
            struct FrozenNegation;
            relation E(x:i64) = (input);
            relation Blocked(x:i64) = (blocked);
            relation R(x:i64);
            output R;
            R(*x) :- E(x), !Blocked(x);
            R(*x) :- R(x), !Blocked(_);
        })
        .unwrap()
        .to_string();
        assert_eq!(
            emitted.matches("FrozenDeviceMembership :: new (").count(),
            1
        );
        assert_eq!(
            emitted
                .matches("FrozenDeviceMembership :: new_exists (")
                .count(),
            1
        );
        assert!(
            emitted.rfind("FrozenDeviceMembership ::").unwrap()
                < emitted.find("while __cq_continue").unwrap()
        );
        assert_eq!(emitted.matches("__CQ_DATALOG_FROZEN_KEY_IR").count(), 2);
        assert_eq!(emitted.matches("fused_retry_scope").count(), 4);
        assert_eq!(emitted.matches("fused_retry_scope (\"hole/key/").count(), 2);
    }

    #[test]
    fn single_output_preserves_direct_result_shape() {
        let emitted = expand(quote! {
            struct Edges;
            relation E(x:i64) = (input);
            output E;
        })
        .unwrap();
        let block: syn::ExprBlock = syn::parse2(emitted).unwrap();
        assert!(matches!(
            block.block.stmts.last(),
            Some(syn::Stmt::Expr(syn::Expr::MethodCall(_), None))
        ));
    }

    #[test]
    fn round_scan_bindings_use_resident_layout_and_delta_snapshots() {
        let source: cq_lang_datalog_source::Program = syn::parse2(program()).unwrap();
        let storage = schedule_pass::compile(&source, Tokens::new()).unwrap();
        let round = cq_datalog_fork::compile(&storage).unwrap();
        let pipelines = routing::resolve(
            &storage,
            &round,
            syn::parse2(Tokens::new()).unwrap(),
        )
        .unwrap();
        let mut bindings = Vec::new();
        let mut layout_scans = 0;
        let mut delta_scans = 0;
        for rule in round.context.strata.iter().flat_map(|stratum| {
            stratum.seeds.iter().chain(&stratum.steps)
        }) {
            let scan = rule.scan.as_ref().unwrap();
            let binding = &scan.binding;
            let expected = match scan.source {
                cq_lang_datalog_round::InputSource::Layout(index) => {
                    layout_scans += 1;
                    let host = &round.context.layouts[index].host;
                    quote!(let #binding = &#host;)
                }
                cq_lang_datalog_round::InputSource::Delta(index) => {
                    delta_scans += 1;
                    let host = round.context.relations[index].delta_host.as_ref().unwrap();
                    quote!(let #binding = #host;)
                }
            };
            bindings.push(expected.to_string());
        }
        assert_eq!((layout_scans, delta_scans), (1, 1));
        let checked = checked_holes(&round).unwrap();
        let emitted = render(&storage, round, checked, pipelines).unwrap().to_string();
        for binding in bindings {
            assert!(emitted.contains(&binding), "{binding}");
        }
    }

    #[test]
    fn cpu_schedule_is_not_reinterpreted() {
        let source: cq_lang_datalog_source::Program = syn::parse_quote! {
            struct Copy;
            relation E(x:i64) = (input);
            relation R(x:i64);
            output R;
            R(*x) :- E(x);
        };
        let source = cq_datalog_stratify::compile(&source).unwrap();
        let source = cq_datalog_incremental::compile(&source).unwrap();
        let source = cq_datalog_storage::compile(&source).unwrap();
        let error = start_holes(&source, syn::parse2(Tokens::new()).unwrap()).unwrap_err();
        assert!(error.to_string().contains("CQ_GPU_DATALOG_POLICY"));
    }
}
