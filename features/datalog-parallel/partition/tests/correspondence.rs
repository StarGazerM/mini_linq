//! Exact erasure, authored expected CQ, and independent mutation rejection.
use cq_datalog_partition::{
    ShardedProgram, check_sharded, compile_sharded, compile_sharded_with, compile_storage,
};
use cq_lang_datalog_round::{self as round, InputSource, Program, cq::BodyItem};
use proc_macro2::TokenStream;
use quote::{ToTokens, quote};

fn lower(tokens: TokenStream) -> Program {
    let source = syn::parse2(tokens).unwrap();
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let storage = cq_datalog_storage::compile(&incremental).unwrap();
    let storage = compile_storage(&storage).unwrap();
    cq_datalog_fork::compile(&storage).unwrap()
}
fn authored() -> Program {
    lower(quote! {
        struct Closure;
        relation Edge(x:i32,y:i32) = (edges);
        relation Blocked(x:i32,y:i32) = (blocked);
        relation Reach(x:i32,y:i32);
        output Reach;
        Reach(*x,*y) :- Edge(x,y), !Blocked(y,_);
        Reach(*x,*z) :- Reach(x,y), Edge(y,z), !Blocked(z,_);
    })
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
fn rejected(change: impl FnOnce(&mut Program)) {
    let source = authored();
    let mut target = compile_sharded(&source).unwrap();
    change(&mut target.round);
    assert!(check_sharded(&source, &target).is_err());
}

#[test]
fn partition_preserves_and_erases_declared_dataset_view_bindings() {
    // Local fixture mirrors runtime/cq-evidence/tests/fixtures for external consumers.
    let json = include_str!("fixtures/planner-recursive-evidence.json");
    let source = lower(quote! {
        struct Closure;
        relation Edge(x:i32,y:i32) = (edges);
        relation Reach(x:i32,y:i32);
        declared_evidence #json { Reach = "R" full [x="a",y="b"]; }
        output Reach;
        Seed: Reach(*x,*y) :- Edge(x,y);
        Step: Reach(*x,*z) :- Reach(x,y), Edge(y,z);
    });
    let target = compile_sharded(&source).unwrap();
    check_sharded(&source, &target).unwrap();
    assert_eq!(round::erase(&target.round).unwrap(), expected_push(source.clone()));
    let entry = target.round.execution.iter().find(|entry| {
        target.round.holes[entry.hole].cq.declared_evidence.as_ref().unwrap()
            .bindings.iter().any(|binding| binding.relation == entry.binding)
    }).unwrap();
    let mut stale = target.clone();
    let facts = stale.round.holes[entry.hole].cq.declared_evidence.as_mut().unwrap();
    facts.bindings.iter_mut().find(|binding| binding.relation == entry.binding)
        .unwrap().relation = syn::parse_quote!(missing_scan);
    assert!(check_sharded(&source, &stale).is_err());
}

#[test]
fn partition_metadata_selects_flat_slices_and_preserves_schemas() {
    let source = authored();
    let target = compile_sharded(&source).unwrap();
    check_sharded(&source, &target).unwrap();
    assert_eq!(
        round::erase(&target.round).unwrap(),
        expected_push(source.clone())
    );
    assert_eq!(
        syn::parse2::<ShardedProgram>(target.to_token_stream()).unwrap(),
        target
    );
    let stratum = &target.round.context.strata[0];
    for (rule, expected_source) in [
        (&stratum.seeds[0], InputSource::Layout(0)),
        (&stratum.steps[0], InputSource::Delta(2)),
    ] {
        let scan = rule.scan.as_ref().unwrap();
        assert_eq!(scan.source, expected_source);
        let execution = target
            .round
            .execution
            .iter()
            .find(|entry| entry.hole == rule.hole)
            .unwrap();
        let hole = &target.round.holes[rule.hole].cq;
        let declaration = &hole.declarations[hole.declarations.len() - 2];
        assert_eq!(declaration.name, execution.binding);
        assert_eq!(
            declaration.columns,
            source.context.relations[0].declaration.columns
        );
        let chunk = &execution.binding;
        assert_eq!(declaration.value, Some(syn::parse_quote!(= (#chunk))));
        let BodyItem::Positive { atom } = &hole.query.body[0] else {
            panic!("positive scan")
        };
        let BodyItem::Positive { atom: original } = &source.holes[rule.hole].cq.query.body[0]
        else {
            unreachable!()
        };
        assert_eq!(atom.relation, execution.binding);
        assert_eq!(atom.args, original.args);
    }
    // Key projections are neither scheduled for workers nor rewritten.
    for key in &stratum.keys {
        assert_eq!(target.round.holes[key.hole], source.holes[key.hole]);
        assert!(
            !target
                .round
                .execution
                .iter()
                .any(|entry| entry.hole == key.hole)
        );
    }
}

#[test]
fn partition_preserves_consumer_context_and_admission_owners() {
    let source = authored();
    let target = compile_sharded(&source).unwrap();
    for (before, after) in source
        .context
        .relations
        .iter()
        .zip(&target.round.context.relations)
    {
        assert_eq!(before.admit, after.admit);
    }
    for rule in source
        .context
        .strata
        .iter()
        .flat_map(|stratum| stratum.seeds.iter().chain(&stratum.steps))
    {
        let before = &source.holes[rule.hole];
        let after = &target.round.holes[rule.hole];
        assert_eq!(before.lower, after.lower);
        let expected = &before.facts;
        assert_eq!(&after.facts, expected);
        assert_eq!(
            after.pipeline.passes[after.lower].arguments.to_string(),
            expected.to_token_stream().to_string(),
        );
        assert!(after.facts.consumer_canonical);
    }
    assert_eq!(round::erase(&target.round).unwrap(), expected_push(source));
}

#[test]
fn partition_rejects_lost_context_and_admission_owners() {
    rejected(|target| {
        let hole = target.execution[0].hole;
        target.holes[hole].facts.consumer_canonical = false;
    });
    rejected(|target| {
        let hole = &mut target.holes[target.execution[0].hole];
        hole.pipeline.passes[hole.lower].arguments = quote!();
    });
    rejected(|target| {
        let head = target.context.strata[0].seeds[0].head;
        target.context.relations[head].admit.clear();
    });
    rejected(|target| {
        let rule = &target.context.strata[0].seeds[0];
        target.context.relations[rule.head].admit.push(rule.hole);
    });
    rejected(|target| {
        let key = target.context.strata[0].keys[0].hole;
        let head = target.context.strata[0].seeds[0].head;
        target.context.relations[head].admit.push(key);
    });
}

#[test]
fn checker_rejects_metadata_binding_and_occurrence_mutations() {
    rejected(|target| {
        target.execution.pop();
    });
    rejected(|target| target.execution.push(target.execution[0].clone()));
    rejected(|target| target.execution[0].hole = usize::MAX);
    rejected(|target| target.execution[0].occurrence = usize::MAX);
    rejected(|target| target.execution[0].strategy = syn::parse_quote!(unknown));
    rejected(|target| target.execution[0].binding = syn::parse_quote!(Edge));
    rejected(|target| target.context.strata[0].steps[0].scan = None);
    rejected(|target| {
        target.context.strata[0].steps[0]
            .scan
            .as_mut()
            .unwrap()
            .source = InputSource::Delta(usize::MAX)
    });
    rejected(|target| {
        target.context.strata[0].seeds[0]
            .scan
            .as_mut()
            .unwrap()
            .source = InputSource::Layout(1)
    });
    rejected(|target| {
        let entry = &target.execution[0];
        let chunk = &entry.binding;
        target.holes[entry.hole]
            .cq
            .declarations
            .iter_mut()
            .find(|declaration| declaration.name == *chunk)
            .unwrap()
            .value = Some(syn::parse_quote!(= (&#chunk)));
    });
    rejected(|target| {
        let entry = &target.execution[0];
        if let BodyItem::Positive { atom } = &mut target.holes[entry.hole].cq.query.body[0] {
            atom.relation = syn::parse_quote!(Edge);
        }
    });
    rejected(|target| {
        target.holes[0].pipeline.passes[0].arguments = quote!(no_reduce);
    });
    rejected(|target| target.policy.timing = syn::parse_quote!(phases));
}

#[test]
fn partition_rewrites_only_first_occurrence_of_a_self_join() {
    let source = lower(quote! {
        struct SelfJoin;
        relation E(x:i32,y:i32) = (edges);
        relation R(x:i32,y:i32);
        output R;
        R(*x,*z) :- E(x,y), E(a,z);
    });
    let target = compile_sharded(&source).unwrap();
    let rule = &target.round.context.strata[0].seeds[0];
    let chunk = &target.round.execution[0].binding;
    let layout = &target.round.context.layouts[0].name;
    let first: BodyItem = syn::parse_quote!(#chunk(x,y));
    let second: BodyItem = syn::parse_quote!(#layout(a,z));
    assert_eq!(
        target.round.holes[rule.hole]
            .cq
            .query
            .body
            .iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![first, second]
    );
    let mut changed = target.clone();
    changed.round.holes[rule.hole].cq.query.body[1] = syn::parse_quote!(#chunk(a,z));
    assert!(check_sharded(&source, &changed).is_err());
}

#[test]
fn a_leading_guard_is_not_moved_to_enable_partitioning() {
    let source = lower(quote! {
        struct GuardFirst;
        relation E(x:i32) = (edges);
        relation R(x:i32);
        output R;
        R(*x) :- if enabled, E(x);
    });
    let target = compile_sharded(&source).unwrap();
    assert!(target.round.execution.is_empty());
    assert_eq!(target.round, expected_push(source));
}

#[test]
fn keyed_initial_scans_and_key_holes_are_not_partitioned() {
    let source = lower(quote! {
        struct Selection;
        relation E(x:i32,y:i32) = (edges);
        relation Blocked(x:i32) = (blocked);
        relation R(y:i32);
        output R;
        R(*y) :- E(7,y), !Blocked(y);
    });
    let target = compile_sharded(&source).unwrap();
    assert!(target.round.execution.is_empty());
    assert_eq!(target.round, expected_push(source));
}

#[test]
fn bindings_are_fresh_per_rule_and_avoid_source_identifiers() {
    let mut source = authored();
    // Reconstruct the source so all embedded observer copies remain coherent.
    let mut input = source.storage.incremental.strata.source.clone();
    input.observer = Some(syn::parse_quote!(__cq_datalog_parallel_chunk_0));
    source = lower(input.to_token_stream());
    let target = compile_sharded(&source).unwrap();
    assert_eq!(target.round.execution.len(), 2);
    assert_ne!(
        target.round.execution[0].binding,
        target.round.execution[1].binding
    );
    for entry in &target.round.execution {
        assert_ne!(entry.binding, "__cq_datalog_parallel_chunk_0");
    }
    assert!(compile_sharded(&target.round).is_err());
    assert!(compile_sharded_with(&source, quote!(unexpected)).is_err());
}

#[test]
fn empty_output_only_program_is_an_identity_rewrite() {
    let source = lower(quote! {
        struct Input;
        relation E(x:i32) = (edges);
        output E;
    });
    assert_eq!(compile_sharded(&source).unwrap().round, source);
}

/// Declare an authored token in the embedded Storage table.
fn declare(program: &mut Program, name: &str, protocol: Option<round::storage::LayoutProtocol>) {
    round::storage::provider::declare(
        &mut program.storage.tokens,
        round::storage::provider::declared(
            name,
            round::storage::provider::Spelling::Verbatim,
            protocol,
            round::storage::provider::Admission::None,
            false,
            round::storage::provider::TaskHasher::Fx,
            &[],
            &[],
        ),
    );
}

/// Replace a layout's provider records in Storage and its Round copy alike,
/// with the hole inputs bound to it declaring the new token's host reads.
fn set_layout(
    program: &mut Program,
    delta: bool,
    index: usize,
    provider: round::storage::Provider,
    hasher: round::storage::Hasher,
    protocol: round::storage::LayoutProtocol,
) {
    let storage = if delta {
        &mut program.storage.delta_layouts[index]
    } else {
        &mut program.storage.layouts[index]
    };
    storage.provider = provider.clone();
    storage.hasher = hasher;
    storage.protocol = protocol;
    let context = if delta {
        &mut program.context.delta_layouts[index]
    } else {
        &mut program.context.layouts[index]
    };
    context.provider = provider;
    context.hasher = hasher;
    context.protocol = protocol;
    let (name, key) = (context.name.clone(), context.key);
    let token = round::storage::provider::lookup(&program.storage.tokens, &context.provider.token)
        .unwrap()
        .clone();
    for hole in &mut program.holes {
        for declaration in hole.cq.declarations.iter_mut().filter(|d| d.name == name) {
            let key: Vec<_> = declaration
                .columns
                .iter()
                .take(key)
                .map(|c| c.name.clone())
                .collect();
            declaration.hosts = round::storage::provider::host_decl(&token, &key, false)
                .into_iter()
                .collect();
        }
    }
}

#[test]
fn partition_requires_sharded_canonical_vector_deltas_and_admissible_layouts() {
    let mutations: &[fn(&mut Program)] = &[
        // A native (set) canonical layout is valid storage, but not sharded admission.
        |program| {
            let canonical = program.storage.providers[2].canonical;
            let provider: round::storage::Provider = syn::parse_quote!(
                @hash_set as ::std::collections::HashSet<(i32, i32,), ::std::hash::BuildHasherDefault<::cq_datalog::rustc_hash::FxHasher>>
            );
            set_layout(
                program,
                false,
                canonical,
                provider,
                round::storage::Hasher::Fx,
                round::storage::LayoutProtocol::Set,
            );
            program.context.outputs[0].conversion = syn::parse_quote!(owned_into_iter);
        },
        // An authored bulk token is not one of partition's admission tokens.
        |program| {
            declare(
                program,
                "sharded_index",
                Some(round::storage::LayoutProtocol::Bulk),
            );
            let provider: round::storage::Provider = syn::parse_quote!(
                @sharded_index as ::cq_datalog_parallel_runtime::ShardedIndex<(), (i32, i32,)>
            );
            set_layout(
                program,
                false,
                0,
                provider,
                round::storage::Hasher::None,
                round::storage::LayoutProtocol::Bulk,
            );
        },
        |program| {
            declare(
                program,
                "sharded_index",
                Some(round::storage::LayoutProtocol::Bulk),
            );
            let provider: round::storage::Provider = syn::parse_quote!(
                @sharded_index as ::cq_datalog_parallel_runtime::ShardedIndex<(), (i32, i32,)>
            );
            set_layout(
                program,
                true,
                0,
                provider,
                round::storage::Hasher::None,
                round::storage::LayoutProtocol::Bulk,
            );
        },
        |program| {
            declare(program, "deque_rows", None);
            let provider = &mut program.storage.providers[2];
            provider.delta =
                syn::parse_quote!(@deque_rows as ::std::collections::VecDeque<(i32, i32,)>);
            provider.next = provider.delta.clone();
            provider.delta_init = syn::parse_quote!(::std::collections::VecDeque::new());
            provider.next_init = provider.delta_init.clone();
            program.context.relations[2].providers = provider.clone();
        },
    ];
    for mutate in mutations {
        let mut source = lower(quote! {
            struct Closure;
            relation Edge(x:i32,y:i32) = (edges);
            relation Blocked(x:i32,y:i32) = (blocked);
            relation Reach(x:i32,y:i32);
            output Reach;
            Reach(*x,*y) :- Edge(x,y), !Blocked(y,_);
            Reach(*x,*z) :- Reach(x,y), Reach(y,z), !Blocked(z,_);
        });
        let mut target = compile_sharded(&source).unwrap();
        mutate(&mut source);
        mutate(&mut target.round);
        round::contract::check(&source).unwrap();
        round::contract::check(&target.round).unwrap();
        assert!(check_sharded(&source, &target).is_err());
        assert!(compile_sharded(&source).is_err());
    }
}

#[test]
fn authored_grouped_map_tokens_partition_by_their_declared_protocol() {
    // Admission reads the declared protocol, never the spelling: an authored
    // ordered grouped map is maintained through grouped-map operations.
    let mut source = authored();
    declare(
        &mut source,
        "ordered_map",
        Some(round::storage::LayoutProtocol::GroupedMap),
    );
    let provider: round::storage::Provider = syn::parse_quote!(
        @ordered_map as ::std::collections::BTreeMap<(), ::std::vec::Vec<(i32, i32,)>>
    );
    let protocol = source.storage.layouts[0].protocol;
    set_layout(
        &mut source,
        false,
        0,
        provider,
        round::storage::Hasher::None,
        protocol,
    );
    round::contract::check(&source).unwrap();
    let target = compile_sharded(&source).unwrap();
    check_sharded(&source, &target).unwrap();
}

#[test]
fn task_realization_preserves_custom_terminals_arguments_and_registry_premises() {
    for (path, arguments, interface) in [
        (
            syn::parse_quote!(custom::translate_pull),
            quote!(opaque(argument)),
            round::HoleInterface::Pull,
        ),
        (
            syn::parse_quote!(::mini_linq::passes::translate_pull),
            quote!(opaque(argument)),
            round::HoleInterface::Pull,
        ),
        (
            syn::parse_quote!(custom::push),
            quote!(opaque(argument)),
            round::HoleInterface::Push,
        ),
    ] {
        let mut source = authored();
        let hole_index = source.context.strata[0].seeds[0].hole;
        if !source.terminals.0.iter().any(|entry| entry.path == path) {
            source.terminals.0.push(round::Terminal {
                path: path.clone(),
                interface,
            });
        }
        let hole = &mut source.holes[hole_index];
        hole.interface = interface;
        let terminal = hole.pipeline.passes.last_mut().unwrap();
        terminal.path = path;
        terminal.arguments = arguments;
        let target = compile_sharded(&source).unwrap();
        check_sharded(&source, &target).unwrap();
        let erased = round::erase(&target.round).unwrap();
        assert_eq!(erased.holes[hole_index], source.holes[hole_index]);
        assert_eq!(erased.terminals, source.terminals);
    }

    let mut source = authored();
    source
        .terminals
        .0
        .retain(|entry| entry.interface == round::HoleInterface::Pull);
    let target = compile_sharded(&source).unwrap();
    assert_eq!(round::erase(&target.round).unwrap(), source);
}

#[test]
fn task_realization_replaces_only_the_standard_sequence_tail() {
    for relative in [false, true] {
        let mut source = authored();
        let index = source.context.strata[0].seeds[0].hole;
        let passes = &mut source.holes[index].pipeline.passes;
        let start = passes.len() - 3;
        if relative {
            for pass in &mut passes[start..] {
                pass.path.leading_colon = None;
            }
        }
        let target = compile_sharded(&source).unwrap();
        check_sharded(&source, &target).unwrap();
        let erased = round::erase(&target.round).unwrap();
        let before = &source.holes[index];
        let after = &erased.holes[index];
        assert_eq!(after.pipeline.passes.len(), start + 1);
        for (after, before) in after.pipeline.passes[..start]
            .iter()
            .zip(&before.pipeline.passes[..start])
        {
            assert_eq!(
                after.to_token_stream().to_string(),
                before.to_token_stream().to_string()
            );
        }
        assert_eq!(
            after.pipeline.passes[start].path,
            syn::parse_quote!(::cq_datalog::passes::print_push)
        );
        assert!(after.pipeline.passes[start].arguments.is_empty());
        assert_eq!(after.lower, before.lower);
        assert_eq!(after.facts, before.facts);

        // Reintroducing any part of the Sequence tail is not an allowed rewrite.
        for pass in &before.pipeline.passes[start..start + 2] {
            let mut invalid = target.clone();
            invalid.round.holes[index]
                .pipeline
                .passes
                .insert(start, pass.clone());
            round::contract::check(&invalid.round).unwrap();
            assert!(check_sharded(&source, &invalid).is_err());
        }
    }
}

#[test]
fn task_realization_preserves_authored_sequence_lowering_grades_and_rewrites() {
    let changes: &[fn(&mut Vec<cq_stage::Invocation>)] = &[
        |passes| {
            let start = passes.len() - 3;
            passes[start].path = syn::parse_quote!(custom::sequence);
        },
        |passes| {
            let grade = passes.len() - 2;
            passes[grade].path = syn::parse_quote!(custom::grade);
        },
        |passes| {
            let start = passes.len() - 3;
            passes[start].arguments = quote!(opaque(arguments));
        },
        |passes| {
            let grade = passes.len() - 2;
            passes[grade].arguments = quote!(opaque(arguments));
        },
        |passes| {
            let grade = passes.len() - 2;
            passes.insert(grade, syn::parse_quote!(custom::decompose));
        },
    ];
    for change in changes {
        let mut source = authored();
        let index = source.context.strata[0].seeds[0].hole;
        change(&mut source.holes[index].pipeline.passes);
        let target = compile_sharded(&source).unwrap();
        check_sharded(&source, &target).unwrap();
        let erased = round::erase(&target.round).unwrap();
        assert_eq!(erased.holes[index], source.holes[index]);
    }
}

#[test]
fn task_realization_preserves_existing_structures_push_holes() {
    let source = expected_push(authored());
    round::contract::check(&source).unwrap();
    let target = compile_sharded(&source).unwrap();
    check_sharded(&source, &target).unwrap();
    assert_eq!(round::erase(&target.round).unwrap(), source);
}
#[test]
fn task_realization_rejects_unrequested_terminal_and_key_rewrites() {
    let source = authored();
    let mut target = compile_sharded(&source).unwrap();
    let hole_index = source.context.strata[0].seeds[0].hole;
    target.round.holes[hole_index].interface = round::HoleInterface::Pull;
    target.round.holes[hole_index]
        .pipeline
        .passes
        .last_mut()
        .unwrap()
        .path = syn::parse_quote!(::mini_linq::passes::translate_pull);
    round::contract::check(&target.round).unwrap();
    assert!(check_sharded(&source, &target).is_err());

    let mut target = compile_sharded(&source).unwrap();
    let key = source.context.strata[0].keys[0].hole;
    target.round.holes[key].interface = round::HoleInterface::Push;
    target.round.holes[key]
        .pipeline
        .passes
        .last_mut()
        .unwrap()
        .path = syn::parse_quote!(::cq_datalog::passes::print_push);
    round::contract::check(&target.round).unwrap();
    assert!(check_sharded(&source, &target).is_err());
}
