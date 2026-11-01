//! Storage provider erasure, admission shards, and exclusive layout publication.
use cq_datalog_partition::{
    DEFAULT_GRAIN, DEFAULT_THRESHOLD, ShardedProgram, check_sharded, check_storage,
    compile_sharded, compile_sharded_with, compile_storage, compile_storage_with, run_layout,
    sharded_layout,
};
use cq_lang_datalog_round::{self as round, Program, storage};
use cq_stage::Language;
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

fn lower_storage(tokens: TokenStream) -> storage::Program {
    let source = syn::parse2(tokens).unwrap();
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    cq_datalog_storage::compile(&incremental).unwrap()
}

fn authored_storage() -> storage::Program {
    lower_storage(quote! {
        struct Closure;
        relation Edge(x:i32,y:i32) = (edges);
        relation Blocked(x:i32) = (blocked);
        relation Reach(x:i32,y:i32);
        relation Answer(x:i32,y:i32);
        output Answer;
        Reach(*x,*y) :- Edge(x,y), !Blocked(y);
        Reach(*x,*z) :- Reach(x,y), Reach(y,z);
        Reach(*x,*z) :- Reach(y,z), Reach(x,y);
        Answer(*x,*y) :- Reach(x,y), !Blocked(x);
    })
}

fn authored_round() -> Program {
    let storage = compile_storage(&authored_storage()).unwrap();
    cq_datalog_fork::compile(&storage).unwrap()
}

fn expected_push(mut program: Program) -> Program {
    for rule in program
        .context
        .strata
        .iter()
        .flat_map(|s| s.seeds.iter().chain(&s.steps))
    {
        let hole = &mut program.holes[rule.hole];
        hole.interface = round::HoleInterface::Push;
        let start = hole.pipeline.passes.len() - 3;
        hole.pipeline.passes.splice(
            start..,
            [syn::parse_quote!(::cq_datalog::passes::print_push)],
        );
    }
    program
}

fn rejected(change: impl FnOnce(&mut ShardedProgram)) {
    let source = authored_round();
    let mut target = compile_sharded(&source).unwrap();
    change(&mut target);
    assert!(check_sharded(&source, &target).is_err());
    assert!(Language::check(&target).is_err());
}

fn rejected_with(diagnostic: &str, change: impl FnOnce(&mut ShardedProgram)) {
    let source = authored_round();
    let mut target = compile_sharded(&source).unwrap();
    change(&mut target);
    for error in [
        check_sharded(&source, &target).unwrap_err(),
        Language::check(&target).unwrap_err(),
    ] {
        assert!(error.to_string().contains(diagnostic), "{error}");
    }
}

#[test]
fn missing_admission_has_specific_diagnostic() {
    rejected_with("missing admission relation", |p| {
        p.certificate.admission.pop();
    });
}

#[test]
fn duplicate_admission_has_specific_diagnostic() {
    rejected_with("duplicate admission relation", |p| {
        p.certificate
            .admission
            .push(p.certificate.admission[0].clone());
    });
}

#[test]
fn missing_layout_coverage_has_specific_diagnostic() {
    for delta in [false, true] {
        rejected_with("missing layout coverage", |p| {
            let owners = if delta {
                &mut p.certificate.delta_layouts
            } else {
                &mut p.certificate.layouts
            };
            owners.pop();
        });
    }
}

#[test]
fn duplicate_layout_coverage_has_specific_diagnostic() {
    for delta in [false, true] {
        rejected_with("duplicate layout coverage", |p| {
            let owners = if delta {
                &mut p.certificate.delta_layouts
            } else {
                &mut p.certificate.layouts
            };
            owners[1].index = owners[0].index;
        });
    }
}

#[test]
fn layout_owner_collision_has_specific_diagnostic() {
    for delta in [false, true] {
        rejected_with("layout owner collision", |p| {
            let owners = if delta {
                &mut p.certificate.delta_layouts
            } else {
                &mut p.certificate.layouts
            };
            owners[1].owner = owners[0].owner;
        });
    }
}

#[test]
fn layout_owner_out_of_range_has_specific_diagnostic() {
    for delta in [false, true] {
        rejected_with("layout owner out of range", |p| {
            let owners = if delta {
                &mut p.certificate.delta_layouts
            } else {
                &mut p.certificate.layouts
            };
            owners[0].owner = owners.len();
        });
    }
}

#[test]
fn layout_index_out_of_range_has_specific_diagnostic() {
    for delta in [false, true] {
        rejected_with("layout index out of range", |p| {
            let owners = if delta {
                &mut p.certificate.delta_layouts
            } else {
                &mut p.certificate.layouts
            };
            owners[0].index = owners.len();
        });
    }
}

#[test]
fn missing_barrier_layout_has_specific_diagnostic() {
    for delta in [false, true] {
        rejected_with("missing publication layout", |p| {
            let barrier = p
                .certificate
                .publication
                .iter_mut()
                .find(|barrier| {
                    if delta {
                        !barrier.delta_layouts.is_empty()
                    } else {
                        !barrier.layouts.is_empty()
                    }
                })
                .unwrap();
            if delta {
                barrier.delta_layouts.pop();
            } else {
                barrier.layouts.pop();
            }
        });
    }
}

#[test]
fn wrong_member_barrier_layout_has_specific_diagnostic() {
    for delta in [false, true] {
        rejected_with("publication layout is not a stratum member", |p| {
            let layouts = if delta {
                &p.round.context.delta_layouts
            } else {
                &p.round.context.layouts
            };
            let (barrier, index) = p
                .certificate
                .publication
                .iter_mut()
                .find_map(|barrier| {
                    let members = &p.round.context.strata[barrier.stratum].members;
                    layouts
                        .iter()
                        .position(|layout| !members.contains(&layout.relation))
                        .map(|index| (barrier, index))
                })
                .unwrap();
            if delta {
                barrier.delta_layouts.push(index);
            } else {
                barrier.layouts.push(index);
            }
        });
    }
}

#[test]
fn duplicate_barrier_has_specific_diagnostic() {
    rejected_with("duplicate publication barrier", |p| {
        p.certificate
            .publication
            .push(p.certificate.publication[0].clone());
    });
}

#[test]
fn missing_barrier_has_specific_diagnostic() {
    rejected_with("missing publication barrier", |p| {
        p.certificate.publication.pop();
    });
}

#[test]
fn accepts_nonidentity_owner_permutations_in_each_domain() {
    let source = authored_round();
    for delta in [false, true] {
        let mut target = compile_sharded(&source).unwrap();
        let owners = if delta {
            &mut target.certificate.delta_layouts
        } else {
            &mut target.certificate.layouts
        };
        assert!(owners.len() > 1);
        let count = owners.len();
        for ownership in owners {
            ownership.owner = (ownership.index + 1) % count;
        }
        check_sharded(&source, &target).unwrap();
        Language::check(&target).unwrap();
    }
}

#[test]
fn accepts_reordered_certificate_sets() {
    let source = authored_round();
    let mut target = compile_sharded(&source).unwrap();
    target.certificate.admission.reverse();
    target.certificate.layouts.reverse();
    target.certificate.delta_layouts.reverse();
    target.certificate.publication.reverse();
    for barrier in &mut target.certificate.publication {
        barrier.relations.reverse();
        barrier.layouts.reverse();
        barrier.delta_layouts.reverse();
    }
    check_sharded(&source, &target).unwrap();
    Language::check(&target).unwrap();
}

#[test]
fn default_shards_only_idb_canonical_layouts() {
    let source = authored_storage();
    let target = compile_storage(&source).unwrap();
    check_storage(&source, &target).unwrap();
    assert_eq!(target.providers, source.providers);
    assert!(!target.delta_layouts.is_empty());
    assert_eq!(target.delta_layouts, source.delta_layouts);
    let idb_canonical: Vec<_> = [2, 3]
        .map(|relation| source.providers[relation].canonical)
        .into();
    for (index, (before, after)) in source.layouts.iter().zip(&target.layouts).enumerate() {
        if idb_canonical.contains(&index) {
            // Every canonical layout is keyed by all columns: a key set.
            assert_eq!(before.protocol, storage::LayoutProtocol::Set);
            assert_eq!(after.provider, converted(&source, before, false));
            assert_eq!(after.hasher, before.hasher);
            assert_eq!(after.protocol, storage::LayoutProtocol::Bulk);
        } else {
            assert_eq!(before, after);
        }
    }
    assert_eq!(
        target.layouts[idb_canonical[0]].provider,
        syn::parse_quote!(@sharded_layout as ::cq_datalog_parallel_runtime::ShardedLayout<(i32, i32,), (), ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>)
    );
    assert_eq!(&target.tokens[source.tokens.len()..], [sharded_layout()]);
    assert_eq!(
        syn::parse2::<storage::Program>(target.to_token_stream()).unwrap(),
        target
    );
    assert!(compile_storage_with(&source, quote!(threshold = 3)).is_err());
}

/// The expected bulk counterpart of a base layout: the parallel token's
/// spelling of the layout's recorded schema (sharded keeps the hasher).
fn converted(source: &storage::Program, layout: &storage::Layout, runs: bool) -> storage::Provider {
    let declaration = source
        .incremental
        .strata
        .source
        .declarations
        .iter()
        .find(|declaration| declaration.name == layout.relation)
        .unwrap();
    let (key, payload) = storage::layout_schema(declaration, &layout.columns, layout.key);
    let (token, hasher) = if runs {
        (run_layout(), storage::Hasher::None)
    } else {
        (sharded_layout(), layout.hasher)
    };
    storage::Provider {
        ty: storage::provider::spell(&token, &key, &payload, hasher).unwrap(),
        token: token.token,
    }
}

fn eligible(source: &storage::Program, layout: &storage::Layout) -> bool {
    layout.key > 0
        && source
            .incremental
            .strata
            .source
            .declarations
            .iter()
            .any(|declaration| declaration.name == layout.relation && declaration.value.is_none())
}

#[test]
fn sharded_layouts_rewrite_and_erase_exactly() {
    let source = authored_storage();
    let target = compile_storage_with(&source, quote!(layouts(sharded))).unwrap();
    check_storage(&source, &target).unwrap();
    let mut changed = 0;
    for (before, after) in source
        .layouts
        .iter()
        .chain(&source.delta_layouts)
        .zip(target.layouts.iter().chain(&target.delta_layouts))
    {
        if eligible(&source, before) {
            assert_eq!(after.provider, converted(&source, before, false));
            assert_eq!(after.protocol, storage::LayoutProtocol::Bulk);
            changed += 1;
        } else {
            assert_eq!(before, after);
        }
    }
    assert!(changed > 0);
    compile_sharded(&cq_datalog_fork::compile(&target).unwrap()).unwrap();
    assert_eq!(
        syn::parse2::<storage::Program>(target.to_token_stream()).unwrap(),
        target
    );
    for args in [
        quote!(layouts(native)),
        quote!(layouts()),
        quote!(layouts(sharded), layouts(sharded)),
    ] {
        assert!(compile_storage_with(&source, args).is_err());
    }
}

#[test]
fn sharded_layout_checker_rejects_ineligible_and_changed_records() {
    let source = authored_storage();
    let target = compile_storage_with(&source, quote!(layouts(sharded))).unwrap();
    for delta in [false, true] {
        let layouts = if delta {
            &source.delta_layouts
        } else {
            &source.layouts
        };
        for (index, layout) in layouts.iter().enumerate() {
            if !eligible(&source, layout) {
                let mut changed = target.clone();
                let selected = layout_mut(&mut changed, delta, index);
                selected.provider = converted(&source, layout, false);
                selected.protocol = storage::LayoutProtocol::Bulk;
                assert!(check_storage(&source, &changed).is_err());
                assert!(compile_sharded(&cq_datalog_fork::compile(&changed).unwrap()).is_err());
            } else {
                // A changed spelling, hasher, or token is rejected.
                let mut changed = target.clone();
                layout_mut(&mut changed, delta, index).provider.ty = syn::parse_quote!(Changed);
                assert!(check_storage(&source, &changed).is_err());
                let mut changed = target.clone();
                let hasher = &mut layout_mut(&mut changed, delta, index).hasher;
                *hasher = if *hasher == storage::Hasher::Fx {
                    storage::Hasher::Keyed
                } else {
                    storage::Hasher::Fx
                };
                assert!(check_storage(&source, &changed).is_err());
                let mut changed = target.clone();
                layout_mut(&mut changed, delta, index).provider.token =
                    syn::parse_quote!(custom_layout);
                assert!(check_storage(&source, &changed).is_err());
            }
        }
    }
}

#[test]
fn run_and_sharded_selections_cover_canonical_and_erase_exactly() {
    let source = authored_storage();
    for args in [quote!(), quote!(layouts(runs)), quote!(layouts(sharded),)] {
        let text = args.to_string();
        let runs = text.contains("layouts (runs)");
        let target = compile_storage_with(&source, args).unwrap();
        check_storage(&source, &target).unwrap();
        assert_eq!(target.providers, source.providers);
        let mut restored = target.clone();
        restored.tokens.truncate(source.tokens.len());
        let mut changed = 0;
        for (before, after) in source.layouts.iter().chain(&source.delta_layouts).zip(
            restored
                .layouts
                .iter_mut()
                .chain(&mut restored.delta_layouts),
        ) {
            if before.provider != after.provider {
                assert_eq!(after.provider, converted(&source, before, runs));
                after.provider = before.provider.clone();
                after.hasher = before.hasher;
                assert_eq!(after.protocol, storage::LayoutProtocol::Bulk);
                after.protocol = before.protocol;
                changed += 1;
            }
        }
        // Canonical layouts change under every selection; reads only when selected.
        assert_eq!(changed > 2, text.contains("layouts"));
        assert_eq!(restored, source);
        assert_eq!(
            syn::parse2::<storage::Program>(target.to_token_stream()).unwrap(),
            target
        );
        compile_sharded(&cq_datalog_fork::compile(&target).unwrap()).unwrap();
    }
    for args in [
        quote!(layouts(runs), layouts(sharded)),
        quote!(layouts(sharded), layouts(runs)),
        quote!(layouts(runs), layouts(runs)),
        quote!(seen(runs)),
        quote!(layouts(runs), seen(runs)),
        quote!(layouts(runs extra)),
    ] {
        assert!(compile_storage_with(&source, args).is_err());
    }
}

#[test]
fn run_layouts_make_each_idb_canonical_a_payload_free_run() {
    let source = authored_storage();
    let target = compile_storage_with(&source, quote!(layouts(runs))).unwrap();
    for relation in [2, 3] {
        let canonical = &target.layouts[target.providers[relation].canonical];
        assert_eq!(
            canonical.provider,
            syn::parse_quote!(@run_layout as ::cq_datalog_parallel_runtime::RunLayout<(i32, i32,), ()>)
        );
        assert_eq!(canonical.hasher, storage::Hasher::None);
        assert_eq!(canonical.protocol, storage::LayoutProtocol::Bulk);
    }
    // EDB canonical layouts stay native key sets.
    let edge = &target.layouts[target.providers[0].canonical];
    assert_eq!(edge.protocol, storage::LayoutProtocol::Set);
    assert_eq!(edge, &source.layouts[source.providers[0].canonical]);
}

#[test]
fn layout_substitution_checks_recorded_maintenance_protocol() {
    let source = authored_storage();
    for arguments in [quote!(layouts(sharded)), quote!(layouts(runs))] {
        let target = compile_storage_with(&source, arguments).unwrap();
        for delta in [false, true] {
            let mut changed = target.clone();
            let layouts = if delta {
                &mut changed.delta_layouts
            } else {
                &mut changed.layouts
            };
            let layout = layouts
                .iter_mut()
                .find(|layout| layout.protocol == storage::LayoutProtocol::Bulk)
                .unwrap();
            layout.protocol = storage::LayoutProtocol::GroupedMap;
            assert!(check_storage(&source, &changed).is_err());
        }
    }
}

#[test]
fn run_layout_checker_rejects_ineligible_records_and_nondefault_hashers() {
    let source = authored_storage();
    let target = compile_storage_with(&source, quote!(layouts(runs))).unwrap();
    for delta in [false, true] {
        let layouts = if delta {
            &source.delta_layouts
        } else {
            &source.layouts
        };
        for (index, layout) in layouts.iter().enumerate() {
            if !eligible(&source, layout) {
                let mut changed = target.clone();
                let selected = layout_mut(&mut changed, delta, index);
                selected.provider = converted(&source, layout, true);
                selected.hasher = storage::Hasher::None;
                selected.protocol = storage::LayoutProtocol::Bulk;
                assert!(check_storage(&source, &changed).is_err());
                assert!(compile_sharded(&cq_datalog_fork::compile(&changed).unwrap()).is_err());
                continue;
            }
            let mut changed = target.clone();
            layout_mut(&mut changed, delta, index).provider.ty = syn::parse_quote!(Changed);
            assert!(check_storage(&source, &changed).is_err());
            let mut changed = target.clone();
            layout_mut(&mut changed, delta, index).hasher = storage::Hasher::Fx;
            assert!(check_storage(&source, &changed).is_err());
            // A keyed source layout has no run counterpart; sharding keeps it.
            let mut keyed = source.clone();
            let selected = layout_mut(&mut keyed, delta, index);
            selected.hasher = storage::Hasher::Keyed;
            selected.provider = {
                let declaration = source
                    .incremental
                    .strata
                    .source
                    .declarations
                    .iter()
                    .find(|declaration| declaration.name == layout.relation)
                    .unwrap();
                let (key, payload) =
                    storage::layout_schema(declaration, &layout.columns, layout.key);
                let token =
                    storage::provider::lookup(&source.tokens, &layout.provider.token).unwrap();
                storage::Provider {
                    token: token.token.clone(),
                    ty: storage::provider::spell(token, &key, &payload, storage::Hasher::Keyed)
                        .unwrap(),
                }
            };
            assert!(
                compile_storage_with(&keyed, quote!(layouts(runs)))
                    .unwrap_err()
                    .to_string()
                    .contains("default storage hasher")
            );
            assert!(check_storage(&keyed, &target).is_err());
            compile_storage_with(&keyed, quote!(layouts(sharded))).unwrap();
        }
    }
}

#[test]
fn storage_checker_rejects_unrelated_and_provider_mutations() {
    let mutations: &[fn(&mut storage::Program)] = &[
        // A native IDB canonical layout is valid storage but cannot admit in parallel.
        |p| {
            let source = authored_storage();
            let canonical = source.providers[2].canonical;
            p.layouts[canonical] = source.layouts[canonical].clone();
        },
        |p| p.providers[2].canonical = p.providers[3].canonical,
        |p| {
            let canonical = p.providers[2].canonical;
            p.layouts[canonical].provider.ty =
                syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedLayout<(i32,), ()>)
        },
        |p| p.layouts[0].provider.token = syn::parse_quote!(sharded_layout),
        |p| p.delta_layouts[0].provider.token = syn::parse_quote!(sharded_layout),
        |p| p.layouts[0].hasher = storage::Hasher::None,
        |p| p.delta_layouts[0].hasher = storage::Hasher::Keyed,
        |p| {
            p.tokens.pop();
        },
        |p| p.tokens.push(run_layout()),
        |p| {
            p.layouts.pop();
        },
        |p| {
            p.delta_layouts.pop();
        },
        |p| p.reads[0].position = usize::MAX,
        |p| p.negations.clear(),
    ];
    let source = authored_storage();
    let target = compile_storage(&source).unwrap();
    for (index, mutate) in mutations.iter().enumerate() {
        let mut changed = target.clone();
        mutate(&mut changed);
        assert!(
            check_storage(&source, &changed).is_err(),
            "mutation {index}"
        );
    }
}

fn verbatim(name: &str, protocol: storage::LayoutProtocol) -> storage::provider::Token {
    storage::provider::declared(
        name,
        storage::provider::Spelling::Verbatim,
        Some(protocol),
        storage::provider::Admission::None,
        false,
        storage::provider::TaskHasher::Fx,
    )
}

#[test]
fn storage_preserves_authored_tokens_but_rejects_unsupported_canonical_providers() {
    // An authored grouped-map token keeps its layout and admits in parallel
    // through its declared operations.
    let mut source = authored_storage();
    source
        .tokens
        .push(verbatim("ordered_map", storage::LayoutProtocol::GroupedMap));
    source.layouts[0].provider = syn::parse_quote!(
        @ordered_map as ::std::collections::BTreeMap<(), ::std::vec::Vec<(i32, i32,)>>
    );
    source.layouts[0].hasher = storage::Hasher::None;
    let target = compile_storage(&source).unwrap();
    assert_eq!(target.layouts[0], source.layouts[0]);
    check_storage(&source, &target).unwrap();
    compile_sharded(&cq_datalog_fork::compile(&target).unwrap()).unwrap();
    // An authored canonical token has no sharded counterpart.
    let mut source = authored_storage();
    source
        .tokens
        .push(verbatim("custom_set", storage::LayoutProtocol::Set));
    let canonical = source.providers[2].canonical;
    source.layouts[canonical].provider = syn::parse_quote!(@custom_set as custom::Set<(i32, i32,)>);
    source.layouts[canonical].hasher = storage::Hasher::None;
    assert!(
        compile_storage(&source)
            .unwrap_err()
            .to_string()
            .contains("IDB canonical layouts require")
    );
}

#[test]
fn sharding_preserves_recorded_hashers_and_checks_their_correspondence() {
    let source = authored_storage();
    let keyed = |program: &storage::Program| {
        let mut keyed = program.clone();
        for layout in keyed.layouts.iter_mut().chain(&mut keyed.delta_layouts) {
            let declaration = program
                .incremental
                .strata
                .source
                .declarations
                .iter()
                .find(|declaration| declaration.name == layout.relation)
                .unwrap();
            let (key, payload) = storage::layout_schema(declaration, &layout.columns, layout.key);
            let token = storage::provider::lookup(&program.tokens, &layout.provider.token).unwrap();
            layout.hasher = storage::Hasher::Keyed;
            layout.provider.ty =
                storage::provider::spell(token, &key, &payload, storage::Hasher::Keyed).unwrap();
        }
        keyed
    };
    let source = keyed(&source);
    let target = compile_storage(&source).unwrap();
    let canonical = target.providers[2].canonical;
    assert_eq!(
        target.layouts[canonical].provider,
        syn::parse_quote!(
            @sharded_layout as ::cq_datalog_parallel_runtime::ShardedLayout<(i32, i32,), (), ::cq_storage::HashState>
        )
    );
    assert_eq!(target.layouts[canonical].hasher, storage::Hasher::Keyed);
    check_storage(&source, &target).unwrap();
    compile_sharded(&cq_datalog_fork::compile(&target).unwrap()).unwrap();
    let mut changed = target;
    changed.layouts[canonical].hasher = storage::Hasher::Fx;
    changed.layouts[canonical].provider.ty = syn::parse_quote!(
        ::cq_datalog_parallel_runtime::ShardedLayout<(i32, i32,), (), ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>
    );
    assert!(check_storage(&source, &changed).is_err());
}

#[test]
fn sharded_round_records_threshold_grain_and_lossless_certificate() {
    let source = authored_round();
    let target = compile_sharded(&source).unwrap();
    assert_eq!(target.certificate.threshold, DEFAULT_THRESHOLD);
    assert_eq!(target.certificate.grain, DEFAULT_GRAIN);
    assert_eq!(
        round::erase(&target.round).unwrap(),
        expected_push(source.clone())
    );
    assert_eq!(
        syn::parse2::<ShardedProgram>(target.to_token_stream()).unwrap(),
        target
    );
    Language::check(&target).unwrap();
    let customized = compile_sharded_with(&source, quote!(grain = 32, threshold = 0,)).unwrap();
    assert_eq!(customized.certificate.threshold, 0);
    assert_eq!(customized.certificate.grain, 32);
    Language::check(&customized).unwrap();
    assert_eq!(customized.round, target.round);
    for arguments in [
        quote!(threshold = 1, threshold = 2),
        quote!(grain = 1, grain = 2),
        quote!(grain = 0),
        quote!(threshold = -1),
        quote!(threshold = nope),
        quote!(threshold = 1 grain = 2),
        quote!(threads = 2),
        quote!(threshold = 999999999999999999999999999999999999999999),
    ] {
        assert!(compile_sharded_with(&source, arguments).is_err());
    }
    assert!(compile_sharded(&target.round).is_err());
    let sequential = cq_datalog_fork::compile(&authored_storage()).unwrap();
    assert!(compile_sharded(&sequential).is_err());
}

#[test]
fn task_self_deduplication_is_erasable_and_matches_each_scan_witness() {
    let source = authored_round();
    for policy in [None, Some(syn::parse_quote!(per_task))] {
        let mut target = compile_sharded(&source).unwrap();
        target.certificate.self_dedup = policy.clone();
        for execution in &mut target.round.execution {
            execution.self_dedup = policy.clone();
        }
        check_sharded(&source, &target).unwrap();
        Language::check(&target).unwrap();
        assert_eq!(
            round::erase(&target.round).unwrap(),
            expected_push(source.clone())
        );
        assert_eq!(
            syn::parse2::<ShardedProgram>(target.to_token_stream()).unwrap(),
            target,
        );
        let mut mismatch = target.clone();
        mismatch.round.execution[0].self_dedup = if policy.is_some() {
            None
        } else {
            Some(syn::parse_quote!(per_task))
        };
        assert!(
            check_sharded(&source, &mismatch)
                .unwrap_err()
                .to_string()
                .contains("disagrees with its execution witness")
        );
        target.certificate.self_dedup = Some(syn::parse_quote!(global));
        assert!(check_sharded(&source, &target).is_err());
    }

    let storage = lower_storage(quote! {
        struct GuardFirst;
        relation E(x:i32) = (rows);
        relation R(x:i32);
        output R;
        R(*x % 2) :- if enabled, E(x);
    });
    let storage = compile_storage(&storage).unwrap();
    let source = cq_datalog_fork::compile(&storage).unwrap();
    let mut target = compile_sharded(&source).unwrap();
    assert!(target.round.execution.is_empty());
    target.certificate.self_dedup = Some(syn::parse_quote!(per_task));
    check_sharded(&source, &target).unwrap();
    target.certificate.self_dedup = Some(syn::parse_quote!(global));
    assert!(
        check_sharded(&source, &target)
            .unwrap_err()
            .to_string()
            .contains("must be per_task or absent")
    );
}

#[test]
fn ownership_covers_admission_shards_and_each_whole_layout_once() {
    let source = authored_round();
    let target = compile_sharded(&source).unwrap();
    assert_eq!(
        target
            .certificate
            .admission
            .iter()
            .map(|a| a.relation)
            .collect::<Vec<_>>(),
        vec![2, 3]
    );
    assert_eq!(
        target.certificate.layouts.len(),
        source.context.layouts.len()
    );
    assert_eq!(
        target.certificate.delta_layouts.len(),
        source.context.delta_layouts.len()
    );
    for ownership in target.certificate.admission.iter().map(|a| &a.ownership) {
        assert_eq!(ownership.start, 0);
        assert_eq!(ownership.end, "shard_count");
        assert_eq!(ownership.owner, "shard_id");
    }
    for (owners, layouts) in [
        (&target.certificate.layouts, &source.context.layouts),
        (
            &target.certificate.delta_layouts,
            &source.context.delta_layouts,
        ),
    ] {
        assert_eq!(
            owners
                .iter()
                .map(|owner| (owner.index, owner.owner))
                .collect::<Vec<_>>(),
            (0..layouts.len())
                .map(|index| (index, index))
                .collect::<Vec<_>>()
        );
    }
    rejected(|p| p.certificate.shard_count = syn::parse_quote!(thread_count));
    rejected(|p| p.certificate.admission[0].ownership.start = 1);
    rejected(|p| {
        p.certificate.admission[0].ownership.end = syn::parse_quote!(shard_count_minus_one)
    });
    rejected(|p| p.certificate.admission[0].ownership.owner = syn::parse_quote!(zero));
    rejected(|p| p.certificate.admission[0].hash = syn::parse_quote!(key));
    rejected(|p| {
        p.certificate.admission.pop();
    });
    rejected(|p| {
        p.certificate
            .admission
            .push(p.certificate.admission[0].clone())
    });
    rejected(|p| p.certificate.admission[0].relation = 0);
    rejected(|p| p.certificate.layouts[0].owner = usize::MAX);
    rejected(|p| p.certificate.layouts[1].owner = p.certificate.layouts[0].owner);
    rejected(|p| p.certificate.layouts[0].index = usize::MAX);
    rejected(|p| {
        p.certificate.layouts.pop();
    });
    rejected(|p| p.certificate.layouts.push(p.certificate.layouts[0].clone()));
    rejected(|p| p.certificate.delta_layouts[0].owner = usize::MAX);
    rejected(|p| p.certificate.delta_layouts[0].index = usize::MAX);
    rejected(|p| p.certificate.delta_layouts[1].owner = p.certificate.delta_layouts[0].owner);
    rejected(|p| {
        p.certificate.delta_layouts.pop();
    });
    rejected(|p| {
        p.certificate
            .delta_layouts
            .push(p.certificate.delta_layouts[0].clone())
    });
    rejected(|p| p.certificate.grain = 0);
}

#[test]
fn publication_covers_every_member_layout_before_next_evaluation() {
    let source = authored_round();
    let target = compile_sharded(&source).unwrap();
    for publication in &target.certificate.publication {
        let stratum = &source.context.strata[publication.stratum];
        assert_eq!(publication.relations, stratum.members);
        assert_eq!(publication.barrier, "before_evaluate");
        for (layouts, covered) in [
            (&source.context.layouts, &publication.layouts),
            (&source.context.delta_layouts, &publication.delta_layouts),
        ] {
            assert_eq!(
                *covered,
                layouts
                    .iter()
                    .enumerate()
                    .filter(|(_, layout)| stratum.members.contains(&layout.relation))
                    .map(|(index, _)| index)
                    .collect::<Vec<_>>()
            );
        }
    }
    rejected(|p| {
        p.certificate.publication.pop();
    });
    rejected(|p| {
        p.certificate
            .publication
            .push(p.certificate.publication[0].clone())
    });
    rejected(|p| p.certificate.publication[0].stratum = usize::MAX);
    rejected(|p| p.certificate.publication[0].relations.clear());
    rejected(|p| p.certificate.publication[0].layouts.clear());
    rejected(|p| p.certificate.publication[0].delta_layouts.clear());
    rejected(|p| {
        let publication = &mut p.certificate.publication[0];
        publication.layouts.push(publication.layouts[0]);
    });
    rejected(|p| {
        let publication = &mut p.certificate.publication[0];
        publication.delta_layouts.push(publication.delta_layouts[0]);
    });
    rejected(|p| p.certificate.publication[0].layouts[0] = usize::MAX);
    rejected(|p| p.certificate.publication[0].delta_layouts[0] = usize::MAX);
    rejected(|p| p.certificate.publication[0].barrier = syn::parse_quote!(after_evaluate));
    rejected(|p| p.round.context.strata[0].phases.swap(0, 2));
    rejected(|p| p.round.context.strata[0].publication[0].operations.clear());
    rejected(|p| {
        let operation = p.round.context.strata[0].publication[0]
            .operations
            .iter_mut()
            .find(|op| op.kind == "delta_rebuild")
            .unwrap();
        operation.layouts.clear();
    });
}

#[test]
fn sharded_checker_preserves_opaque_holes_and_exact_scan_coverage() {
    rejected(|p| {
        p.round.execution.pop();
    });
    rejected(|p| p.round.execution.push(p.round.execution[0].clone()));
    rejected(|p| p.round.execution[0].strategy = syn::parse_quote!(unknown));
    rejected(|p| p.round.execution[0].occurrence = 1);
    rejected(|p| p.round.execution[0].binding = syn::parse_quote!(Edge));
    let original = authored_round();
    let mut changed = compile_sharded(&original).unwrap();
    changed.round.holes[0].pipeline.passes[0].arguments = quote!(unrelated);
    // Opaque pass arguments are admissible in a standalone program; only the
    // source-relative refinement checker can reject changing a supplied queue.
    assert!(check_sharded(&original, &changed).is_err());
    Language::check(&changed).unwrap();
    let storage = lower_storage(quote! {
        struct GuardFirst;
        relation E(x:i32) = (edges);
        relation R(x:i32);
        output R;
        R(*x) :- if enabled, E(x);
    });
    let source = cq_datalog_fork::compile(&compile_storage(&storage).unwrap()).unwrap();
    let target = compile_sharded(&source).unwrap();
    assert!(target.round.execution.is_empty());
    assert_eq!(target.round, expected_push(source));
    Language::check(&target).unwrap();
}

#[test]
fn output_only_program_has_no_phantom_owners_or_publications() {
    let storage = lower_storage(quote! {
        struct Input;
        relation E(x:i32) = (edges);
        output E;
    });
    let sharded = compile_storage(&storage).unwrap();
    assert_eq!(sharded, storage);
    let source = cq_datalog_fork::compile(&sharded).unwrap();
    let target = compile_sharded(&source).unwrap();
    assert_eq!(target.round, source);
    assert!(target.certificate.admission.is_empty());
    // The EDB's canonical layout is an ordinary layout with one owner; it is
    // loaded once and never published, so no barrier names it.
    assert_eq!(source.context.layouts.len(), 1);
    assert_eq!(target.certificate.layouts.len(), 1);
    assert!(target.certificate.delta_layouts.is_empty());
    assert!(target.certificate.publication.is_empty());
    Language::check(&target).unwrap();
}

fn layout_mut(program: &mut storage::Program, delta: bool, index: usize) -> &mut storage::Layout {
    if delta {
        &mut program.delta_layouts[index]
    } else {
        &mut program.layouts[index]
    }
}
