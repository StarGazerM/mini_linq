use cq_datalog_fork::{check, compile, compile_with};
use cq_lang_datalog_round::{self as round, storage};
use quote::{ToTokens, quote};
fn source() -> storage::Program {
    let s = syn::parse_quote! {struct Q;relation E(x:i32)=(e);relation R(x:i32);output R;R(*x):-E(x);R(*x):-R(x);};
    let strata = cq_datalog_stratify::compile(&s).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    cq_datalog_storage::compile(&incremental).unwrap()
}
#[test]
fn storage_and_round_are_complete_checked_languages() {
    let source = source();
    let target = compile(&source).unwrap();
    let storage: storage::Program = syn::parse2(source.to_token_stream()).unwrap();
    assert_eq!(storage, source);
    let round: round::Program = syn::parse2(target.to_token_stream()).unwrap();
    assert_eq!(round, target);
    round::contract::check(&round).unwrap();
    assert!(round.execution.is_empty());
}

#[test]
fn default_negation_sets_and_hole_dedup_use_fx() {
    let source = syn::parse_quote! {
        struct Q;
        relation E(x:i32) = (e);
        relation Blocked(x:i32) = (blocked);
        relation R(x:i32);
        output R;
        R(*x) :- E(x), !Blocked(x);
    };
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let storage = cq_datalog_storage::compile(&incremental).unwrap();
    let target = compile(&storage).unwrap();
    let provider: syn::Type = syn::parse_quote!(
        ::std::collections::HashSet<_, ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>
    );
    let keys: Vec<_> = target.context.strata.iter().flat_map(|s| &s.keys).collect();
    assert!(!keys.is_empty());
    assert!(keys.iter().all(|key| key.provider == provider));
    let expected: cq_stage::Pipeline = syn::parse_quote!(pipeline {
        ::mini_linq::passes::plan,
        ::mini_linq::passes::index_demands,
        ::mini_linq::passes::lower,
        ::mini_linq::passes::select,
        ::cq_datalog::cq_rewrites::retype(
            dedup => ::std::collections::HashSet<_, ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>
        ),
        ::mini_linq::passes::represent,
        ::mini_linq::passes::stage,
        ::mini_linq::passes::print_pull
    });
    assert!(!target.holes.is_empty());
    let expected = expected.to_token_stream().to_string();
    assert!(
        target
            .holes
            .iter()
            .all(|hole| { hole.pipeline.to_token_stream().to_string() == expected })
    );
    check(&storage, &target).unwrap();
}
#[test]
fn per_hole_queues_check_terminal_path_not_incidental_tokens() {
    let source = source();
    let target = compile(&source).unwrap();
    let mut changed = target.clone();
    changed.holes[0].pipeline = syn::parse_quote!(pipeline { custom::transform(::mini_linq::passes::print_pull), custom::print });
    assert!(round::contract::check(&changed).is_err());
    changed.holes[0].pipeline =
        syn::parse_quote!(pipeline { custom::transform, ::mini_linq::passes::print_pull });
    round::contract::check(&changed).unwrap();
    round::check_rewrite(&target, &changed).unwrap();
    changed.holes[0].interface = round::HoleInterface::Push;
    assert!(
        round::contract::check(&changed)
            .unwrap_err()
            .to_string()
            .contains("push hole interface is unsupported")
    );
    changed = target.clone();
    changed.holes[0].pipeline.passes.clear();
    assert!(round::contract::check(&changed).is_err());
    changed = target.clone();
    changed.holes[0].pipeline = syn::parse_quote!(pipeline{other::print_pull});
    assert!(round::contract::check(&changed).is_err());
    changed = target.clone();
    changed.holes[0].pipeline =
        syn::parse_quote!(pipeline{mini_linq::passes::print_pull(unchecked)});
    assert!(round::contract::check(&changed).is_err());
}
#[test]
fn fork_arguments_record_queue_and_timing_and_reject_unknown_options() {
    let source = source();
    let target = compile_with(
        &source,
        quote!(timing=phases;cq=pipeline{custom::plan,mini_linq::passes::print_pull};),
    )
    .unwrap();
    assert_eq!(target.policy.timing, "phases");
    check(&source, &target).unwrap();
    assert!(target.holes.iter().all(|h| h.pipeline.passes.len() == 2));
    for args in [
        quote!(unknown=none;),
        quote!(timing=none;timing=phases;),
        quote!(timing=invalid;),
        quote!(cq=pipeline{};),
        quote!(pull_printers=[];),
        quote!(pull_printers=[];pull_printers=[];),
    ] {
        assert!(compile_with(&source, args).is_err());
    }
}
#[test]
fn policy_and_correspondence_mutations_are_rejected() {
    let source = source();
    let target = compile(&source).unwrap();
    let mutations: &[fn(&mut round::Program)] = &[
        |p| p.context.strata[0].phases.reverse(),
        |p| p.context.strata[0].seeds[0].insertion = syn::parse_quote!(bag),
        |p| p.context.strata[0].termination.clear(),
        |p| p.context.strata[0].observer = syn::parse_quote!(after_step),
        |p| p.context.outputs[0].conversion = syn::parse_quote!(borrowed),
        |p| p.context.strata[0].key_scope = syn::parse_quote!(global),
        |p| p.context.strata[0].loop_form = syn::parse_quote!(once),
        |p| p.context.outputs[0].relation = usize::MAX,
        |p| {
            p.execution.push(round::Execution {
                hole: usize::MAX,
                occurrence: 0,
                binding: syn::parse_quote!(fresh),
                strategy: syn::parse_quote!(custom),
            })
        },
        |p| p.context.strata[0].seeds[0].scan = None,
    ];
    for mutate in mutations {
        let mut changed = target.clone();
        mutate(&mut changed);
        assert!(round::contract::check(&changed).is_err());
    }
}

#[test]
fn input_certificates_erase_exactly_and_cannot_hide_other_changes() {
    let source = compile(&source()).unwrap();
    let mut target = source.clone();
    let rule = &source.context.strata[0].seeds[0];
    let original = &rule.scan.as_ref().unwrap().binding;
    let binding: syn::Ident = syn::parse_quote!(fresh_input);
    let hole = &mut target.holes[rule.hole].cq;
    let declaration = hole
        .declarations
        .iter_mut()
        .find(|d| d.name == *original)
        .unwrap();
    declaration.name = binding.clone();
    declaration.value = Some(syn::parse_quote!(= (#binding)));
    let round::cq::BodyItem::Positive { atom } = &mut hole.query.body[0] else {
        unreachable!()
    };
    atom.relation = binding.clone();
    target.execution.push(round::Execution {
        hole: rule.hole,
        occurrence: 0,
        binding,
        strategy: syn::parse_quote!(custom),
    });
    round::contract::check(&target).unwrap();
    round::check_rewrite(&source, &target).unwrap();
    assert_eq!(round::erase(&target).unwrap(), source);
    let replay: round::Program = syn::parse2(target.to_token_stream()).unwrap();
    assert_eq!(replay, target);
    assert!(cq_datalog_print::compile(&target).is_err());
    let changes: &[fn(&mut round::Program)] = &[
        |p| p.execution.push(p.execution[0].clone()),
        |p| p.execution[0].occurrence = 1,
        |p| p.execution[0].binding = syn::parse_quote!(E),
        |p| p.execution[0].hole = usize::MAX,
        |p| p.holes[0].cq.query.head.args[0] = syn::parse_quote!(7),
        |p| p.holes[0].cq.query.body[0] = syn::parse_quote!(E(x)),
    ];
    for mutate in changes {
        let mut changed = target.clone();
        mutate(&mut changed);
        assert!(round::contract::check(&changed).is_err());
        assert!(round::check_rewrite(&source, &changed).is_err());
        assert!(round::check_rewrite(&changed, &source).is_err());
    }
}

#[test]
fn reusable_printer_splices_and_delegates_only_phase_evaluation() {
    let program = compile(&source()).unwrap();
    let holes = cq_datalog_print::splice_with(&program, |_| Ok(quote!(rows()))).unwrap();
    let mut phases = Vec::new();
    let printed =
        cq_datalog_print::render_with(&program, &holes, |rules, context, holes, execution| {
            assert!(execution.is_empty());
            phases.push(rules.iter().map(|r| r.hole).collect::<Vec<_>>());
            Ok(cq_datalog_print::evaluate(rules, context, holes, false))
        })
        .unwrap();
    assert_eq!(phases.len(), 2);
    let text = printed.to_token_stream().to_string();
    assert!(text.contains("Canonical"));
    assert!(text.contains("while"));
    assert!(text.contains("into_iter"));
    assert!(cq_datalog_print::render_with(&program, &[], |_, _, _, _| unreachable!()).is_err());
}

#[test]
fn declared_pull_paths_and_typed_interfaces_survive_round_text() {
    let source = source();
    let defaults = compile(&source).unwrap();
    assert_eq!(
        defaults.pull_printers.0,
        vec![
            syn::parse_quote!(::mini_linq::passes::print_pull),
            syn::parse_quote!(mini_linq::passes::print_pull),
        ]
    );
    let target = compile_with(
        &source,
        quote! {
            cq = pipeline { crate::alias::pull };
            pull_printers = [crate::alias::pull, ::declared::unused_pull];
        },
    )
    .unwrap();
    assert!(
        target
            .holes
            .iter()
            .all(|hole| hole.interface == round::HoleInterface::Pull)
    );
    let replay: round::Program = syn::parse2(target.to_token_stream()).unwrap();
    assert_eq!(replay, target);
    round::contract::check(&replay).unwrap();
    check(&source, &replay).unwrap();
    assert_eq!(replay.pull_printers.0.len(), 2);
    for terminal in [
        quote!(unlisted::pull),
        quote!(::crate::alias::pull),
        quote!(crate::alias::pull(unchecked)),
    ] {
        let mut changed = replay.clone();
        changed.holes[0].pipeline = syn::parse2(quote!(pipeline { #terminal })).unwrap();
        assert!(round::contract::check(&changed).is_err());
    }
    let mut changed = replay.clone();
    changed.pull_printers.0.clear();
    assert!(round::contract::check(&changed).is_err());
    let mut changed = replay;
    changed.holes[0].interface = round::HoleInterface::Push;
    let text = changed.to_token_stream();
    let changed: round::Program = syn::parse2(text).unwrap();
    assert_eq!(changed.holes[0].interface, round::HoleInterface::Push);
    assert!(round::contract::check(&changed).is_err());
}

#[test]
fn default_allowlist_rejects_push_terminals() {
    let source = source();
    for path in [
        quote!(mini_linq::passes::print_push),
        quote!(::mini_linq::passes::print_push),
    ] {
        let error = compile_with(&source, quote!(cq = pipeline { #path };)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("recorded pull_printers allowlist")
        );
        let mut changed = compile(&source).unwrap();
        changed.holes[0].pipeline = syn::parse2(quote!(pipeline { #path })).unwrap();
        assert!(
            round::contract::check(&changed)
                .unwrap_err()
                .to_string()
                .contains("recorded pull_printers allowlist")
        );
    }
}

fn mutual() -> round::Program {
    let source = syn::parse_quote! {
        struct Mutual;
        relation E(x:i32,y:i32) = (edges);
        relation A(x:i32,y:i32);
        relation B(x:i32,y:i32);
        output A, B, E;
        A(*x,*y) :- E(x,y);
        A(*x,*z) :- A(x,y), B(y,z);
        B(*x,*y) :- A(x,y);
    };
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    compile(&cq_datalog_storage::compile(&incremental).unwrap()).unwrap()
}

#[test]
fn publication_permutations_and_loop_forms_are_checked_records() {
    let source = mutual();
    let mut target = source.clone();
    let stratum = &mut target.context.strata[0];
    assert_eq!(stratum.members.len(), 2);
    stratum.publication.reverse();
    stratum.termination.reverse();
    stratum.termination_test = syn::parse_quote!(any_delta);
    stratum.loop_form = syn::parse_quote!(loop_until_empty);
    // All extension may precede full-layout updates: both finish before swap.
    for publication in &mut stratum.publication {
        if publication.operations[0].kind == "full_layouts" {
            publication.operations.swap(0, 1);
        }
        for operation in &mut publication.operations {
            operation.layouts.reverse();
        }
    }
    round::contract::check(&target).unwrap();
    round::check_rewrite(&source, &target).unwrap();
    check(&source.storage, &target).unwrap();
    let replay: round::Program = syn::parse2(target.to_token_stream()).unwrap();
    assert_eq!(target, replay);
    round::contract::check(&replay).unwrap();
}

#[test]
fn publication_obligations_reject_incomplete_or_stale_rounds() {
    let source = mutual();
    assert!(!source.context.delta_layouts.is_empty());
    let changes: &[fn(&mut round::Program)] = &[
        |p| {
            p.context.strata[0].publication.pop();
        },
        |p| {
            let first = p.context.strata[0].publication[0].clone();
            p.context.strata[0].publication.push(first);
        },
        |p| p.context.strata[0].publication[0].relation = usize::MAX,
        |p| {
            let first = p.context.strata[0].termination[0];
            p.context.strata[0].termination.push(first);
        },
        |p| p.context.strata[0].termination[0] = usize::MAX,
        |p| p.context.strata[0].termination_test = syn::parse_quote!(first_delta),
        |p| {
            for member in &mut p.context.strata[0].publication {
                member
                    .operations
                    .retain(|operation| operation.kind != "all_extend");
            }
        },
        |p| {
            for member in &mut p.context.strata[0].publication {
                member
                    .operations
                    .retain(|operation| operation.kind != "delta_swap");
            }
        },
        |p| {
            for member in &mut p.context.strata[0].publication {
                member
                    .operations
                    .retain(|operation| operation.kind != "delta_rebuild");
            }
        },
        |p| {
            for member in &mut p.context.strata[0].publication {
                member
                    .operations
                    .retain(|operation| operation.kind != "full_layouts");
            }
        },
        |p| p.context.strata[0].publication[0].operations.reverse(),
        |p| {
            let operation = p.context.strata[0].publication[0].operations[0].clone();
            p.context.strata[0].publication[0]
                .operations
                .push(operation);
        },
        |p| p.context.strata[0].publication[0].operations[0].kind = syn::parse_quote!(if_nonempty),
        |p| {
            let operation = p.context.strata[0]
                .publication
                .iter_mut()
                .flat_map(|member| &mut member.operations)
                .find(|operation| operation.kind == "delta_rebuild")
                .unwrap();
            operation.layouts.push(operation.layouts[0]);
        },
        |p| {
            let operation = p.context.strata[0]
                .publication
                .iter_mut()
                .flat_map(|member| &mut member.operations)
                .find(|operation| operation.kind == "delta_rebuild")
                .unwrap();
            operation.layouts[0] = usize::MAX;
        },
        |p| {
            let operation = p.context.strata[0]
                .publication
                .iter_mut()
                .flat_map(|member| &mut member.operations)
                .find(|operation| operation.kind == "all_extend")
                .unwrap();
            operation.layouts.push(0);
        },
        |p| p.context.outputs[2].conversion = syn::parse_quote!(owned_into_iter),
        |p| p.context.relations[1].providers.all = syn::parse_quote!(UnrecordedProvider),
        |p| p.context.layouts[0].provider = syn::parse_quote!(UnrecordedProvider),
        |p| p.context.layouts[0].maintenance = syn::parse_quote!(never),
        |p| p.context.delta_layouts[0].provider = syn::parse_quote!(UnrecordedProvider),
        |p| p.context.delta_layouts[0].maintenance = syn::parse_quote!(when_nonempty),
    ];
    for (index, change) in changes.iter().enumerate() {
        let mut target = source.clone();
        change(&mut target);
        assert!(
            round::contract::check(&target).is_err(),
            "publication mutation {index}"
        );
        assert!(
            round::check_rewrite(&source, &target).is_err(),
            "rewrite mutation {index}"
        );
    }
}

#[test]
fn later_direct_delta_remains_a_whole_delta_input() {
    let mut storage = mutual().storage;
    let declarations = &storage.incremental.strata.source.declarations;
    let mut changed = false;
    for read in &mut storage.reads {
        if read.kind == "delta" && read.position > 0 {
            let layout = &storage.delta_layouts[read.index];
            let relation = declarations
                .iter()
                .position(|declaration| declaration.name == layout.relation)
                .unwrap();
            read.kind = syn::parse_quote!(direct);
            read.index = relation;
            read.columns = (0..declarations[relation].arity()).collect();
            changed = true;
        }
    }
    assert!(changed);
    storage::contract::check(&storage).unwrap();
    let target = compile(&storage).unwrap();
    check(&storage, &target).unwrap();
    for read in storage
        .reads
        .iter()
        .filter(|read| read.kind == "direct" && read.position > 0)
    {
        let rule = target
            .context
            .strata
            .iter()
            .flat_map(|stratum| &stratum.steps)
            .find(|rule| rule.variant == read.variant)
            .unwrap();
        let round::cq::BodyItem::Positive { atom } =
            &target.holes[rule.hole].cq.query.body[read.position]
        else {
            panic!("direct delta occurrence must remain positive");
        };
        assert_eq!(
            Some(&atom.relation),
            target.context.relations[read.index].delta.as_ref()
        );
    }
    let replay: round::Program = syn::parse2(target.to_token_stream()).unwrap();
    assert_eq!(replay, target);
}

#[test]
fn execution_records_have_authored_lossless_grammar() {
    let publication: round::Publication = syn::parse_quote! {
        relation { 2 }
        operations { [
            { kind { full_layouts } layouts { [{ 3 } { 1 }] } }
            { kind { all_extend } layouts { [] } }
            { kind { delta_swap } layouts { [] } }
            { kind { delta_rebuild } layouts { [{ 0 }] } }
        ] }
    };
    assert_eq!(publication.relation, 2);
    assert_eq!(publication.operations[0].layouts, [3, 1]);
    assert_eq!(publication.operations[2].kind, "delta_swap");
    assert_eq!(
        syn::parse2::<round::Publication>(publication.to_token_stream()).unwrap(),
        publication
    );
    let output: round::Output = syn::parse_quote! {
        relation { 1 }
        conversion { owned_into_iter }
    };
    assert_eq!(
        syn::parse2::<round::Output>(output.to_token_stream()).unwrap(),
        output
    );
    assert!(
        syn::parse2::<round::Publication>(quote! {
            relation { 2 } operations { [{ kind { delta_swap } layouts { [] } extra { 0 } }] }
        })
        .is_err()
    );
    assert!(
        syn::parse2::<round::Output>(quote! {
            relation { 1 } conversion { owned_into_iter unexpected }
        })
        .is_err()
    );
}
