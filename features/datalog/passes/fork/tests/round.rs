use cq_datalog_fork::{check, compile, compile_with, resolve_cq};
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
fn default_negation_sets_use_fx_without_a_hole_dedup_override() {
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
        ::mini_linq::passes::represent,
        ::mini_linq::passes::stage,
        ::mini_linq::passes::sequence,
        ::mini_linq::passes::sequential,
        ::mini_linq::passes::translate_pull
    });
    assert!(!target.holes.is_empty());
    for hole in &target.holes {
        let mut expected = expected.clone();
        expected.passes[hole.lower].arguments = hole.facts.to_token_stream();
        assert_eq!(
            hole.pipeline.to_token_stream().to_string(),
            expected.to_token_stream().to_string()
        );
        assert!(hole.facts.consumer_canonical);
    }
    check(&storage, &target).unwrap();
    let batch = compile_with(
        &storage,
        quote! {
            terminals = [custom::batch => batch];
            cq = pipeline { custom::lower, custom::batch };
            lower = 0;
        },
    )
    .unwrap();
    for key in batch
        .context
        .strata
        .iter()
        .flat_map(|stratum| &stratum.keys)
    {
        let query = batch.holes[key.hole].cq.to_token_stream().to_string();
        assert!(query.contains(": i32"));
        assert!(!query.contains(": & i32"));
        assert!(query.contains("*"));
    }
    check(&storage, &batch).unwrap();
}

#[test]
fn explicit_push_skips_sequence_defaults_but_pull_inherits_them() {
    for (terminal, sequence) in [
        (quote!(::cq_datalog::passes::print_push), false),
        (quote!(::mini_linq::passes::translate_pull), true),
    ] {
        let input = if sequence {
            quote!(sequence)
        } else {
            quote!(structures)
        };
        let configuration =
            syn::parse2(quote!(pipeline { print = #terminal; print_input = #input; })).unwrap();
        let queue = resolve_cq(configuration).unwrap();
        let sequence_path: syn::Path = syn::parse_quote!(::mini_linq::passes::sequence);
        let grade_path: syn::Path = syn::parse_quote!(::mini_linq::passes::sequential);
        assert_eq!(
            queue.passes.iter().any(|pass| pass.path == sequence_path),
            sequence
        );
        assert_eq!(
            queue.passes.iter().any(|pass| pass.path == grade_path),
            sequence
        );
        assert_eq!(
            queue.passes.last().unwrap().path,
            syn::parse2::<syn::Path>(terminal).unwrap()
        );
    }
}
#[test]
fn per_hole_queues_check_terminal_path_not_incidental_tokens() {
    let source = source();
    let target = compile(&source).unwrap();
    let mut changed = target.clone();
    changed.holes[0].pipeline = syn::parse_quote!(pipeline { custom::transform(::mini_linq::passes::translate_pull), custom::print });
    assert!(round::contract::check(&changed).is_err());
    let facts = &changed.holes[0].facts;
    changed.holes[0].pipeline = syn::parse_quote!(pipeline { custom::transform, custom::lower(#facts), ::mini_linq::passes::translate_pull });
    changed.holes[0].lower = 1;
    round::contract::check(&changed).unwrap();
    round::check_rewrite(&target, &changed).unwrap();
    changed.holes[0].interface = round::HoleInterface::Push;
    assert!(
        round::contract::check(&changed)
            .unwrap_err()
            .to_string()
            .contains("hole interface does not match its registered terminal interface")
    );
    changed = target.clone();
    changed.holes[0].pipeline.passes.clear();
    assert!(round::contract::check(&changed).is_err());
    changed = target.clone();
    changed.holes[0].pipeline = syn::parse_quote!(pipeline{other::translate_pull});
    assert!(round::contract::check(&changed).is_err());
    changed = target.clone();
    let facts = &changed.holes[0].facts;
    changed.holes[0].pipeline = syn::parse_quote!(pipeline{custom::lower(#facts), mini_linq::passes::translate_pull(unchecked)});
    changed.holes[0].lower = 0;
    changed.holes[0].plan = None;
    round::check_rewrite(&target, &changed).unwrap();
}
#[test]
fn fork_arguments_record_queue_and_timing_and_reject_unknown_options() {
    let source = source();
    let target = compile_with(
        &source,
        quote!(timing=phases;cq=pipeline{custom::plan,custom::lower,mini_linq::passes::translate_pull};plan=0;lower=1;),
    )
    .unwrap();
    assert_eq!(target.policy.timing, "phases");
    check(&source, &target).unwrap();
    assert!(
        target
            .holes
            .iter()
            .all(|h| { h.pipeline.passes.len() == 3 && h.plan == Some(0) && h.lower == 1 })
    );
    for args in [
        quote!(unknown=none;),
        quote!(timing=none;timing=phases;),
        quote!(timing=invalid;),
        quote!(cq=pipeline{};),
        quote!(terminals=[];),
        quote!(terminals=[];terminals=[];),
        quote!(terminals=[printer => pull, printer => push];),
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
                self_dedup: None,
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
    target.holes[rule.hole].rename_host_fact(original, &binding);
    target.execution.push(round::Execution {
        hole: rule.hole,
        occurrence: 0,
        binding,
        strategy: syn::parse_quote!(custom),
        self_dedup: None,
    });
    round::contract::check(&target).unwrap();
    round::check_rewrite(&source, &target).unwrap();
    assert_eq!(round::erase(&target).unwrap(), source);
    let replay: round::Program = syn::parse2(target.to_token_stream()).unwrap();
    assert_eq!(replay, target);
    assert!(cq_datalog_print::compile(&target).is_err());
    // Task-local suppression is an erasable realization, not another owner.
    let mut suppressed = target.clone();
    suppressed.execution[0].strategy = syn::parse_quote!(rayon_sharded);
    suppressed.execution[0].self_dedup = Some(syn::parse_quote!(per_task));
    round::contract::check(&suppressed).unwrap();
    round::check_rewrite(&target, &suppressed).unwrap();
    assert_eq!(round::erase(&suppressed).unwrap(), source);
    assert_eq!(suppressed.context, target.context);
    assert_eq!(suppressed.holes, target.holes);
    let replay: round::Program = syn::parse2(suppressed.to_token_stream()).unwrap();
    assert_eq!(replay, suppressed);
    assert!(cq_datalog_print::compile(&suppressed).is_err());
    let changes: &[fn(&mut round::Program)] = &[
        |p| p.execution[0].self_dedup = Some(syn::parse_quote!(global)),
        |p| p.execution[0].strategy = syn::parse_quote!(custom),
        |p| p.context.relations[1].admit.clear(),
        |p| p.holes[0].facts.consumer_canonical = false,
    ];
    for change in changes {
        let mut changed = suppressed.clone();
        change(&mut changed);
        assert!(round::contract::check(&changed).is_err());
        assert!(round::check_rewrite(&suppressed, &changed).is_err());
    }
    let mut invalid_mode = suppressed.clone();
    invalid_mode.execution[0].self_dedup = Some(syn::parse_quote!(unknown));
    assert!(round::erase(&invalid_mode).is_err());
    let mut stale_facts = target.clone();
    stale_facts.holes[rule.hole].facts = source.holes[rule.hole].facts.clone();
    let hole = &mut stale_facts.holes[rule.hole];
    hole.pipeline.passes[hole.lower].arguments = hole.facts.to_token_stream();
    assert!(round::contract::check(&stale_facts).is_err());
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
            Ok(cq_datalog_print::evaluate(
                rules,
                context,
                holes,
                &program.holes,
                false,
            ))
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
fn declared_terminal_paths_and_typed_interfaces_survive_round_text() {
    let source = source();
    let defaults = compile(&source).unwrap();
    assert_eq!(
        defaults.terminals,
        syn::parse_quote!([
            ::mini_linq::passes::translate_pull => pull,
            mini_linq::passes::translate_pull => pull,
            ::cq_datalog::passes::print_push => push
        ])
    );
    let target = compile_with(
        &source,
        quote! {
            cq = pipeline { custom::lower, crate::alias::pull };
            lower = 0;
            terminals = [crate::alias::pull => pull, ::declared::unused_push => push];
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
    assert_eq!(replay.terminals.0.len(), 2);
    for terminal in [quote!(unlisted::pull), quote!(::crate::alias::pull)] {
        let mut changed = replay.clone();
        changed.holes[0].pipeline = syn::parse2(quote!(pipeline { #terminal })).unwrap();
        assert!(round::contract::check(&changed).is_err());
    }
    let mut changed = replay.clone();
    changed.terminals.0.clear();
    assert!(round::contract::check(&changed).is_err());
    let mut changed = replay;
    changed.holes[0].interface = round::HoleInterface::Push;
    let text = changed.to_token_stream();
    let changed: round::Program = syn::parse2(text).unwrap();
    assert_eq!(changed.holes[0].interface, round::HoleInterface::Push);
    assert!(round::contract::check(&changed).is_err());
}

#[test]
fn default_registry_rejects_unregistered_push_terminals() {
    let source = source();
    for path in [
        quote!(mini_linq::passes::print_push),
        quote!(::mini_linq::passes::print_push),
    ] {
        let error = compile_with(&source, quote!(cq = pipeline { #path };)).unwrap_err();
        assert!(error.to_string().contains("recorded terminal registry"));
        let mut changed = compile(&source).unwrap();
        changed.holes[0].pipeline = syn::parse2(quote!(pipeline { #path })).unwrap();
        assert!(
            round::contract::check(&changed)
                .unwrap_err()
                .to_string()
                .contains("recorded terminal registry")
        );
    }
}

#[test]
fn fork_infers_both_interfaces_and_round_rewrites_preserve_the_registry() {
    let storage = source();
    for (terminal, interface) in [
        (
            quote!(::mini_linq::passes::translate_pull),
            round::HoleInterface::Pull,
        ),
        (
            quote!(::cq_datalog::passes::print_push),
            round::HoleInterface::Push,
        ),
    ] {
        let source = compile_with(
            &storage,
            quote!(cq = pipeline { custom::lower, #terminal }; lower = 0;),
        )
        .unwrap();
        assert!(source.holes.iter().all(|hole| hole.interface == interface));
        let replay: round::Program = syn::parse2(source.to_token_stream()).unwrap();
        assert_eq!(replay, source);
        round::contract::check(&replay).unwrap();
        check(&storage, &replay).unwrap();

        let mut changed = source.clone();
        for hole in &mut changed.holes {
            let facts = &hole.facts;
            hole.pipeline = syn::parse2(quote!(pipeline {
                custom::plan(any(tokens)), custom::rewrite(changed = arguments),
                custom::lower(#facts), #terminal(terminal = arguments)
            }))
            .unwrap();
            hole.lower = 2;
        }
        round::check_rewrite(&source, &changed).unwrap();

        let mut missing = changed.clone();
        missing.holes[0].pipeline.passes.last_mut().unwrap().path =
            syn::parse_quote!(unregistered::terminal);
        assert!(round::check_rewrite(&source, &missing).is_err());

        let mut mismatched = changed.clone();
        mismatched.holes[0].pipeline.passes.last_mut().unwrap().path = match interface {
            round::HoleInterface::Pull => syn::parse_quote!(::cq_datalog::passes::print_push),
            round::HoleInterface::Push | round::HoleInterface::Batch => {
                syn::parse_quote!(::mini_linq::passes::translate_pull)
            }
        };
        assert!(round::check_rewrite(&source, &mismatched).is_err());
        mismatched.holes[0].interface = match interface {
            round::HoleInterface::Pull => round::HoleInterface::Push,
            round::HoleInterface::Push | round::HoleInterface::Batch => round::HoleInterface::Pull,
        };
        round::contract::check(&mismatched).unwrap();
        round::check_rewrite(&source, &mismatched).unwrap();
        round::check_rewrite(&mismatched, &source).unwrap();
        assert_eq!(mismatched.holes[0].facts, source.holes[0].facts);
        assert_eq!(mismatched.context, source.context);

        let mut altered_cq = changed.clone();
        altered_cq.holes[0].cq.name = syn::parse_quote!(ChangedHole);
        assert!(round::check_rewrite(&source, &altered_cq).is_err());

        changed
            .terminals
            .0
            .push(syn::parse_quote!(unused::terminal => pull));
        round::contract::check(&changed).unwrap();
        assert!(round::check_rewrite(&source, &changed).is_err());
        let duplicate = changed.terminals.0[0].clone();
        changed.terminals.0.push(duplicate);
        assert!(round::contract::check(&changed).is_err());
        assert!(syn::parse2::<round::Program>(changed.to_token_stream()).is_err());
    }

    let alias = compile_with(
        &storage,
        quote! {
            terminals = [crate::alias::push => push];
            cq = pipeline { custom::lower, crate::alias::push };
            lower = 0;
        },
    )
    .unwrap();
    assert!(
        alias
            .holes
            .iter()
            .all(|hole| hole.interface == round::HoleInterface::Push)
    );
    let replay: round::Program = syn::parse2(alias.to_token_stream()).unwrap();
    assert_eq!(alias, replay);
    round::contract::check(&replay).unwrap();
}

#[test]
fn batch_registry_round_trips_and_fork_records_its_interface() {
    let registry: round::TerminalRegistry = syn::parse_quote!([
        ::cq_gpu_sequence::translate_arena => batch
    ]);
    let replay: round::TerminalRegistry = syn::parse2(registry.to_token_stream()).unwrap();
    assert_eq!(registry, replay);
    let program = compile_with(
        &source(),
        quote! {
            terminals = #registry;
            cq = pipeline { custom::lower, ::cq_gpu_sequence::translate_arena };
            lower = 0;
        },
    )
    .unwrap();
    assert!(!program.holes.is_empty());
    assert!(
        program
            .holes
            .iter()
            .all(|hole| hole.interface == round::HoleInterface::Batch)
    );
    let replay: round::Program = syn::parse2(program.to_token_stream()).unwrap();
    assert_eq!(program, replay);
    round::contract::check(&replay).unwrap();
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
    for mode in [quote!([]), quote!([per_task])] {
        let execution: round::Execution = syn::parse2(quote! {
            hole { 0 }
            occurrence { 0 }
            binding { task_rows }
            strategy { rayon_sharded }
            self_dedup { #mode }
        })
        .unwrap();
        assert_eq!(
            syn::parse2::<round::Execution>(execution.to_token_stream()).unwrap(),
            execution
        );
    }
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

#[test]
fn canonical_consumers_have_exact_owners_and_certified_inputs() {
    let target = compile(&source()).unwrap();
    for (index, relation) in target.context.relations.iter().enumerate() {
        let expected: Vec<_> = target
            .context
            .strata
            .iter()
            .flat_map(|s| s.seeds.iter().chain(&s.steps))
            .filter(|r| r.head == index)
            .map(|r| r.hole)
            .collect();
        assert_eq!(relation.admit, expected);
    }
    for hole in &target.holes {
        assert!(hole.facts.consumer_canonical);
        let inputs: std::collections::BTreeSet<_> = hole
            .cq
            .query
            .body
            .iter()
            .filter_map(|item| match item {
                round::cq::BodyItem::Positive { atom } => {
                    Some(cq_syntax::symbol_name(&atom.relation))
                }
                _ => None,
            })
            .collect();
        assert_eq!(hole.facts.discharged_host_sets, inputs);
        let packet: cq_stage::Context =
            syn::parse2(hole.pipeline.passes[hole.lower].arguments.clone()).unwrap();
        assert_eq!(packet, hole.facts);
    }
    let changes: &[fn(&mut round::Program)] = &[
        |p| p.context.relations[1].admit.clear(),
        |p| {
            let hole = p.context.relations[1].admit[0];
            p.context.relations[1].admit.push(hole);
        },
        |p| {
            let hole = p.context.relations[1].admit[0];
            p.context.relations[0].admit.push(hole);
        },
        |p| p.context.relations[1].admit[0] = usize::MAX,
        |p| p.holes[0].facts.consumer_canonical = false,
        |p| {
            p.holes[0]
                .facts
                .discharged_host_sets
                .insert("Unknown".into());
        },
        |p| p.holes[0].facts.discharged_host_sets.clear(),
        |p| p.holes[0].cq.declarations[0].value = Some(syn::parse_quote!(= (unproved_rows))),
    ];
    for (index, change) in changes.iter().enumerate() {
        let mut changed = target.clone();
        change(&mut changed);
        for hole in &mut changed.holes {
            hole.pipeline.passes[hole.lower].arguments = hole.facts.to_token_stream();
        }
        assert!(
            round::contract::check(&changed).is_err(),
            "owner/fact mutation {index}"
        );
    }
    let mut changed = target.clone();
    let hole = &mut changed.holes[0];
    hole.pipeline.passes[hole.lower].arguments = quote!(consumer = canonical);
    assert!(round::contract::check(&changed).is_err());
}

#[test]
fn uncertified_hosts_do_not_receive_discharge_facts() {
    let mut storage = source();
    storage.certificates.clear();
    let target = compile(&storage).unwrap();
    assert!(target.holes.iter().all(|hole| hole.facts.consumer_canonical
        && hole.facts.discharged_host_sets.is_empty()));
    round::contract::check(&target).unwrap();
}

#[test]
fn custom_lower_aliases_require_an_explicit_role() {
    let storage = source();
    let queue = quote!(cq = pipeline { custom::physical, ::mini_linq::passes::translate_pull };);
    assert!(
        compile_with(&storage, queue.clone())
            .unwrap_err()
            .to_string()
            .contains("lower")
    );
    let target = compile_with(&storage, quote!(#queue lower = 0;)).unwrap();
    assert!(target.holes.iter().all(|hole| hole.lower == 0));
    for arguments in [
        quote!(#queue lower = 7;),
        quote!(#queue lower = 0; lower = 0;),
        quote!(cq = pipeline { lower, lower, ::mini_linq::passes::translate_pull };),
        quote!(cq = pipeline { lower(host_set Unknown discharged), ::mini_linq::passes::translate_pull };),
    ] {
        assert!(compile_with(&storage, arguments).is_err());
    }
}
