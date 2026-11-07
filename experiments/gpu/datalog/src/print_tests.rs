use crate::{print, storage};
use quote::{ToTokens, quote};

fn program() -> cq_lang_datalog_round::Program {
    program_with_symbols(syn::parse_quote!(i32), quote!())
}

fn program_with_symbols(
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
        host.certificates.clear();
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
    let bound = crate::bindings::compile_with(&round, quote!(binding = #binding)).unwrap();
    cq_lang_datalog_round::check_storage(&device, &bound).unwrap();
    for (before, after) in round.holes.iter().zip(&bound.holes) {
        assert_eq!(before.cq.declarations, after.cq.declarations);
    }
    let key = &bound.context.strata[0].keys[0];
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
    assert!(source.contains("concatenate_batches"));
    // Admission is read-only fresh; publication appends to every layout.
    assert!(source.contains(". fresh ("));
    assert!(!source.contains(". admit ("));
    assert!(source.contains("append_sorted"));
    assert!(source.contains("RadixLayout"));
    assert!(source.contains("DeviceRadixRun"));
    assert!(source.contains("RadixRunParams"));
    assert!(source.contains("radix_bits"));
    assert!(source.contains("with_sorted"));
    assert!(source.contains(". republish (&"));
    assert!(source.contains("while"));
    assert!(source.contains("translate_arena"));
    assert_eq!(
        source.matches("to_rows").count(),
        program.context.outputs.len()
    );
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
        source.matches("release_cached_scratch").count(),
        program.context.strata.len()
    );
    assert_eq!(
        source.matches("release_unused").count(),
        program.context.strata.len()
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
    assert!(source.contains("sort_known_unique"));
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
    let target = crate::bind_inputs(
        &physical,
        &source,
        quote! {
            providers { #declaration }
            Delta => @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word #word; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,i32,),(),#word,u64,()> ;
            Edge => @radix_relation input { keys 2; columns [i32 => i32, i32 => i32] } radix { word #word; offset u64; ratio 4; radix_bits 0; merge_tile 32 } as ::gpu_datalog_runtime::RadixLayout<(i32,i32,),(),#word,u64,()> ;
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
