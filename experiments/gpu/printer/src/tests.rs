use crate::{
    compiler::{compile, validate},
    options::Options,
    predicate::Constants,
};
use cq_lang_structures as ir;
use quote::{quote, ToTokens};
use std::collections::BTreeMap;

fn shared() -> ir::Program {
    syn::parse_quote! { physical {
        relation L(a:i64); relation R(a:i64,b:i64);
        l = pipe rename L@0 {a -> x};
        b = let rename R@1 {a -> x,b -> y};
        s = pipe semi eager [x] l,b;
        p = pipe canon probe first [x] s,b;
        return p as answer(x,y);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        b host scan = ::core::iter::Map<_,_> [];
        b rows = ::std::vec::Vec<_>;
        b = ::cq_storage::SortedRun<_,_,::cq_storage::BorrowedI64TupleCodec> [x];
        p canon = ::cq_gpu_runtime::DeviceSet<_>;
        s share = lookup b [x] first s force p;
        s repr = tuples witness { b [x] at s as ::cq_storage::RunCursor<'_,_> };
    } }
}

#[test]
fn pure_projection_uses_runtime_without_a_redundant_copy_kernel() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation Arbitrary(k:i64);
        scan = pipe rename Arbitrary@0 {k -> renamed};
        output = pipe canon project scan keep {renamed};
        return output as result(renamed);
    } structures {
        scan host scan = ::core::iter::Map<_,_> [];
        output canon = ::cq_gpu_runtime::DeviceSet<_>;
    } };
    let generated = compile(&source, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(!generated.contains("cuda_module"));
    assert!(generated.contains("__execution . dedup"));
    assert!(generated.contains("bind_host"));
    assert!(generated.contains("into_columns :: < (i64 ,) , :: cq_gpu_runtime :: Plain >"));
    assert!(!generated.contains("download"));
    assert!(!generated.contains("decode_value"));
    assert!(!generated.contains("into_iter"));
    assert_eq!(generated.matches("pub fn cq_gpu_stream_").count(), 0);
    assert!(generated.contains("project_table"));
    assert!(!generated.contains("prepare_run"));
    assert!(!generated.contains("blocked_run :: point"));
}

#[test]
fn every_keyed_read_has_exactly_one_provenance_entry_and_witness_does_not_reprobe() {
    let source = shared();
    let facts = ir::contract::check(&source).unwrap();
    let reads = validate(&source, &facts).unwrap();
    assert_eq!(
        reads.len(),
        facts
            .physical
            .keyed_reads
            .values()
            .map(Vec::len)
            .sum::<usize>()
    );
    assert_eq!(
        reads.iter().find(|r| r.reader == "s").unwrap().source,
        "rank-point"
    );
    assert_eq!(
        reads.iter().find(|r| r.reader == "p").unwrap().source,
        "witness:s"
    );
    assert!(reads.iter().all(|r| r.build == "b" && r.keys == ["x"]));
    let generated = compile(&source, syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert_eq!(generated.matches("blocked_run :: point").count(), 1);
    assert_eq!(generated.matches("prepare_run").count(), 1);
    assert_eq!(generated.matches("pub fn cq_gpu_stream_").count(), 1);
    assert!(generated.contains("cuda_module"));
    assert!(generated.contains("p:b[x]:witness:s"));
    assert!(generated
        .contains("let (__cq_gpu_begin_p , __cq_gpu_end_p) = (__cq_gpu_begin_s , __cq_gpu_end_s)"));
    assert!(generated.contains("while __cq_gpu_row_p < __cq_gpu_end_p"));
    assert!(!generated.contains("tiles :: owner"));
    assert!(generated.contains("let __cq_gpu_payload_p_y = __cq_gpu_run_view_b . columns"));
    assert!(generated.contains("__cq_gpu_payload_p_y . add (__cq_gpu_row_p as usize) . read ()"));
}

#[test]
fn unshared_first_probe_still_expands_every_range_row() {
    let mut source = shared();
    source.structures.entries.retain(|entry| {
        !matches!(
            entry.structure,
            ir::Structure::Share { .. } | ir::Structure::Repr { .. }
        )
    });
    let generated = compile(&source, syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert_eq!(generated.matches("blocked_run :: point").count(), 2);
    assert!(generated.contains("while __cq_gpu_row_p < __cq_gpu_end_p"));
    assert!(generated.contains("__cq_gpu_row_p += 1"));
}

#[test]
fn recorded_expand_enables_frontier_and_zero_disables_it() {
    let mut source = shared();
    source
        .structures
        .entries
        .push(syn::parse_quote!(p stage = batch 32 expand;));
    let enabled = compile(&source, syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(enabled.contains("frontier_claim"));
    assert!(enabled.contains("32usize"));
    let disabled = compile(
        &source,
        syn::parse2(quote!(capacity(R), frontier(0))).unwrap(),
    )
    .unwrap()
    .to_token_stream()
    .to_string();
    assert!(!disabled.contains("frontier_claim"));
    assert!(disabled.contains("false &&"));
}

#[test]
fn backend_rejection_codes_name_every_unsupported_operator() {
    let source = shared();
    let facts = ir::contract::check(&source).unwrap();
    // Exercise the backend policy independently: the language checker continues
    // to own validity of the complete graph; unsupported choices cannot hide in
    // an unused definition or require reaching a CUDA invocation to be rejected.
    for (definition, code) in [
        (quote!(bad = pipe merge [x] l,b;), "CQ_GPU_MERGE"),
        (quote!(bad = pipe union l,b;), "CQ_GPU_UNION"),
        (quote!(bad = pipe anti eager [x] l,b;), "CQ_GPU_ANTI"),
        (
            quote!(bad = pipe probe smallest [x] l,b;),
            "CQ_GPU_SMALLEST",
        ),
        (quote!(bad = pipe guard l if (true);), "CQ_GPU_OPAQUE_GUARD"),
        (
            quote!(bad = pipe bind l with z:i64 = (1);),
            "CQ_GPU_OPAQUE_BIND",
        ),
    ] {
        let mut target = source.clone();
        target
            .physical
            .program
            .definitions
            .push(syn::parse2(definition).unwrap());
        assert!(
            validate(&target, &facts)
                .unwrap_err()
                .to_string()
                .contains(code),
            "{code}"
        );
    }
}

#[test]
fn backend_rejection_codes_name_unsupported_recorded_storage() {
    let source = shared();
    let facts = ir::contract::check(&source).unwrap();
    for (entry, code) in [
        (
            quote!(b = ::cq_storage::Trie<_,_> [x][y];),
            "CQ_GPU_MULTILEVEL_BUILD",
        ),
        (
            quote!(b = ::cq_storage::HashIndex<'_,_,_> [x];),
            "CQ_GPU_BUILD_TYPE",
        ),
        (
            quote!(p canon = ::cq_storage::HashSet<_>;),
            "CQ_GPU_NON_DEVICE_CANON",
        ),
        (
            quote!(l host s = ::cq_storage::HashIndex<'_,_,_> [x];),
            "CQ_GPU_HOST_KEYED_FALLBACK",
        ),
    ] {
        let mut target = source.clone();
        target
            .structures
            .entries
            .insert(0, syn::parse2(entry).unwrap());
        assert!(
            validate(&target, &facts)
                .unwrap_err()
                .to_string()
                .contains(code),
            "{code}"
        );
    }
    let mut bad_types = facts;
    bad_types
        .physical
        .rows
        .get_mut("b")
        .unwrap()
        .types
        .insert("x".into(), syn::parse_quote!(String));
    assert!(validate(&source, &bad_types)
        .unwrap_err()
        .to_string()
        .contains("CQ_GPU_NON_INTEGRAL_KEY"));
}

#[test]
fn printer_options_are_explicit_and_checked() {
    let options: Options = syn::parse2(quote!(tile(256, 4), frontier(0), directory(rank))).unwrap();
    assert_eq!(options.frontier, 0);
    for (args, code) in [
        (quote!(tile(128, 4)), "CQ_GPU_TILE"),
        (quote!(directory(fences)), "CQ_GPU_DIRECTORY"),
        (
            quote!(frontier(1), frontier(2)),
            "CQ_GPU_DUPLICATE_ARGUMENT",
        ),
    ] {
        assert!(syn::parse2::<Options>(args)
            .unwrap_err()
            .to_string()
            .contains(code));
    }
}

#[test]
fn dictionary_order_is_never_silently_integer_order() {
    let vars = BTreeMap::from([("name".into(), quote!(__name))]);
    let types = BTreeMap::from([("name".into(), syn::parse_quote!(String))]);
    let theta = syn::parse_quote!(name < "z");
    assert!(Constants::default()
        .theta(&theta, &vars, &types)
        .unwrap_err()
        .to_string()
        .contains("CQ_GPU_STRING_ORDER"));
}

#[test]
fn recorded_prefix_run_preserves_all_residual_key_comparisons() {
    let source: ir::Program = syn::parse_quote! {physical {
        relation L(a:i64,b:i64); relation R(a:i64,b:i64,c:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = let rename R@1 {a -> x,b -> y,c -> z};
        p = pipe canon probe first [x,y] l,r;
        return p as answer(x,y,z);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        r rows = ::std::vec::Vec<_>;
        r = ::cq_gpu_runtime::PrefixRun<_,_,::cq_storage::BorrowedI64TupleCodec,0> [x][y];
        p canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    let facts = ir::contract::check(&source).unwrap();
    let reads = validate(&source, &facts).unwrap();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].keys, ["x", "y"]);
    assert_eq!(reads[0].source, "rank-point+residual");
    let generated = compile(&source, syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert_eq!(generated.matches("prepare_run").count(), 1);
    let residual = quote!(
        ::cq_gpu_runtime::run_column(__cq_gpu_run_view_r, 1usize, __cq_gpu_row_p)
            == __cq_gpu_tile_column_1
    )
    .to_string();
    assert!(generated.contains(&residual));
    assert!(generated.contains("while __cq_gpu_row_p < __cq_gpu_end_p"));
}

#[test]
fn recorded_membership_uses_its_bitmap_budget_and_reconstructs_constants() {
    let source: ir::Program = syn::parse_quote! {physical {
        relation L(k:i64); relation R(k:i64,name:String);
        l = pipe rename L@0 {k -> x};
        r = pipe rename R@1 {k -> x,name -> label};
        b = let point select r if (label = "tag");
        p = pipe canon probe first [x] l,b;
        return p as answer(x,label);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        r host b = ::core::iter::Map<_,_> [label];
        b = ::cq_storage::AdaptiveKeySet<_,::cq_storage::BorrowedI64TupleCodec,4096> [x]
            membership payload [x] constants {label = "tag"};
        p canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    let generated = compile(&source, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("prepare_membership"));
    assert!(generated.contains("4096usize"));
    assert!(generated.contains("membership_contains"));
    assert!(!generated.contains("prepare_run"));
    assert!(generated.contains("bitmap-membership"));
}

#[test]
fn recorded_join_filter_keeps_its_exact_covering_read_and_budget() {
    let mut source = shared();
    source.structures.entries.push(syn::parse_quote! {
        l join_filter = ::cq_storage::AdaptiveKeySet<i64, ::cq_storage::I64Codec, 8192>
            build b [x] covering s marker ::cq_load::ColumnAt<0>;
    });
    let facts = ir::contract::check(&source).unwrap();
    let reads = validate(&source, &facts).unwrap();
    assert_eq!(reads.len(), 2);
    assert_eq!(
        reads.iter().find(|read| read.reader == "s").unwrap().source,
        "rank-point"
    );
    let generated = compile(&source, syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert_eq!(generated.matches("prepare_membership").count(), 1);
    assert_eq!(generated.matches("membership_contains").count(), 1);
    assert_eq!(generated.matches("prepare_run").count(), 1);
    assert_eq!(generated.matches("blocked_run :: point").count(), 1);
    assert!(generated.contains("8192usize"));
    assert!(generated.contains("__CQ_GPU_JOIN_FILTER_READS"));
    assert!(generated.contains("l:join-filter:0:b[x]:covering=s"));
    assert!(generated.contains("s:b[x]:rank-point"));
    assert!(!generated.contains("positional_handle"));
    assert!(
        generated.find("membership_contains").unwrap()
            < generated.find("blocked_run :: point").unwrap()
    );
}

#[test]
fn join_filter_does_not_silently_replace_bloom_or_opaque_providers() {
    for ty in [
        quote!(::cq_join_filters::BloomFilter<16>),
        quote!(::user::AdaptiveKeySet<i64, ::cq_storage::I64Codec, 8192>),
    ] {
        let mut source = shared();
        source.structures.entries.push(syn::parse_quote! {
            l join_filter = #ty build b [x] covering s marker ::cq_load::ColumnAt<0>;
        });
        let error = compile(&source, Options::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains("CQ_GPU_JOIN_FILTER_TYPE"), "{error}");
    }
}

#[test]
fn provider_names_must_resolve_to_the_supported_recorded_paths() {
    for ty in [
        quote!(::user::SortedRun<_, _, ::cq_storage::BorrowedI64TupleCodec>),
        quote!(SortedRun<_, _, ::cq_storage::BorrowedI64TupleCodec>),
        quote!(::user::AdaptiveKeySet<_, ::cq_storage::BorrowedI64TupleCodec, 4096>),
    ] {
        let mut source = shared();
        let ir::Structure::Built { keyed, .. } = &mut source.structures.entries[3].structure else {
            unreachable!("fixture build");
        };
        keyed.ty = syn::parse2(ty).unwrap();
        let facts = ir::contract::check(&shared()).unwrap();
        let error = validate(&source, &facts).unwrap_err().to_string();
        assert!(error.contains("CQ_GPU_BUILD_TYPE"), "{error}");
    }
    let mut source = shared();
    let ir::Structure::Repr {
        witness: Some(witness),
        ..
    } = &mut source.structures.entries[6].structure
    else {
        unreachable!("fixture witness");
    };
    witness.ty = syn::parse_quote!(::user::RunCursor<'_, _>);
    let error = compile(&source, Options::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("CQ_GPU_WITNESS_TYPE"), "{error}");

    for ty in [
        quote!(::std::vec::Vec<_>),
        quote!(::alloc::vec::Vec<_>),
        quote!(Vec<_>),
    ] {
        let mut source = shared();
        let ir::Structure::Rows { ty: rows, .. } = &mut source.structures.entries[2].structure
        else {
            unreachable!("fixture rows");
        };
        *rows = syn::parse2(ty).unwrap();
        compile(&source, Options::default()).unwrap();
    }
    let mut source = shared();
    let ir::Structure::Rows { ty, .. } = &mut source.structures.entries[2].structure else {
        unreachable!("fixture rows");
    };
    *ty = syn::parse_quote!(::user::Vec<_>);
    let error = compile(&source, Options::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("CQ_GPU_ROWS_TYPE"), "{error}");
}

#[test]
fn resident_host_binding_preserves_codec_ownership_and_explicit_download() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation Input(k:i64,label:&str) = (&resident);
        scan = pipe rename Input@0 {k -> x,label -> name};
        output = pipe canon project scan keep {x,name};
        return output as answer(name,x,name);
    } structures {
        scan host scan = ::cq_gpu_runtime::DeviceColumns<'_,(i64,&str),::cq_gpu_runtime::Delta> [];
        output canon = ::cq_gpu_runtime::DeviceSet<_>;
    }};
    let generated = compile(&source, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("HostColumns :: pool"));
    assert!(generated.contains("with_runtime_for"));
    assert!(generated.contains("bind_host"));
    assert!(generated.contains("HostColumns :: seed_dictionary"));
    assert!(generated.find("seed_dictionary").unwrap() < generated.find("bind_host").unwrap());
    assert!(generated
        .contains("into_columns :: < (& str , i64 , & str ,) , :: cq_gpu_runtime :: Plain >"));
    assert!(generated.contains("& [0usize , 1usize , 0usize]"));
    assert!(generated.contains("__execution . dedup"));
    assert!(!generated.contains("Codec :: Delta"));
    assert!(!generated.contains("encode_slice"));
    assert!(!generated.contains("decode_value"));
    assert!(!generated.contains("download"));
}

#[test]
fn derived_codecs_follow_only_recorded_provider_types() {
    for (marker, codec) in [
        (quote!(::cq_gpu_runtime::Plain), "Plain"),
        (quote!(::cq_gpu_runtime::For), "For"),
        (quote!(::cq_gpu_runtime::Delta), "Delta"),
        (quote!(::cq_gpu_runtime::DictionaryCodec), "Dictionary"),
    ] {
        let mut source = shared();
        source.structures.entries[4] =
            syn::parse_quote!(p canon = ::cq_gpu_runtime::DeviceSet<_,#marker>;);
        let generated = compile(&source, Options::default())
            .unwrap()
            .to_token_stream()
            .to_string();
        assert!(
            generated.contains(&format!("Codec :: {codec}")),
            "{generated}"
        );
        assert!(generated.contains("__CQ_GPU_DERIVED_CODECS"));
        assert_eq!(generated.matches("__execution . dedup").count(), 1);
        assert!(generated.contains(&format!("(\"p\" , \"{codec}\")")));
    }
    let mut source = shared();
    source.structures.entries[2] = syn::parse_quote! {
        b rows = ::cq_gpu_runtime::DeviceColumns<_,::cq_gpu_runtime::For>;
    };
    let generated = compile(&source, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("(\"b\" , \"For\")"));
    assert!(generated.contains("into_columns :: < (i64 , i64 ,) , :: cq_gpu_runtime :: Plain >"));
    assert_eq!(generated.matches("__execution . dedup").count(), 1);
}

#[test]
fn unknown_codec_aliases_and_conflicting_materializations_are_rejected() {
    for marker in [
        quote!(For),
        quote!(::user::For),
        quote!(::cq_gpu_runtime::Unknown),
    ] {
        let mut source = shared();
        source.structures.entries[4] =
            syn::parse_quote!(p canon = ::cq_gpu_runtime::DeviceSet<_,#marker>;);
        let error = compile(&source, Options::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains("CQ_GPU_CODEC_TYPE"), "{error}");
    }
    let mut source = shared();
    source.physical.program.definitions[1].canon = Some(Default::default());
    source.structures.entries.push(syn::parse_quote! {
        b canon = ::cq_gpu_runtime::DeviceSet<_,::cq_gpu_runtime::For>;
    });
    let error = compile(&source, Options::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("CQ_GPU_CODEC_CONFLICT"), "{error}");
}

#[test]
fn capacity_is_explicit_and_names_a_declared_input() {
    let options: Options = syn::parse2(quote!(capacity(R), frontier(0))).unwrap();
    assert_eq!(options.capacity.as_ref().unwrap(), "R");
    let generated = compile(&shared(), options)
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("__cq_gpu_host_R"));
    assert!(generated.contains("__CQ_GPU_CAPACITY"));
    assert!(generated.contains("execute_bounded"));
    assert!(!generated.contains("execute_resumable"));
    let options: Options = syn::parse2(quote!(capacity(invented))).unwrap();
    let error = compile(&shared(), options).unwrap_err().to_string();
    assert!(error.contains("CQ_GPU_CAPACITY"), "{error}");
    assert!(syn::parse2::<Options>(quote!(capacity(L), capacity(R))).is_err());
}

#[test]
fn kernels_read_resident_logical_count_not_allocation_capacity() {
    let generated = compile(&shared(), syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("table_len (__driver)"));
    assert!(!generated.contains("__row < __driver . len"));
    assert!(generated.contains("__driver . len . div_ceil"));
}

#[test]
fn unbounded_expansion_uses_saved_device_execution_state() {
    let generated = compile(&shared(), Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("execute_resumable"));
    assert!(generated.contains("__resume_pc"));
    assert!(generated.contains("__resume_registers"));
    assert!(!generated.contains("execute_bounded"));
    assert!(!generated.contains("download"));
}

#[test]
fn resumable_expansion_preserves_recorded_batch_and_frontier() {
    let mut source = shared();
    source.structures.entries.push(syn::parse_quote! {
        p stage = batch 32 expand;
    });
    let generated = compile(&source, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("execute_resumable"));
    assert!(generated.contains("frontier_claim"));
    assert!(generated.contains("32usize"));
    assert!(generated.contains("p:b[x]:witness:s"));
}

#[test]
fn fixed_full_tile_scan_keeps_only_two_decoded_registers_live_per_column() {
    let generated = compile(&shared(), syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert_eq!(generated.matches("decode_tile_pair (").count(), 1);
    assert!(generated.contains("launch_bounds (256 , 4u32)"));
    assert!(generated.contains("__cq_gpu_root_column_0 : :: cq_gpu_runtime :: ColumnView"));
    assert!(generated.contains("column_view (__cq_gpu_host_L , 0usize)"));
    assert!(generated.contains("decode_tile_pair (__cq_gpu_root_column_0"));
    assert!(generated.contains("let [__cq_gpu_tile_register_0_0"));
    assert!(generated.contains("let __cq_gpu_tile_column_0 = if __stripe % 512 == 0"));
    assert!(generated.contains("while __pair < 2"));
    assert!(generated.contains("while __stripe < (__pair + 1) * 512"));
    assert!(!generated.contains("__cq_gpu_tile_register_0_2"));
    assert!(!generated.contains("[__stripe / 256]"));
    assert!(!generated.contains("sync_threads"));
    assert!(
        generated.find("decode_tile_pair (").unwrap() < generated.find("let mut __stripe").unwrap()
    );
    assert!(!generated.contains("column (__driver"));
    assert!(generated.contains("let __driver_len = :: cq_gpu_runtime :: table_len (__driver)"));
    assert!(generated.contains("while __tile < __driver_len . div_ceil"));
    assert!(generated.contains("__row < __driver_len"));
    let paired_slot = quote!(
        let __slot = (__tid / 32) * 128 + (__tid % 32) * 2
            + (__stripe / 512) * 64 + (__stripe / 256) % 2;
    )
    .to_string();
    assert!(generated.contains(&paired_slot));

    let mut source = shared();
    source.structures.entries.push(syn::parse_quote! {
        p stage = batch 32 expand;
    });
    let partial = compile(&source, syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(!partial.contains("decode_tile_pair ("));
    assert!(!partial.contains("__cq_gpu_root_column_0"));
    assert!(partial.contains("column (__driver"));
    assert!(!partial.contains(&paired_slot));
}

#[test]
fn selection_decodes_one_convergent_stripe_before_row_predicates() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation Arbitrary(k:i64,v:i64);
        scan = pipe rename Arbitrary@0 {k -> key,v -> value};
        selected = pipe select scan if (key > 0);
        output = pipe canon project selected keep {key,value};
        return output as result(key,value);
    } structures {
        scan host scan = ::core::iter::Map<_,_> [];
        output canon = ::cq_gpu_runtime::DeviceSet<_>;
    } };
    let generated = compile(&source, Options::default())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("execute_selection"));
    assert_eq!(generated.matches("decode_stripe (").count(), 2);
    assert!(generated.contains("column_view (__cq_gpu_host_Arbitrary , 0usize)"));
    assert!(generated.contains("column_view (__cq_gpu_host_Arbitrary , 1usize)"));
    assert!(generated.contains("__row / 1024"));
    assert!(generated.contains("(__row % 1024) / 256"));
    assert!(
        generated.find("decode_stripe (").unwrap() < generated.find("let mut __valid").unwrap()
    );
    assert!(!generated.contains("decode_tile_pair ("));
    assert!(!generated.contains("column (__driver"));
    assert!(!generated.contains("execute_resumable"));
}

#[test]
fn decoded_root_descriptors_come_from_the_materialized_table() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation Input(k:i64);
        stored = let rename Input@0 {k -> x};
        filtered = pipe select stored if (x > 0);
        output = pipe canon project filtered keep {x};
        return output as result(x);
    } structures {
        stored host scan = ::core::iter::Map<_,_> [];
        stored rows = ::std::vec::Vec<_>;
        output canon = ::cq_gpu_runtime::DeviceSet<_>;
    } };
    let generated = compile(&source, syn::parse2(quote!(capacity(Input))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains("decode_tile_pair (__cq_gpu_root_column_0"));
    assert!(generated.contains("column_view (__cq_gpu_table_stored , 0usize)"));
    assert!(!generated.contains("column_view (__cq_gpu_host_Input"));
}

#[test]
fn generated_module_cache_is_thread_local_and_context_owned() {
    let generated = compile(&shared(), syn::parse2(quote!(capacity(R))).unwrap())
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(generated.contains(":: std :: thread_local !"));
    assert!(generated.contains(":: std :: sync :: Arc < :: cuda_core :: CudaContext >"));
    assert!(generated.contains(":: std :: rc :: Rc <"));
    assert!(generated.contains("Arc :: ptr_eq (__context , __runtime . context ())"));
    assert_eq!(
        generated
            .matches(":: load (__runtime . context ())")
            .count(),
        1
    );
    assert!(generated.contains("__cache . borrow_mut ()"));
}
