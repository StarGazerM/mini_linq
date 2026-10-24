//! Storage provider erasure, admission shards, and exclusive layout publication.
use cq_datalog_partition::{
    DEFAULT_GRAIN, DEFAULT_THRESHOLD, ShardedProgram, check_sharded, check_storage,
    compile_sharded, compile_sharded_with, compile_storage, compile_storage_with,
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
fn storage_rewrites_only_idb_seen_and_preserves_all_native_layouts() {
    let mut source = authored_storage();
    // Opaque Rust syntax unrelated to sharding must survive without rewriting.
    source.providers[2].all_init = syn::parse_quote!(make_rows());
    let target = compile_storage(&source).unwrap();
    check_storage(&source, &target).unwrap();
    assert_eq!(target.providers[0], source.providers[0]);
    assert_eq!(target.providers[1], source.providers[1]);
    assert_eq!(
        target.providers[2].seen,
        syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedSeen<(i32, i32,), ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>)
    );
    assert!(!target.delta_layouts.is_empty());
    assert_eq!(target.layouts, source.layouts);
    assert_eq!(target.delta_layouts, source.delta_layouts);
    let mut restored = target.clone();
    for (original, rewritten) in source.providers.iter().zip(&mut restored.providers) {
        rewritten.seen = original.seen.clone();
    }
    assert_eq!(restored, source);
    assert_eq!(
        syn::parse2::<storage::Program>(target.to_token_stream()).unwrap(),
        target
    );
    assert!(compile_storage_with(&source, quote!(threshold = 3)).is_err());
}

fn shard_layout_type(provider: &syn::Type) -> syn::Type {
    let syn::Type::Path(path) = provider else {
        panic!("map provider")
    };
    let syn::PathArguments::AngleBracketed(args) = &path.path.segments.last().unwrap().arguments
    else {
        panic!("map arguments")
    };
    let key = &args.args[0];
    let syn::GenericArgument::Type(syn::Type::Path(values)) = &args.args[1] else {
        panic!("vector payload")
    };
    let syn::PathArguments::AngleBracketed(payload) =
        &values.path.segments.last().unwrap().arguments
    else {
        panic!("vector arguments")
    };
    let value = &payload.args[0];
    let hasher = &args.args[2];
    syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedLayout<#key, #value, #hasher>)
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
        if before.key > 0
            && source
                .incremental
                .strata
                .source
                .declarations
                .iter()
                .any(|declaration| {
                    declaration.name == before.relation && declaration.value.is_none()
                })
        {
            assert_eq!(after.provider, shard_layout_type(&before.provider));
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
fn sharded_layout_checker_rejects_ineligible_and_changed_types() {
    let source = authored_storage();
    let target = compile_storage_with(&source, quote!(layouts(sharded))).unwrap();
    for delta in [false, true] {
        let layouts = if delta {
            &source.delta_layouts
        } else {
            &source.layouts
        };
        for (index, layout) in layouts.iter().enumerate() {
            let eligible = layout.key > 0
                && source
                    .incremental
                    .strata
                    .source
                    .declarations
                    .iter()
                    .any(|declaration| {
                        declaration.name == layout.relation && declaration.value.is_none()
                    });
            if !eligible {
                let mut changed = target.clone();
                let layouts = if delta {
                    &mut changed.delta_layouts
                } else {
                    &mut changed.layouts
                };
                layouts[index].provider = shard_layout_type(&layout.provider);
                assert!(check_storage(&source, &changed).is_err());
                assert!(compile_sharded(&cq_datalog_fork::compile(&changed).unwrap()).is_err());
            } else {
                for argument in 0..3 {
                    let mut changed = target.clone();
                    let layouts = if delta {
                        &mut changed.delta_layouts
                    } else {
                        &mut changed.layouts
                    };
                    let syn::Type::Path(path) = &mut layouts[index].provider else {
                        unreachable!()
                    };
                    let syn::PathArguments::AngleBracketed(args) =
                        &mut path.path.segments.last_mut().unwrap().arguments
                    else {
                        unreachable!()
                    };
                    args.args[argument] = syn::parse_quote!(Changed);
                    assert!(check_storage(&source, &changed).is_err());
                }
                let mut changed = target.clone();
                let layouts = if delta {
                    &mut changed.delta_layouts
                } else {
                    &mut changed.layouts
                };
                let syn::Type::Path(path) = &mut layouts[index].provider else {
                    unreachable!()
                };
                path.path.leading_colon = None;
                assert!(check_storage(&source, &changed).is_err());
                assert!(compile_sharded(&cq_datalog_fork::compile(&changed).unwrap()).is_err());
            }
        }
    }
}

#[test]
fn storage_checker_rejects_unrelated_and_provider_mutations() {
    let mutations: &[fn(&mut storage::Program)] = &[
        |p| p.providers[2].seen = syn::parse_quote!(::cq_storage::HashSet<(i32, i32,)>),
        |p| {
            p.providers[2].seen =
                syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedSeen<(i32,)>)
        },
        |p| p.providers[2].all_init = syn::parse_quote!(other_rows()),
        |p| p.providers[0].canonical_name = syn::parse_quote!(OtherCanonical),
        |p| {
            p.layouts[0].provider =
                syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedIndex<(), (i32, i32,)>)
        },
        |p| {
            p.delta_layouts[0].provider =
                syn::parse_quote!(::cq_datalog_parallel_runtime::ShardedIndex<(), (i32, i32,)>)
        },
        |p| {
            p.layouts[0].provider =
                syn::parse_quote!(::std::collections::HashMap<(), ::std::vec::Vec<(i32, i32,)>>)
        },
        |p| {
            p.delta_layouts[0].provider =
                syn::parse_quote!(::std::collections::HashMap<(), ::std::vec::Vec<(i32, i32,)>>)
        },
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
    for mutate in mutations {
        let mut changed = target.clone();
        mutate(&mut changed);
        assert!(check_storage(&source, &changed).is_err());
    }
}

#[test]
fn storage_preserves_layout_shapes_but_rejects_unsupported_seen_providers() {
    let mut source = authored_storage();
    source.layouts[0].provider =
        syn::parse_quote!(::std::collections::BTreeMap<(), ::std::vec::Vec<(i32, i32,)>>);
    let target = compile_storage(&source).unwrap();
    assert_eq!(target.layouts, source.layouts);
    check_storage(&source, &target).unwrap();
    let round = cq_datalog_fork::compile(&target).unwrap();
    assert!(compile_sharded(&round).is_err());
    let mut source = authored_storage();
    source.providers[2].seen = syn::parse_quote!(custom::Set<(i32, i32,)>);
    assert!(compile_storage(&source).is_err());
}

#[test]
fn sharding_preserves_explicit_hashers_and_checks_their_correspondence() {
    let mut source = authored_storage();
    source.providers[2].seen = syn::parse_quote!(
        ::std::collections::HashSet<(i32, i32,), ::std::hash::BuildHasherDefault<::rustc_hash::FxHasher>>
    );
    for layout in source.layouts.iter_mut().chain(&mut source.delta_layouts) {
        let syn::Type::Path(path) = &mut layout.provider else {
            unreachable!()
        };
        let syn::PathArguments::AngleBracketed(arguments) =
            &mut path.path.segments.last_mut().unwrap().arguments
        else {
            unreachable!()
        };
        arguments.args[2] =
            syn::parse_quote!(::std::hash::BuildHasherDefault<::rustc_hash::FxHasher>);
    }
    let target = compile_storage(&source).unwrap();
    assert_eq!(
        target.providers[2].seen,
        syn::parse_quote!(
            ::cq_datalog_parallel_runtime::ShardedSeen<(i32, i32,), ::std::hash::BuildHasherDefault<::rustc_hash::FxHasher>>
        )
    );
    check_storage(&source, &target).unwrap();
    compile_sharded(&cq_datalog_fork::compile(&target).unwrap()).unwrap();
    let mut changed = target;
    changed.providers[2].seen = syn::parse_quote!(
        ::cq_datalog_parallel_runtime::ShardedSeen<(i32, i32,), ::cq_storage::HashState>
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
    assert!(target.certificate.layouts.is_empty());
    assert!(target.certificate.delta_layouts.is_empty());
    assert!(target.certificate.publication.is_empty());
    Language::check(&target).unwrap();
}
