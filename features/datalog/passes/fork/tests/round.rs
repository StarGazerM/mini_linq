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
        |p| p.policy.phases = syn::parse_quote!(publish_evaluate),
        |p| p.policy.admission = syn::parse_quote!(bag),
        |p| p.policy.termination = syn::parse_quote!(first_delta),
        |p| p.policy.observer = syn::parse_quote!(after_step),
        |p| p.policy.outputs = syn::parse_quote!(borrowed),
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
