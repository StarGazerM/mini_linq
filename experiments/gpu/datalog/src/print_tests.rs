use crate::{print, storage};
use quote::{ToTokens, quote};

fn program() -> cq_lang_datalog_round::Program {
    program_with_symbols(syn::parse_quote!(i32), quote!())
}

pub(crate) fn program_with_symbols(
    ty: syn::Type,
    symbols: proc_macro2::TokenStream,
) -> cq_lang_datalog_round::Program {
    program_with_orders(ty, symbols, false)
}

fn program_with_orders(
    ty: syn::Type,
    symbols: proc_macro2::TokenStream,
    extra_order: bool,
) -> cq_lang_datalog_round::Program {
    let mut host = cq_datalog_storage::compile(&syn::parse_quote! {
        struct Paths;
        relation Edge(x:#ty,y:#ty) = (edges);
        relation Reach(x:#ty,y:#ty);
        output Reach;
        Reach(*x,*y) :- Edge(x,y);
        Reach(*x,*z) :- Reach(x,y), Edge(y,z);
        strata { component [Reach]; }
        incremental { scheme seminaive_covering;
            driver DeltaFirst;
            delta Reach = ReachChanges;
            variant 0 seed { Reach(*x,*y) :- Edge(x,y); }
            variant 1 delta 0 { Reach(*x,*z) :- ReachChanges(x,y), Edge(y,z); }
        }
    })
    .unwrap();
    if extra_order {
        let mut extra = host.layouts[host.providers[1].canonical].clone();
        extra.columns.reverse();
        host.layouts.push(extra.clone());
        extra.maintenance = syn::parse_quote!(replace_each_run);
        host.delta_layouts.push(extra);
    }
    let device = storage::compile_with(&host, symbols).unwrap();
    let mut pipeline = cq_datalog_fork::default_pipeline();
    let terminal = pipeline.passes.last_mut().unwrap();
    terminal.path = syn::parse_quote!(::cq_gpu_sequence::translate_arena);
    // Printer contract tests splice tokens only; actual stage execution belongs
    // to the CUDA oracle using the fully configured Structures/Sequence queue.
    pipeline.passes.insert(
        pipeline.passes.len() - 1,
        syn::parse_quote!(::cq_gpu_datalog::bind_inputs),
    );
    let binding = pipeline.passes.len() - 2;
    let round = cq_datalog_fork::compile_with(
        &device,
        quote!(
            cq = #pipeline;
            lower = 2;
            terminals = [::cq_gpu_sequence::translate_arena => batch];
        ),
    )
    .unwrap();
    crate::bindings::compile_with(&round, quote!(binding = #binding)).unwrap()
}
#[test]
fn union_project_certificate_is_bound_to_the_rule_and_its_head_positions() {
    let mut p = program();
    let inputs = crate::bindings::input_providers(&p).unwrap();
    let s = &p.context.strata[0];
    let r = &s.steps[0];
    let certified = crate::union_project::plans(&p, &inputs);
    assert_eq!(certified.len(), 1, "only the recursive composition is a union");
    let mut plan = certified[&r.hole].clone();
    assert!(plan.witness_second && plan.group_first);
    let terminal = p.holes[r.hole].pipeline.passes.last_mut().unwrap();
    let (record, ordinary) = crate::union_project::split(&terminal.arguments).unwrap();
    assert_eq!(record.as_ref(), Some(&plan));
    plan.group_first = false;
    terminal.arguments = quote!(#plan #ordinary);
    assert!(crate::bindings::check(&p).is_err(), "transposed head must be rejected");
}

#[test]
fn union_project_rejects_residual_predicates_and_nonendpoint_heads() {
    let mut p = program();
    let inputs = crate::bindings::input_providers(&p).unwrap();
    let r = p.context.strata[0].steps[0].clone();
    let original = p.holes[r.hole].cq.query.clone();
    for bad in [syn::parse_quote!(*y), syn::parse_quote!(*x + 1), syn::parse_quote!(*z)] {
        p.holes[r.hole].cq.query.head.args[0] = bad;
        assert!(crate::union_project::UnionProject::recognize(&p, &r, &inputs).is_none());
    }
    p.holes[r.hole].cq.query = original;
    p.holes[r.hole].cq.query.body.push(syn::parse_quote!(where x != z));
    assert!(crate::union_project::UnionProject::recognize(&p, &r, &inputs).is_none());
}


#[test]
fn round_shared_marker_cannot_merge_seed_and_step_across_publication() {
    let mut p = program_with_symbols(syn::parse_quote!(u32), quote!());
    for h in &mut p.holes {
        let pass = h.pipeline.passes.last_mut().unwrap();
        let arena = &pass.arguments;
        pass.arguments = quote!(round_shared(0,2); #arena);
    }
    assert!(crate::bindings::check(&p).unwrap_err().to_string().contains("RoundShared"));
}

#[test]
fn packed_word_names_are_stable_and_distinguish_column_tuples() {
    let mut names = std::collections::BTreeSet::new();
    // DOOP mixes symbol IDs and integer columns at several arities.
    for text in [
        "()", "(u32,)", "(u32,u32)", "(u32,u32,u32)",
        "(u32,u32,u32,u32)", "(u32,u32,u32,u32,u32)",
        "(i32,)", "(i32,u32)", "(u32,i32)", "(u32,i32,u32)",
        "(u32,u32,i32)",
    ] {
        let tuple: syn::TypeTuple = syn::parse_str(text).unwrap();
        let first = storage::word_type(&tuple.elems).unwrap();
        let reparsed: syn::TypeTuple = syn::parse2(tuple.to_token_stream()).unwrap();
        assert_eq!(first, storage::word_type(&reparsed.elems).unwrap());
        assert!(names.insert(first.to_token_stream().to_string()));
        let types = tuple.elems.iter().collect::<Vec<_>>();
        assert!(names.insert(storage::layout_type(&types).to_token_stream().to_string()));
    }
    let compact: syn::Type = syn::parse_str("types::Symbol<u32>").unwrap();
    let spaced: syn::Type = syn::parse_str("types :: Symbol < u32 >").unwrap();
    assert_eq!(storage::word_type([&compact]).unwrap(), storage::word_type([&spaced]).unwrap());
}

#[test]
fn printer_defines_each_packed_type_once_before_holes() {
    let program = program();
    let printed = print::compile(&program).unwrap();
    let syn::Expr::Block(block) = &printed else { panic!("expected generated block") };
    let mut names = std::collections::BTreeSet::new();
    let mut layout_definitions = 0;
    let mut saw_hole = false;
    for statement in &block.block.stmts {
        match statement {
            syn::Stmt::Item(syn::Item::Type(item)) => {
                assert!(!saw_hole, "aliases must be visible to every hole");
                assert!(names.insert(item.ident.to_string()), "duplicate alias");
                if item.ident.to_string().starts_with("__cq_gpu_layout_type_") {
                    assert!(item.ty.to_token_stream().to_string().contains("RadixLayout"));
                    layout_definitions += 1;
                }
            }
            syn::Stmt::Item(syn::Item::Fn(_)) => saw_hole = true,
            _ => {}
        }
    }
    let device = program.storage.device.as_ref().unwrap();
    for word in device.relations.iter().map(|record| &record.word).chain(
        device.layouts.iter().chain(&device.delta_layouts).chain(&device.negations)
            .map(|record| &record.word),
    ) {
        assert!(names.contains(&word.to_token_stream().to_string()));
    }
    assert!(saw_hole);
    let text = printed.to_token_stream().to_string();
    let words = names.iter().filter(|name| name.starts_with("__cq_gpu_word_")).count();
    assert_eq!(text.matches("WordWidth").count(), words);
    assert_eq!(layout_definitions, names.len() - words);
}

#[test]
fn printer_rejects_undefined_word_aliases_in_records_and_holes() {
    let mut changed = program();
    changed.storage.device.as_mut().unwrap().relations[0].word =
        syn::parse_quote!(__cq_gpu_word_undefined);
    assert!(print::compile(&changed).unwrap_err().to_string().contains("undefined GPU packed-type alias"));

    let mut changed = program();
    changed.holes[0].pipeline.passes[0].arguments = quote!((__cq_gpu_word_undefined));
    assert!(print::compile(&changed).unwrap_err().to_string().contains("undefined GPU packed-type alias"));

    let mut changed = program();
    changed.holes[0].pipeline.passes[0].arguments = quote!((__cq_gpu_layout_type_undefined));
    assert!(print::compile(&changed).unwrap_err().to_string().contains("undefined GPU packed-type alias"));
}

#[test]
fn physical_full_states_keep_logical_hosts_without_duplicate_backing_locals() {
    let program = program();
    let states = storage::FullStates::build(&program.storage).unwrap();
    let printed = print::compile(&program).unwrap().to_token_stream().to_string();
    let mut aliases = 0;
    for (index, layout) in program.context.layouts.iter().enumerate() {
        let owner = states.owner(index);
        let state = format!("__cq_gpu_layout_{index}");
        if owner == index {
            assert!(printed.contains(&format!("mut {state} ,")));
        } else {
            aliases += 1;
            assert!(!printed.split_whitespace().any(|token| token == state));
            let source = &program.context.layouts[owner].host;
            let host = &layout.host;
            assert!(printed.contains(&quote!(#host = #source.clone()).to_string()));
        }
    }
    assert!(aliases > 0, "fixture must exercise physical full-state sharing");
}

#[test]
fn device_round_preserves_delta_driver_and_source_permutation_facts() {
    let round = program();
    assert_eq!(
        round.storage.incremental.driver,
        cq_lang_datalog_round::incremental::DriverPolicy::DeltaFirst,
    );
    for stratum in &round.context.strata {
        for rule in stratum.seeds.iter().chain(&stratum.steps) {
            let variant = &round.storage.incremental.variants[rule.variant];
            let hole = &round.holes[rule.hole];
            let facts = hole.planning.as_ref().unwrap();
            assert_eq!(facts.driver, round.storage.incremental.driver);
            assert_eq!(facts.delta, variant.delta);
            assert_eq!(facts.permutation, variant.order);
            assert_eq!(facts.shared_demand, None);
        }
    }
    let parsed: cq_lang_datalog_round::Program = syn::parse2(round.to_token_stream()).unwrap();
    assert_eq!(parsed, round);
}

#[test]
fn device_input_packets_record_shared_construction_and_reject_unrequested_statistics() {
    struct Packet {
        providers: cq_lang_structures::Providers,
        inputs: Vec<(syn::Ident, cq_lang_structures::Provider)>,
    }
    impl syn::parse::Parse for Packet {
        fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
            let providers = input.parse()?;
            let mut inputs = Vec::new();
            while !input.is_empty() {
                let name = input.parse()?;
                input.parse::<syn::Token![=>]>()?;
                let provider = input.parse()?;
                input.parse::<syn::Token![;]>()?;
                inputs.push((name, provider));
            }
            Ok(Self { providers, inputs })
        }
    }
    let program = program();
    let mut checked = 0;
    for (index, hole) in program.holes.iter().enumerate() {
        let owner = hole.binding.unwrap();
        let mut packet: Packet =
            syn::parse2(hole.pipeline.passes[owner].arguments.clone()).unwrap();
        for (_, provider) in &packet.inputs {
            let declared = packet.providers.get(&provider.token).unwrap().instance();
            assert_eq!(provider.construction, declared.construction);
            assert_eq!(provider.construction, cq_lang_structures::stats::Construction::Ordinary);
            assert_eq!(
                syn::parse2::<cq_lang_structures::Provider>(quote!(#provider)).unwrap(),
                *provider,
            );
            checked += 1;
        }
        if let Some((_, provider)) = packet.inputs.first_mut() {
            provider.construction = cq_lang_structures::stats::Construction::Statistics(
                cq_lang_structures::stats::BuildDemand { rows: true, max_group: false },
            );
            let declarations = &packet.providers;
            let entries = packet.inputs.iter().map(|(name, provider)| quote!(#name => #provider;));
            let mut forged = program.clone();
            forged.holes[index].pipeline.passes[owner].arguments = quote!(#declarations #(#entries)*);
            assert!(crate::bindings::check(&forged).is_err());
        }
    }
    assert!(checked > 0);
}

#[test]
fn negative_device_bindings_preserve_fork_declarations() {
    let host = cq_datalog_storage::compile(&syn::parse_quote! {
        struct Negative;
        relation Edge(x:i16,y:i16) = (edges);
        relation Blocked(x:i16) = (blocked);
        relation Result(x:i16,y:i16);
        output Result;
        Result(*x,*y) :- Edge(x,y), !Blocked(x);
        strata { component [Result]; }
        incremental { scheme seminaive_covering;
            delta Result = ResultChanges;
            variant 0 seed { Result(*x,*y) :- Edge(x,y), !Blocked(x); }
        }
    })
    .unwrap();
    let mut device = storage::compile(&host).unwrap();
    let records = device.device.as_mut().unwrap();
    for record in records.layouts.iter_mut()
        .chain(&mut records.delta_layouts).chain(&mut records.negations)
    {
        let word = &record.word;
        record.active_bits = syn::parse_quote!(<#word as ::gpu_datalog_runtime::Word>::BITS);
        let state = &record.state;
        let active_bits = &record.active_bits;
        let (ratio, radix_bits, merge_tile_rows, block_threads) = (
            record.ratio, record.radix_bits, record.merge_tile_rows, record.block_threads,
        );
        record.state_init = syn::parse_quote!(<#state>::new_set(
            __cq_gpu_pool.clone(),
            ::gpu_datalog_runtime::RadixRunParams {
                ratio: #ratio, radix_bits: #radix_bits,
                merge_tile_rows: #merge_tile_rows, block_threads: #block_threads
            }, #active_bits
        )?);
    }
    storage::check(&host, &device).unwrap();
    let mut pipeline = cq_datalog_fork::default_pipeline();
    pipeline.passes.last_mut().unwrap().path =
        syn::parse_quote!(::cq_gpu_sequence::translate_arena);
    pipeline.passes.insert(
        pipeline.passes.len() - 1,
        syn::parse_quote!(::cq_gpu_datalog::bind_inputs),
    );
    let binding = pipeline.passes.len() - 2;
    let round = cq_datalog_fork::compile_with(
        &device,
        quote!(
            cq = #pipeline;
            lower = 2;
            terminals = [::cq_gpu_sequence::translate_arena => batch];
        ),
    )
    .unwrap();
    let bound = crate::bindings::compile_with(&round, quote!(binding = #binding, head_cursor)).unwrap();
    cq_lang_datalog_round::check_storage(&device, &bound).unwrap();
    for (before, after) in round.holes.iter().zip(&bound.holes) {
        assert_eq!(before.cq.declarations, after.cq.declarations);
    }
    let key = &bound.context.strata[0].keys[0];
    assert!(!crate::bindings::head_argument(&bound.holes[key.hole]));
    let mut bad_key = bound.clone();
    bad_key.holes[key.hole].pipeline.passes.last_mut().unwrap().arguments =
        quote!(head; &mut *__cq_gpu_arena);
    assert!(crate::bindings::check(&bad_key).is_err());
    assert_eq!(key.provider.token, storage::RADIX_RELATION);
    let consumer = &bound.holes[bound.context.strata[0].seeds[0].hole];
    let declaration = consumer
        .cq
        .declarations
        .iter()
        .find(|declaration| declaration.name == key.name)
        .unwrap();
    assert!(
        declaration.hosts.is_empty(),
        "device inputs do not declare CPU host reads"
    );
    print::compile(&bound).unwrap();
    let records = bound.storage.device.as_ref().unwrap();
    for record in records.layouts.iter()
        .chain(&records.delta_layouts).chain(&records.negations)
    {
        let word = &record.word;
        let expected: syn::Expr =
            syn::parse_quote!(<#word as ::gpu_datalog_runtime::Word>::BITS);
        assert_eq!(record.active_bits, expected);
    }

    // The correspondence checker must still reject a fabricated host capability.
    let mut forged = bound.clone();
    let consumer = &mut forged.holes[bound.context.strata[0].seeds[0].hole];
    let declaration = consumer
        .cq
        .declarations
        .iter_mut()
        .find(|declaration| declaration.name == key.name)
        .unwrap();
    let column = &declaration.columns[0].name;
    declaration
        .hosts
        .push(syn::parse_quote!(host [#column] reads [membership]));
    assert!(cq_lang_datalog_round::check_storage(&device, &forged).is_err());
}

#[test]
fn tc_emits_resident_waves_admission_and_publication() {
    let program = program();
    let source = print::compile(&program)
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(source.contains("append_sorted"));
    assert!(source.contains("RadixLayout"));
    assert!(source.contains("DeviceRadixRun"));
    assert!(source.contains("RadixRunParams"));
    assert!(source.contains("radix_bits"));
    assert!(source.contains("with_sorted"));
    assert!(source.contains(". republish (&"));
    assert!(source.contains("while"));
    assert!(source.contains("translate_arena"));
    assert_eq!(source.matches("Rows :: rows").count(), 1);
    assert_eq!(source.matches(". hole (").count(), program.holes.len());
    for hole in 0..program.holes.len() {
        assert_eq!(
            source.matches(&quote!(.hole(#hole)).to_string()).count(),
            1,
            "each hole must retain its own previous-capacity reservation"
        );
    }
    assert!(!source.contains("synchronize"));
    assert_eq!(
        source.matches("release_at_stratum_boundary").count(),
        program.context.strata.len()
    );
    assert_eq!(
        source.matches("release_cached_scratch").count(),
        program.context.strata.len()
    );
    // Unused pool storage is trimmed between strata, not after the last one.
    assert_eq!(
        source.matches("release_unused").count(),
        program.context.strata.len() - 1
    );
    assert!(source.contains("std :: mem :: drop"));
}

#[test]
fn rejects_host_observers_and_timing() {
    let mut observed = program();
    observed.storage.incremental.strata.source.observer = Some(syn::parse_quote!(observer));
    observed.context.observer = Some(syn::parse_quote!(observer));
    assert!(
        print::compile(&observed)
            .unwrap_err()
            .to_string()
            .contains("observer")
    );
    let mut timed = program();
    timed.policy.timing = syn::parse_quote!(phases);
    assert!(
        print::compile(&timed)
            .unwrap_err()
            .to_string()
            .contains("timing")
    );
}

#[test]
fn rejects_hole_chunking_policy() {
    let mut program = program();
    program.schedule = program.schedule.clone().with_policy(
        0,
        cq_lang_datalog_round::schedule::HolePolicy {
            phase: syn::parse_quote!(step),
            index: 0,
            grain: Some(64),
            inline_below: None,
        },
    );
    assert!(
        print::compile(&program)
            .unwrap_err()
            .to_string()
            .contains("grain")
    );
}

#[test]
fn key_sets_and_layouts_print_recorded_device_records() {
    let program = program();
    let device = program.storage.device.as_ref().unwrap();
    let source = print::compile(&program)
        .unwrap()
        .to_token_stream()
        .to_string();
    for record in device.layouts.iter().chain(&device.delta_layouts) {
        assert!(source.contains(&record.state_init.to_token_stream().to_string()));
        assert!(source.contains(&record.init.to_token_stream().to_string()));
    }
    for record in &device.relations {
        assert!(source.contains(&record.packing.to_token_stream().to_string()));
    }
    for relation in &program.context.relations {
        if let Some(delta) = &relation.delta_host {
            assert!(source.contains(&quote!(#delta = ).to_string()));
        }
    }
}

#[test]
fn admitted_orders_are_printed_once_per_publication_and_reused() {
    let program = program_with_orders(syn::parse_quote!(i32), quote!(), true);
    let device = program.storage.device.as_ref().unwrap();
    let source = print::compile(&program).unwrap().to_token_stream().to_string();
    assert!(source.contains("new_set"));
    for (relation, record) in device.relations.iter().enumerate() {
        let sites = source.matches(&format!("let __cq_gpu_admitted_{relation}_0 =")).count();
        for order in 1..record.admitted_orders.len() {
            assert_eq!(
                source.matches(&format!("let __cq_gpu_admitted_{relation}_{order} =")).count(),
                sites,
                "an admitted order is constructed once, not once per consumer",
            );
        }
    }
    assert!(source.contains("trace_work"));
}

#[test]
fn printer_rejects_tampered_device_facts() {
    let source = program();
    let changes: &[fn(&mut cq_lang_datalog_storage::DeviceStorage)] = &[
        |device| device.layouts[0].ratio += 1,
        |device| device.layouts[0].offset = syn::parse_quote!(u32),
        |device| device.layouts[0].construction = syn::parse_quote!(occurrences),
        |device| device.layouts[0].read = syn::parse_quote!(item_range),
        |device| device.layouts[0].radix_bits = 33,
        |device| device.layouts[0].radix_bits = 1,
        |device| device.layouts[0].merge_tile_rows = 0,
        |device| device.layouts[0].block_threads = 512,
        |device| device.layouts[0].permutation_threads = 128,
        |device| device.layouts[0].active_bits = syn::parse_quote!(1u32),
        |device| device.layouts[0].offset_layout = syn::parse_quote!(materialized),
        |device| device.layouts[0].item_sources = syn::parse_quote!(allocated),
        |device| device.layouts[0].admitted_order = None,
        |device| device.relations[0].admitted_orders[0].certificate = syn::parse_quote!(unchecked),
        |device| device.relations[0].output_permutation.clear(),
        |device| device.layouts[0].input.columns[0].storage = syn::parse_quote!(f32),
        |device| device.layouts[0].token = syn::parse_quote!(unknown),
    ];
    for change in changes {
        let mut target = source.clone();
        change(target.storage.device.as_mut().unwrap());
        assert!(print::compile(&target).is_err());
    }
}

#[test]
fn rejects_missing_input_binding_stage() {
    let mut program = program();
    for hole in &mut program.holes {
        hole.binding = None;
    }
    assert!(
        print::compile(&program)
            .unwrap_err()
            .to_string()
            .contains("binding invocation owner")
    );
}

#[test]
fn checked_invocations_survive_roundtrip_and_aliasing() {
    let mut program = program();
    for hole in &mut program.holes {
        let binding = hole.binding.unwrap();
        hole.pipeline.passes[binding].path = syn::parse_quote!(::consumer::resident_inputs);
    }
    let roundtrip = syn::parse2::<cq_lang_datalog_round::Program>(quote!(#program)).unwrap();
    assert_eq!(roundtrip, program);
    assert!(print::compile(&roundtrip).is_ok());
    let binding = program.holes[0].binding.unwrap();
    program.holes[0].pipeline.passes[binding].arguments = quote!();
    assert!(
        print::compile(&program)
            .unwrap_err()
            .to_string()
            .contains("declared inputs")
    );
}

#[test]
fn rejects_unrecorded_batch_arena_arguments() {
    let mut program = program();
    program.holes[0]
        .pipeline
        .passes
        .last_mut()
        .unwrap()
        .arguments = quote!();
    assert!(
        print::compile(&program)
            .unwrap_err()
            .to_string()
            .contains("arena argument")
    );
}

#[test]
fn device_bindings_preserve_scan_and_keyed_readers() {
    let (physical, source) = syn::parse::Parser::parse2(|input: syn::parse::ParseStream<'_>| {
        let physical = input.parse()?;
        let structures = cq_lang_structures::Program::parse(input, &physical)?;
        Ok((physical, structures))
    }, quote! {
        physical {
            relation Delta(x:i32,y:i32) = (&delta);
            relation Edge(y:i32,z:i32) = (&edge);
            consumer canonical;
            d = pipe rename Delta {x -> x,y -> y};
            e = pipe rename Edge {y -> y,z -> z};
            joined = pipe probe first [y] d,e;
            result = pipe project joined keep {x,z};
            return result as answer(x,z);
        }
        structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; hash_index = ::cq_storage::HashIndex<'_, _, _> { reads [exact, membership], supports [keys], build typed, join_filter, statistics_build [rows, max_group], statistics [ rows: rows occurrences exact hash_finalize stat_rows host optional build single, keys: keys distinct_keys exact hash_finalize stat_keys host optional build single, max_group: max_group max_bucket_occurrences exact hash_finalize stat_max_group host optional build single ] }; }
            d host scan = @host_scan as ::core::iter::Map<_,_> [];
            e host scan = @host_scan as ::core::iter::Map<_,_> [];
            e host result(joined) = @hash_index as ::cq_storage::HashIndex<'_,_,_> [y];
        }
    }).unwrap();
    let declaration = cq_gpu_sequence_core::providers::declaration("radix_relation");
    assert_eq!(
        declaration.facts.abi,
        Some(cq_lang_structures::provider::NativeAbi::RadixRelation)
    );
    let scalar = syn::parse_quote!(i32);
    let word = storage::word_type([&scalar, &scalar]).unwrap();
    let layout = storage::layout_type(&[&scalar, &scalar]);
    let target = crate::bind_inputs(
        &physical,
        &source,
        quote! {
            providers { #declaration }
            Delta => @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word #word; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as #layout ;
            Edge => @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word #word; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as #layout ;
        },
    )
    .unwrap();
    assert_eq!(target.structures.entries.len(), 3);
    for (before, after) in source
        .structures
        .entries
        .iter()
        .zip(&target.structures.entries)
    {
        let cq_lang_structures::Structure::Host { reader: before, .. } = &before.structure else {
            panic!()
        };
        let cq_lang_structures::Structure::Host { reader: after, .. } = &after.structure else {
            panic!()
        };
        assert_eq!(before, after);
    }
    let mut opaque = target.clone();
    for entry in &mut opaque.structures.entries {
        if let cq_lang_structures::Structure::Host { keyed, .. } = &mut entry.structure {
            let input = keyed.provider.input.as_ref().unwrap();
            assert_eq!(input.key_count, 2);
            assert_eq!(input.columns.len(), 2);
            keyed.provider.ty = syn::parse_quote!(OpaqueLayoutAlias);
        }
    }
    cq_lang_structures::contract::check(&physical, &opaque).unwrap();
    cq_gpu_sequence_core::runs::bind(&physical, &opaque).unwrap();
}

#[test]
fn symbols_cross_only_host_boundaries_and_keep_logical_types() {
    let program = program_with_symbols(
        syn::parse_quote!(String),
        quote!(symbols(String => TextCodec)),
    );
    let source = print::compile(&program)
        .unwrap()
        .to_token_stream()
        .to_string();
    assert_eq!(source.matches(":: code").count(), 2);
    assert_eq!(source.matches(":: decode").count(), 2);
    assert!(source.contains("Symbol < String , TextCodec >"));
    assert!(source.contains("as :: gpu_datalog_runtime :: Scalar > :: COLUMN"));
    assert!(!source.contains("as String"));
    for relation in &program.context.relations {
        assert!(
            relation
                .declaration
                .columns
                .iter()
                .all(|column| column.ty == syn::parse_quote!(String))
        );
    }
}

#[test]
fn gpu_admission_is_recorded_at_admit_and_rejects_in_plan_advice() {
    let fork = |known: proc_macro2::TokenStream| {
        let host = cq_datalog_storage::compile(&syn::parse_quote! {
            struct Flags;
            relation Edge(x:i32,y:i32) = (edges);
            relation Flag(x:i32);
            output Flag;
            // `x` binds before the last read: in_plan on the CPU route.
            Flag(*x) :- Edge(x,y), Edge(y,x);
            strata { component [Flag]; }
            incremental { scheme seminaive_covering;
                delta Flag = FlagChanges;
                variant 0 seed #known { Flag(*x) :- Edge(x,y), Edge(y,x); }
            }
        })
        .unwrap();
        let device = storage::compile_with(&host, quote!()).unwrap();
        cq_datalog_fork::compile(&device)
    };
    let round = fork(quote!()).unwrap();
    let rule = &round.context.strata[0].seeds[0];
    assert_eq!(rule.known, cq_lang_datalog_round::Admission::Admit);
    let error = fork(quote!(known(at(level 1)))).unwrap_err().to_string();
    assert!(error.contains("GPU Datalog admission is at admit only (ADM-InPlan GPU follow-up)"));
    // A forged in-plan record is rejected (by DL-AdmitOnce before printing).
    let mut forged = program();
    forged.context.strata[0].steps[0].known = cq_lang_datalog_round::Admission::InPlan;
    assert!(print::compile(&forged).is_err());
}

// Two heads publish together by default. The second seed for A also lets us
// split its writes with B's publication or make one A producer non-replayable.
fn multihead_round(second_a: syn::Expr) -> (cq_lang_datalog_round::Program, usize) {
    let host = cq_datalog_storage::compile(&syn::parse_quote! {
        struct Pair;
        relation E(x:i32,y:i32) = (edges);
        relation A(x:i32);
        relation B(x:i32);
        output A, B;
        FirstA: A(*x) :- E(x,y);
        B(*x) :- E(x,y);
        A(#second_a) :- E(x,y);
        A(*x) :- B(x);
        B(*x) :- A(x);
        strata { component [A,B]; }
        incremental { scheme seminaive_covering;
            delta A = RecentA;
            delta B = RecentB;
            variant 0 seed { A(*x) :- E(x,y); }
            variant 1 seed { B(*x) :- E(x,y); }
            variant 2 seed { A(#second_a) :- E(x,y); }
            variant 3 delta 0 { A(*x) :- RecentB(x); }
            variant 4 delta 0 { B(*x) :- RecentA(x); }
        }
    }).unwrap();
    let device = storage::compile(&host).unwrap();
    let mut pipeline = cq_datalog_fork::default_pipeline();
    pipeline.passes.last_mut().unwrap().path = syn::parse_quote!(::consumer::output);
    let binding = pipeline.passes.len() - 1;
    pipeline.passes.insert(binding, syn::parse_quote!(::consumer::inputs));
    let round = cq_datalog_fork::compile_with(&device, quote! {
        cq = #pipeline; lower = 2; terminals = [::consumer::output => batch];
    }).unwrap();
    (round, binding)
}

#[test]
fn grouped_abi_is_explicit_checked_per_head_and_per_phase() {
    use crate::bindings::{check, compile_with, head_argument};
    let (round, binding) = multihead_round(syn::parse_quote!(*y));
    let legacy = compile_with(&round, quote!(binding = #binding)).unwrap();
    assert!(!legacy.holes.iter().any(head_argument));
    let grouped = compile_with(&round, quote!(binding = #binding, head_cursor)).unwrap();
    if std::env::var_os("CQ_EXPANSION_COUNTERS").is_some() {
        assert_eq!(grouped, legacy);
        return;
    }
    assert!(grouped.holes.iter().all(head_argument));
    let grouped: cq_lang_datalog_round::Program = syn::parse2(quote!(#grouped)).unwrap();
    let a = grouped.context.strata[0].seeds[0].head;
    let a_seeds: Vec<_> = grouped.context.strata[0].seeds.iter()
        .filter(|rule| rule.head == a).map(|rule| rule.hole).collect();
    assert_eq!(a_seeds.len(), 2);
    let mut mixed = grouped.clone();
    for (i, &hole) in a_seeds.iter().enumerate() {
        mixed.holes[hole].pipeline.passes.last_mut().unwrap().arguments = quote!(&mut *__cq_gpu_arena);
        if i == 0 { assert!(check(&mixed).is_err()); }
    }
    // Explicit legacy is legal even when the whole head could use a cursor.
    check(&mixed).unwrap();
    let source = print::compile(&grouped).unwrap().to_token_stream().to_string();
    assert_eq!(source.matches("HeadCursors :: new").count(), 2);
    assert_eq!(source.matches(". read ()").count(), 2);
    assert_eq!(source.matches(". begin_head (").count(), 4);
    assert_eq!(source.matches(". finish_head (").count(), 4);
    assert_eq!(source.matches(". retry_head (").count(), 4);
    assert_eq!(source.matches("if __cq_gpu_trace_rounds { __cq_gpu_head_replays += 1").count(), 4);
    assert_eq!(source.matches(". clear ()").count(), 4);
    assert_eq!(source.matches(". trace_head_count ()").count(), grouped.holes.len());
    // Each macro queue appears in a standalone item, not at both call sites.
    assert_eq!(source.matches("consumer :: output").count(), grouped.holes.len());
    assert!(source.contains("if __cq_gpu_active"));
    assert!(source.contains("if let Some (__cq_gpu_batch)"));
    assert!(source.contains("else if __cq_gpu_trace_memory || :: gpu_datalog_runtime :: trace_kernels ()"));
    assert!(source.contains("checked_sub (* __cq_gpu_last)"));
    // The seed group's first read precedes every retry. All retry call sites
    // retain the original wave order and occur before the first admission.
    let (_, retry) = source.split_once("__cq_gpu_seed_group_0 . read ()").unwrap();
    let (mut retry, _) = retry.split_once(". fresh_batches (").unwrap();
    for wave in grouped.schedule.strata[0].waves.iter().filter(|w| w.phase == "seed") {
        for &i in &wave.holes {
            let hole = grouped.context.strata[0].seeds[i].hole;
            let function = format!("__cq_gpu_launch_{hole} (");
            retry = retry.split_once(function.as_str()).unwrap().1;
        }
    }
}

#[test]
fn hooked_version_keeps_its_whole_head_legacy_without_disabling_other_heads_or_phases() {
    use crate::bindings::{compile_with, head_argument};
    let (round, binding) = multihead_round(syn::parse_quote!(*y));
    let enabled = std::env::var_os("CQ_EXPANSION_COUNTERS").is_none();
    let unknown = compile_with(&round, quote!(binding = #binding, head_cursor, legacy("Typo@seed")));
    assert!(unknown.unwrap_err().to_string().contains("names no rule version"));
    let bound = compile_with(&round, quote!(binding = #binding, head_cursor, legacy("FirstA@seed"))).unwrap();
    let stratum = &bound.context.strata[0];
    let first = stratum.seeds.iter().find(|rule| {
        print::version_label(&bound, rule.variant, rule.head) == "FirstA@seed"
    }).unwrap();
    for rule in &stratum.seeds {
        assert_eq!(head_argument(&bound.holes[rule.hole]), enabled && rule.head != first.head);
    }
    for rule in &stratum.steps {
        assert_eq!(head_argument(&bound.holes[rule.hole]), enabled);
    }

    let mut hooks = print::HoleHooks::default();
    if enabled {
        let wrong = compile_with(&round, quote!(binding = #binding, head_cursor)).unwrap();
        for manual in [true, false] {
            let mut hook = print::HoleHooks::default();
            let paths = if manual { &mut hook.manual } else { &mut hook.observe };
            paths.insert("FirstA@seed".into(), syn::parse_quote!(my_hook));
            assert!(print::compile_with(&wrong, &hook).unwrap_err().to_string().contains("requires a legacy batch"));
        }
    }
    hooks.manual.insert("FirstA@seed".into(), syn::parse_quote!(first_manual));
    hooks.observe.insert("FirstA@seed".into(), syn::parse_quote!(first_observer));
    let printed = print::compile_with(&bound, &hooks).unwrap();
    let source = printed.to_token_stream().to_string();
    assert_eq!(source.matches("first_manual (").count(), 1);
    assert_eq!(source.matches("first_observer (").count(), 1);
    assert_eq!(source.matches("done (batch . len ())").count(), 1);
    if enabled {
        // Both hooks are on the initial legacy launch, outside all retry loops.
        let (initial, replay) = source.split_once("__cq_gpu_seed_group_0 . read ()").unwrap();
        assert!(initial.contains("first_manual (") && initial.contains("first_observer ("));
        assert!(!replay.contains("first_manual (") && !replay.contains("first_observer ("));
    }

    let syn::Expr::Block(block) = printed else { panic!("expected generated block") };
    for (i, hole) in bound.holes.iter().enumerate() {
        let name = quote::format_ident!("__cq_gpu_hole_{}", i);
        let signature = block.block.stmts.iter().find_map(|stmt| match stmt {
            syn::Stmt::Item(syn::Item::Fn(item)) if item.sig.ident == name => Some(&item.sig),
            _ => None,
        });
        if i == first.hole {
            assert!(signature.is_none(), "manual replacement should compile no kernel");
        } else {
            let expected: syn::ReturnType = if head_argument(hole) {
                syn::parse_quote!(-> ::core::option::Option<::gpu_datalog_runtime::CandidateBatch>)
            } else {
                syn::parse_quote!(-> ::gpu_datalog_runtime::CandidateBatch)
            };
            assert_eq!(signature.unwrap().output, expected);
        }
    }
}

#[test]
fn grouped_abi_rejects_cross_epoch_replay_and_arbitrary_expressions() {
    use crate::bindings::{check, compile_with, head_argument};
    let (mut round, binding) = multihead_round(syn::parse_quote!(*y));
    let seeds = &round.context.strata[0].seeds;
    let a = seeds[0].head;
    let a_indices: Vec<_> = seeds.iter().enumerate().filter(|(_, r)| r.head == a)
        .map(|(i, _)| i).collect();
    let b_index = seeds.iter().position(|r| r.head != a).unwrap();
    let mut waves = vec![
        cq_lang_datalog_round::schedule::Wave { phase: syn::parse_quote!(seed), holes: vec![a_indices[0], b_index] },
        cq_lang_datalog_round::schedule::Wave { phase: syn::parse_quote!(seed), holes: vec![a_indices[1]] },
    ];
    waves.extend(round.schedule.strata[0].waves.iter().filter(|w| w.phase == "step").cloned());
    round.schedule = round.schedule.with_stratum_waves(0, waves, &round.context);
    for (source, binding) in [
        (round, binding),
        multihead_round(syn::parse_quote!(next_id(*y))),
        multihead_round(syn::parse_quote!(*y + local_offset)),
    ] {
        let bound = compile_with(&source, quote!(binding = #binding, head_cursor)).unwrap();
        let mut forged = bound.clone();
        let enabled = std::env::var_os("CQ_EXPANSION_COUNTERS").is_none();
        for rule in &bound.context.strata[0].seeds {
            assert_eq!(head_argument(&bound.holes[rule.hole]), enabled && rule.head != a);
            forged.holes[rule.hole].pipeline.passes.last_mut().unwrap().arguments = quote!(head; &mut *__cq_gpu_arena);
        }
        assert!(check(&forged).is_err());
        // The step-phase writes to A read a different, stable epoch.
        for rule in &bound.context.strata[0].steps {
            assert_eq!(head_argument(&bound.holes[rule.hole]), enabled);
        }
        print::compile(&bound).unwrap();
    }
}

#[test]
fn frontier_prefixes_only_its_terminal_and_preserves_the_arena_abi() {
    let (round, binding) = multihead_round(syn::parse_quote!(*y));
    for head in [false, true] {
        let args = if head { quote!(binding = #binding, head_cursor) } else { quote!(binding = #binding) };
        let bound = crate::bindings::compile_with(&round, args).unwrap();
        let mut hooks = print::HoleHooks::default();
        hooks.frontier.insert("FirstA@seed".into());
        if !head {
            hooks.observe.insert("FirstA@seed".into(), syn::parse_quote!(watch));
        }
        let source = print::compile_with(&bound, &hooks).unwrap().to_token_stream().to_string();
        let expected = if head && std::env::var_os("CQ_EXPANSION_COUNTERS").is_none() {
            quote!(frontier; head; &mut *__cq_gpu_arena)
        } else {
            quote!(frontier; &mut *__cq_gpu_arena)
        };
        assert_eq!(source.matches("frontier ;").count(), 1);
        assert!(source.contains(&expected.to_string()));
        if !head {
            assert!(source.contains("watch ("));
        }
    }
}

#[test]
fn frontier_rejects_unknown_versions_and_manual_replacements() {
    let (round, binding) = multihead_round(syn::parse_quote!(*y));
    let bound = crate::bindings::compile_with(&round, quote!(binding = #binding)).unwrap();
    let mut hooks = print::HoleHooks::default();
    hooks.frontier.insert("Typo@0".into());
    assert!(print::compile_with(&bound, &hooks).unwrap_err().to_string().contains("`Typo@0` names no rule version"));
    hooks.frontier.clear();
    hooks.frontier.insert("FirstA@seed".into());
    hooks.manual.insert("FirstA@seed".into(), syn::parse_quote!(replace));
    assert!(print::compile_with(&bound, &hooks).unwrap_err().to_string().contains("`FirstA@seed` cannot be both manual and frontier"));
}
