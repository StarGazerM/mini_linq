use cq_lang_physical as physical;
use cq_lang_structures::{self as ir, Structure};
use quote::{ToTokens, quote};
fn physical() -> physical::Program {
    syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(c:i64,d:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = pipe rename R@1 {c -> x,d -> z};
        s = let point select r if (z = 1);
        t = let semi eager [x] l,s;
        u = pipe semi eager [x] s,t;
        j = pipe probe first [x] u,t;
        out = let canon project j keep {x,y}; return out as answer(x,y);
    }}
}
fn selected() -> ir::Program {
    cq_structure_selection::compile_with(&physical(), quote!(positional(L, R))).unwrap()
}
#[test]
fn decisions_roundtrip_and_physical_is_identical() {
    let source = physical();
    let target = selected();
    assert_eq!(source, target.physical);
    assert_eq!(
        source.to_token_stream().to_string(),
        target.physical.to_token_stream().to_string()
    );
    assert_eq!(
        syn::parse2::<ir::Program>(target.to_token_stream()).unwrap(),
        target
    );
    let facts = ir::contract::check(&target).unwrap();
    assert_eq!(facts.selections.len(), 2);
    assert_eq!(facts.keysets.len(), 2);
    assert!(
        target
            .to_token_stream()
            .to_string()
            .contains("AdaptiveKeySet < _ , :: cq_storage :: I64Codec , 67108864 >")
    );
    assert_eq!(facts.reads.len(), 3);
    assert_eq!(
        facts
            .retained
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["out"]
    );
    assert_eq!(
        cq_structure_selection::compile_with(&source, quote!()).unwrap(),
        cq_structure_selection::compile(&source).unwrap()
    );
}
#[test]
fn lineage_and_summary_forgery_are_rejected_independently() {
    for alteration in 0..4 {
        let mut target = selected();
        for e in &mut target.structures.entries {
            match (&mut e.structure, alteration) {
                (Structure::Selection { origin, .. }, 0) => {
                    origin.base = syn::parse_quote!(L);
                    break;
                }
                (Structure::Selection { origin, .. }, 1) => {
                    origin.columns.pop();
                    break;
                }
                (Structure::Selection { origin, .. }, 2) => {
                    origin.columns[1].column = origin.columns[0].column.clone();
                    break;
                }
                (
                    Structure::Read {
                        predicate: ir::ReadPredicate::Membership { build, .. },
                        ..
                    },
                    3,
                ) => {
                    *build = syn::parse_quote!(j);
                    break;
                }
                _ => {}
            }
        }
        assert!(
            ir::contract::check(&target).is_err(),
            "forgery {alteration}"
        );
    }
}

#[test]
fn positional_marker_capabilities_are_rustc_obligations() {
    let mut target = selected();
    let entry = target
        .structures
        .entries
        .iter_mut()
        .find(|entry| matches!(entry.structure, Structure::Read { .. }))
        .unwrap();
    let Structure::Read { marker, .. } = &mut entry.structure else {
        unreachable!()
    };
    *marker = syn::parse_quote!(CustomColumnMarker);
    ir::contract::check(&target).unwrap();
}

#[test]
fn keyset_provider_and_codec_capabilities_are_rustc_obligations() {
    for (token, ty) in [
        (
            "std_hash_set",
            syn::parse_quote!(::std::collections::HashSet<_>),
        ),
        (
            "adaptive_keyset",
            syn::parse_quote!(::cq_storage::AdaptiveKeySet<_, ::cq_storage::BorrowedI64TupleCodec, 64>),
        ),
    ] {
        let mut target = selected();
        let entry = target
            .structures
            .entries
            .iter_mut()
            .find(|entry| matches!(entry.structure, Structure::KeySet { .. }))
            .unwrap();
        let Structure::KeySet { provider, .. } = &mut entry.structure else {
            unreachable!()
        };
        *provider = cq_lang_structures::provider::base::instance(token, ty);
        target
            .structures
            .declare(&[cq_lang_structures::provider::base::declaration(token)])
            .unwrap();
        // Lineage, ownership and demand are checked here; provider/codec
        // implementations for those recorded keys are checked by Rust.
        ir::contract::check(&target).unwrap();
    }
}
#[test]
fn representation_rewrite_cannot_change_an_execution_choice() {
    let source = selected();
    let mut target = source.clone();
    if let physical::Operator::Semi(_, k) = &mut target.physical.program.definitions[3].operator {
        k.choice = Some(syn::parse_quote!(lazy));
    }
    assert!(ir::check_rewrite(&source, &target).is_err());
}
#[test]
fn duplicate_and_missing_summaries_and_false_payload_indices_fail() {
    let source = selected();
    let mut duplicate = source.clone();
    duplicate
        .structures
        .entries
        .push(source.structures.entries.last().unwrap().clone());
    assert!(ir::contract::check(&duplicate).is_err());
    for kind in ["read", "build"] {
        let mut target = source.clone();
        target.structures.entries.retain(|e| {
            if kind == "read" {
                !matches!(e.structure, Structure::Read { .. })
            } else {
                !matches!(e.structure, Structure::Built { .. })
            }
        });
        assert!(ir::contract::check(&target).is_err());
    }
}

#[test]
fn index_only_rhs_prunes_dependent_candidates_without_changing_the_old_representation() {
    let physical = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation H(c:i64); relation R(d:i64,e:i64);
        l = pipe rename L@0 {a -> x,b -> y}; h = pipe rename H@1 {c -> x};
        a = let semi eager [x] l,h;
        r = pipe rename R@2 {d -> x,e -> z};
        t = let semi eager [x] r,a;
        out = let canon project t keep {x,z}; return out as answer(x,z);
    }};
    let original = cq_structure_selection::compile(&physical).unwrap();
    let selected =
        cq_structure_selection::compile_with(&physical, quote!(positional(L, H, R))).unwrap();
    assert_eq!(original, selected);
}

#[test]
fn eager_semi_with_joined_support_admits_left_positional_keyset() {
    let source = syn::parse_quote! { physical {
        relation L(a:i64); relation R(b:i64); relation S(c:i64);
        l = pipe rename L@0 {a -> x};
        r = pipe rename R@1 {b -> x};
        s = pipe rename S@2 {c -> x};
        support = let probe first [x] r,s;
        filtered = let semi eager [x] l,support;
        outer = pipe rename L@3 {a -> x};
        out = let canon semi eager [x] outer,filtered;
        return out as answer(x);
    }};
    let mut target = cq_structure_selection::compile(&source).unwrap();
    target.structures.entries.push(syn::parse_quote!(
        filtered rows = @vec as ::std::vec::Vec<_>;
    ));
    target.structures.entries.push(syn::parse_quote!(
        filtered keyset =
            @adaptive_keyset as ::cq_storage::AdaptiveKeySet<_, ::cq_storage::NoDense, 67108864>
            [x] owned base L lineage { x -> a };
    ));
    target
        .structures
        .declare(&[
            cq_lang_structures::provider::base::declaration("vec"),
            cq_lang_structures::provider::base::declaration("adaptive_keyset"),
        ])
        .unwrap();
    ir::contract::check(&target).unwrap();
}

fn join_filter_plan(branch: bool) -> ir::Program {
    let build = if branch {
        quote! {
            h = pipe rename H@2 {e -> z,f -> w};
            b = let probe first [z] s,h;
        }
    } else {
        quote!(b = let project s keep {x,z};)
    };
    let source: physical::Program = syn::parse2(quote! { physical {
        relation L(a:i64,b:i64); relation R(c:i64,d:i64); relation H(e:i64,f:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = pipe rename R@1 {c -> x,d -> z};
        s = pipe range select r if (z > 1);
        #build
        j = pipe probe first [x] l,b;
        out = let canon project j keep {x,y,z}; return out as answer(x,y,z);
    }})
    .unwrap();
    cq_structure_selection::compile_with(&source, quote!(positional(L, R, H))).unwrap()
}

#[test]
fn join_filters_preserve_coverage_and_lineage() {
    for branch in [false, true] {
        let selected = join_filter_plan(branch);
        let facts = ir::contract::check(&selected).unwrap();
        assert_eq!(facts.join_filters["l"].len(), 1);
        let filter = &facts.join_filters["l"][0];
        assert_eq!(
            filter.provider,
            syn::parse_quote!(@join_filter_membership as ::cq_storage::DenseMembership<_, 67108864>)
        );
        assert_eq!(filter.covering, "j");
        assert_eq!(
            syn::parse2::<ir::Program>(selected.to_token_stream()).unwrap(),
            selected
        );
        for forgery in 0..3 {
            let mut bad = selected.clone();
            for entry in &mut bad.structures.entries {
                if let Structure::JoinFilter { filter, .. } = &mut entry.structure {
                    match forgery {
                        0 => filter.covering = syn::parse_quote!(out),
                        1 => filter.build = syn::parse_quote!(s),
                        _ => filter.key = syn::parse_quote!([z]),
                    }
                }
            }
            assert!(ir::contract::check(&bad).is_err(), "forgery {forgery}");
        }
    }
}

#[test]
fn borrowed_run_filter_is_a_checked_recorded_scalar_realization() {
    let original = join_filter_plan(false);
    let selected = cq_structure_selection::retype(&original, quote!(
        b => @sorted_run as ::cq_storage::SortedRun<_, _> supports [keys]
    )).unwrap();
    let default_facts = ir::contract::check(&selected).unwrap();
    let default_filter = &default_facts.join_filters["l"][0];
    assert!(!default_facts.facts(&default_filter.provider).borrowed_filter);
    assert!(!default_facts.facts(&default_filter.provider).attached_filter);
    assert!(default_filter.abandon.as_ref().unwrap().statistic.is_none());
    let selected = cq_structure_selection::retype(&selected, quote!(
        l join_filter => @borrowed_run_filter as ::cq_storage::RunFilterView<'_, _, _>
    )).unwrap();
    let facts = ir::contract::check(&selected).unwrap();
    let filter = &facts.join_filters["l"][0];
    assert!(facts.facts(&filter.provider).borrowed_filter);
    assert_eq!(filter.abandon.as_ref().unwrap().statistic.as_ref().unwrap().accessor, "stat_keys");
    assert_eq!(syn::parse2::<ir::Program>(selected.to_token_stream()).unwrap(), selected);
    for invalid_accessor in [false, true] {
        let mut bad = selected.clone();
        for entry in &mut bad.structures.entries {
            if let Structure::JoinFilter { filter, .. } = &mut entry.structure {
                if invalid_accessor {
                    filter.abandon.as_mut().unwrap().statistic.as_mut().unwrap().accessor = syn::parse_quote!(stat_rows);
                } else {
                    filter.key = syn::parse_quote!([z]);
                }
            }
        }
        assert!(ir::contract::check(&bad).is_err());
    }
    for mutation in 0..3 {
        let mut bad = selected.clone();
        let provider = bad.structures.providers.declarations.iter_mut()
            .find(|provider| provider.token == "sorted_run").unwrap();
        let statistic = provider.facts.statistics.iter_mut()
            .find(|statistic| statistic.field == ir::stats::Field::Keys).unwrap();
        match mutation {
            0 => statistic.placement = ir::stats::Placement::Cuda,
            1 => statistic.precision = ir::stats::Precision::Estimate,
            _ => statistic.computation = ir::stats::Computation::Additional,
        }
        physical::contract::check(&bad.physical).unwrap();
        assert!(ir::contract::check(&bad).is_err());
    }
    assert!(cq_structure_selection::retype(&selected, quote!(
        b => @hash_index as ::cq_storage::HashIndex<'_, _, _> supports [keys]
    )).is_err());
    let projected = cq_structure_selection::retype(&selected, quote!(
        l join_filter => @join_filter_membership as ::cq_storage::DenseMembership<_, 67108864>
    )).unwrap();
    let changed = cq_structure_selection::retype(&projected, quote!(
        b => @hash_index as ::cq_storage::HashIndex<'_, _, _> supports [keys]
    )).unwrap();
    let changed_facts = ir::contract::check(&changed).unwrap();
    assert!(!changed_facts.facts(&changed_facts.join_filters["l"][0].provider).borrowed_filter);
}

#[test]
fn attached_join_filter_requires_explicit_unambiguous_realization() {
    let original = join_filter_plan(false);
    let selected = cq_structure_selection::retype(&original, quote!(
        l join_filter => @attached_join_filter_membership as ::cq_storage::AttachedDenseMembership<_, 67108864>
    )).unwrap();
    let facts = ir::contract::check(&selected).unwrap();
    let filter = &facts.join_filters["l"][0];
    assert!(facts.facts(&filter.provider).attached_filter);
    assert!(!facts.facts(&filter.provider).borrowed_filter);
    assert_eq!(syn::parse2::<ir::Program>(selected.to_token_stream()).unwrap(), selected);
    let mut bad = selected.clone();
    let provider = bad.structures.providers.declarations.iter_mut()
        .find(|provider| provider.token == "attached_join_filter_membership").unwrap();
    provider.facts.borrowed_filter = true;
    let error = ir::contract::check(&bad).err().expect("conflicting realization flags");
    assert!(error.to_string().contains("borrowed_filter and attached_filter are mutually exclusive"));
}

#[test]
fn membership_provider_selection_is_type_blind_and_has_no_redundant_filter() {
    for key_type in [
        quote!(i64),
        quote!(i32),
        quote!(u32),
        quote!(::opaque::KeyAlias),
        quote!((::opaque::KeyAlias)),
        quote!((i64, i64)),
    ] {
        let source: physical::Program = syn::parse2(quote! { physical {
            relation L(a:#key_type); relation R(a:#key_type,b:i64);
            l = pipe rename L@0 {a -> x};
            r = pipe rename R@1 {a -> x,b -> y};
            selected = let range select r if (y > 1);
            joined = pipe probe first [x] l,selected;
            result = let canon project joined keep {x};
            return result as answer(x);
        }})
        .unwrap();
        let selected =
            cq_structure_selection::compile_with(&source, quote!(positional(L, R))).unwrap();
        assert_eq!(source, selected.physical);
        let facts = ir::contract::check(&selected).unwrap();
        assert!(
            facts
                .membership
                .contains(&("selected".into(), vec!["x".into()]))
        );
        assert_eq!(
            facts.built["selected"][0].1,
            syn::parse_quote!(@dense_membership as ::cq_storage::DenseMembership<_, 16777216>)
        );
        assert!(facts.join_filters.is_empty());
    }
}

#[test]
fn join_filter_capability_selection_does_not_inspect_key_types() {
    let original = join_filter_plan(false);
    for key_type in [quote!(i32), quote!(u32), quote!(::opaque::KeyAlias)] {
        let mut source = original.physical.clone();
        for declaration in &mut source.program.declarations {
            for column in &mut declaration.columns {
                column.ty = syn::parse2(key_type.clone()).unwrap();
            }
        }
        let selected =
            cq_structure_selection::compile_with(&source, quote!(positional(L, R, H))).unwrap();
        assert_eq!(original.structures, selected.structures);
    }
}

#[test]
fn join_filter_cannot_cross_an_opaque_guard() {
    let mut selected = join_filter_plan(true);
    let index = selected
        .physical
        .program
        .definitions
        .iter()
        .position(|d| d.name == "j")
        .unwrap();
    selected.physical.program.definitions.insert(
        index,
        syn::parse_quote!(g = pipe guard l if (panic!("must not disappear"));),
    );
    if let physical::Operator::Probe(_, k) =
        &mut selected.physical.program.definitions[index + 1].operator
    {
        k.left = syn::parse_quote!(g);
    }
    assert!(ir::contract::check(&selected).is_err());
}

/// A filter reaches the scan through an intermediate probe only on a key
/// column the scan itself carries; a key bound by that probe's build is not
/// a scan column, so filtering the scan by it would be unsound.
#[test]
fn join_filters_cross_probes_only_on_scan_key_columns() {
    let plan = |key: proc_macro2::TokenStream| {
        let source: physical::Program = syn::parse2(quote! { physical {
            relation L(a:i64,b:i64); relation R(c:i64,d:i64); relation H(e:i64,f:i64);
            l = pipe rename L@0 {a -> x,b -> y};
            h = pipe rename H@2 {e -> y,f -> w};
            m = let range select h if (w > 1);
            p = pipe probe first [y] l,m;
            r = pipe rename R@1 {c -> #key,d -> z};
            s = pipe range select r if (z > 1);
            b = let project s keep {#key,z};
            j = pipe probe first [#key] p,b;
            out = let canon project j keep {x,y,z}; return out as answer(x,y,z);
        }})
        .unwrap();
        let selected =
            cq_structure_selection::compile_with(&source, quote!(positional(L, R, H))).unwrap();
        let facts = ir::contract::check(&selected).unwrap();
        let mut found: Vec<_> = facts
            .join_filters
            .get("l")
            .into_iter()
            .flatten()
            .map(|f| f.covering.to_string())
            .collect();
        found.sort();
        found
    };
    assert!(plan(quote!(x)).contains(&"j".to_owned()));
    assert!(!plan(quote!(w)).contains(&"j".to_owned()));
}

fn repeated_filter_plan(distinct_build: bool, distinct_layout: bool) -> ir::Program {
    let version = distinct_build.then(|| quote!(c = let project b keep {x,z};));
    let build = if distinct_build { quote!(c) } else { quote!(b) };
    let key = if distinct_layout {
        quote!(x, z)
    } else {
        quote!(x)
    };
    let first = if distinct_layout || distinct_build {
        quote!(probe first)
    } else {
        quote!(semi eager)
    };
    let projection = distinct_build.then(|| quote!(p = pipe project q keep {x,y};));
    let input = if distinct_build { quote!(p) } else { quote!(q) };
    let source: physical::Program = syn::parse2(quote! { physical {
        relation L(a:i64,b:i64); relation R(c:i64,d:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = pipe rename R@1 {c -> x,d -> z};
        s = pipe range select r if (z > 1);
        b = let project s keep {x,z};
        #version
        q = pipe #first [x] l,b;
        #projection
        j = pipe probe first [#key] #input,#build;
        out = let canon project j keep {x,z}; return out as answer(x,z);
    }})
    .unwrap();
    let selected = cq_structure_selection::compile_with(&source, quote!(positional(L, R))).unwrap();
    assert_eq!(selected.physical, source, "exact operators must remain");
    selected
}

#[test]
fn equivalent_filter_witnesses_share_checked_runtime_work() {
    let selected = repeated_filter_plan(false, false);
    let witnesses: Vec<_> = selected
        .structures
        .entries
        .iter()
        .filter_map(|entry| match &entry.structure {
            Structure::JoinFilter { filter, .. } if entry.name == "l" => Some(filter),
            _ => None,
        })
        .collect();
    assert_eq!(witnesses.len(), 2, "retain both validation witnesses");
    assert_eq!(
        ir::contract::check(&selected).unwrap().join_filters["l"].len(),
        1
    );
    assert_eq!(
        syn::parse2::<ir::Program>(selected.to_token_stream()).unwrap(),
        selected
    );

    // A witness that would otherwise be coalesced is still checked.
    // Column marker identity is a Rust ColumnSlot obligation, exercised by
    // provider-contract compile-fail tests rather than provider-name inspection.
    let mut invalid = selected.clone();
    for entry in &mut invalid.structures.entries {
        if let Structure::JoinFilter { filter, .. } = &mut entry.structure {
            if filter.covering == "j" {
                filter.covering = syn::parse_quote!(out);
            }
        }
    }
    assert!(ir::contract::check(&invalid).is_err());

    let mut different_type = selected;
    for entry in &mut different_type.structures.entries {
        if let Structure::JoinFilter { filter, .. } = &mut entry.structure {
            if filter.covering == "j" {
                filter.provider =
                    syn::parse_quote!(@other_conservative_filter as OtherConservativeFilter);
            }
        }
    }
    different_type.structures.declare(&[syn::parse_quote!(
        other_conservative_filter = OtherConservativeFilter { reads [membership], build inferred };
    )]).unwrap();
    assert_eq!(
        ir::contract::check(&different_type).unwrap().join_filters["l"].len(),
        2
    );
}

#[test]
fn filter_build_versions_and_key_layouts_remain_distinct() {
    let versions = repeated_filter_plan(true, false);
    assert_eq!(
        ir::contract::check(&versions).unwrap().join_filters["l"].len(),
        2
    );
    let layouts = repeated_filter_plan(false, true);
    let facts = ir::contract::check(&layouts).unwrap();
    assert_eq!(facts.join_filters["l"].len(), 2);
    assert_eq!(
        facts.join_filters["l"]
            .iter()
            .filter(|f| f.key == syn::parse_quote!([x]))
            .count(),
        2
    );
}

#[test]
fn filters_do_not_share_across_occurrences_of_one_host() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(c:i64,d:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        m = pipe rename L@2 {a -> x,b -> y};
        r = pipe rename R@1 {c -> x,d -> z};
        b = let range select r if (z > 1);
        p = pipe probe first [x] l,b;
        q = pipe probe first [x] m,b;
        out = let canon union p,q; return out as answer(x,y,z);
    }};
    let selected = cq_structure_selection::compile_with(&source, quote!(positional(L, R))).unwrap();
    let facts = ir::contract::check(&selected).unwrap();
    assert_eq!(facts.join_filters["l"].len(), 1);
    assert_eq!(facts.join_filters["m"].len(), 1);
    assert_eq!(selected.physical, source);
}

#[test]
fn filters_preserve_distinct_scalar_key_mappings_and_markers() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(c:i64,d:i64,e:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = pipe rename R@1 {c -> x,d -> y,e -> z};
        b = let range select r if (y > 1);
        q = pipe semi eager [x,y] l,b;
        j = pipe probe first [x,y] q,b;
        out = let canon project j keep {x,y,z}; return out as answer(x,y,z);
    }};
    // Keep a payload demand so the build records an enumerable HashIndex,
    // rather than membership-only HashSet storage.
    let selected = cq_structure_selection::compile_with(&source, quote!(positional(L, R))).unwrap();
    let facts = ir::contract::check(&selected).unwrap();
    let filters = &facts.join_filters["l"];
    assert_eq!(filters.len(), 2);
    assert_ne!(filters[0].key, filters[1].key);
    assert_ne!(filters[0].marker, filters[1].marker);
}

fn block_source() -> physical::Program {
    syn::parse_quote! { physical {
        relation R(a: OpaqueAlias [17], b: OtherSpelling [17]);
        r = pipe rename R@0 { a -> x, b -> y };
        p = pipe range select r if (x >= 3);
        g = pipe guard p if (opaque(y));
        v = pipe bind g with value: u64 = (compute(x));
        out = let canon project v keep {x, value};
        return out as answer(x, value);
    }}
}

fn block_advice() -> proc_macro2::TokenStream {
    quote!(blocks(R {
        block_rows 1024; summary none;
        columns [a: bytes u64, b: plain u64];
    }))
}

#[test]
fn blocks_are_authored_complete_printable_and_preserve_physical() {
    let source = block_source();
    let ordinary = cq_structure_selection::compile(&source).unwrap();
    assert_eq!(ordinary.encoded, Default::default());
    let target = cq_structure_selection::compile_with(&source, block_advice()).unwrap();
    cq_structure_selection::check(&source, &target).unwrap();
    assert_eq!(target.physical, source);
    assert_eq!(syn::parse2::<ir::Program>(target.to_token_stream()).unwrap(), target);
    let facts = ir::contract::check(&target).unwrap();
    for result in ["r", "p", "g", "v", "out"] {
        assert!(matches!(facts.repr[result], ir::Repr::Positions { .. }));
    }
    let blocks = &target.encoded.blocks;
    assert_eq!(blocks.scans["r"].columns.len(), 2);
    assert_eq!(blocks.predicates["p"].strategy, ir::blocks::Strategy::Window);
    assert_eq!(blocks.reads["g"].fields.keys().map(String::as_str).collect::<Vec<_>>(), ["y"]);
    assert_eq!(blocks.reads["v"].fields.keys().map(String::as_str).collect::<Vec<_>>(), ["x"]);
    assert_eq!(blocks.reads["canon:out"].purpose, ir::encoded::ReadPurpose::PreserveCarrier);
    assert_eq!(blocks.reads["canon:out"].fields["x"].action, ir::blocks::ReadAction::Ordinal);
}

#[test]
fn blocks_reject_partial_schema_and_unknown_hosts_without_type_recognition() {
    for advice in [
        quote!(blocks(R { block_rows 1024; summary none; columns [a: bytes u64]; })),
        quote!(blocks(Other { block_rows 1024; summary none; columns [a: bytes u64, b: plain u64]; })),
        quote!(blocks(R { block_rows 0; summary none; columns [a: bytes u64, b: plain u64]; })),
    ] {
        assert!(cq_structure_selection::compile_with(&block_source(), advice).is_err());
    }
    let mut source = block_source();
    source.program.declarations[0].columns[0].ty = syn::parse_quote!(UnrelatedRustTokens);
    let selected = cq_structure_selection::compile_with(&source, block_advice()).unwrap();
    assert_eq!(selected.encoded.blocks.sources.values().next().unwrap().columns["a"].logical, physical::syntax::LogicalTypeId(17));
}

#[test]
fn blocks_keep_exact_disjunction_and_cross_column_predicates() {
    for condition in [
        quote!(x != 3),
        quote!(x < y),
        quote!((x < 3 and y > 9) or (x > 7 and y < 2)),
    ] {
        let source: physical::Program = syn::parse2(quote! { physical {
            relation R(a: Alias [17], b: Alias [17]);
            r = pipe rename R@0 { a -> x, b -> y };
            p = pipe select r if (#condition);
            return canon p as answer(x,y);
        }}).unwrap();
        let target = cq_structure_selection::compile_with(&source, block_advice()).unwrap();
        assert_eq!(target.physical, source);
        let predicate = &target.encoded.blocks.predicates["p"];
        assert_eq!(predicate.strategy, ir::blocks::Strategy::Formula);
        assert_eq!(predicate.test, ir::blocks::Test::DecodeCodes);
        assert_eq!(predicate.prune, ir::blocks::Prune::None);
        let mut forged = target.clone();
        forged.encoded.blocks.predicates.get_mut("p").unwrap().strategy = ir::blocks::Strategy::Point;
        assert!(ir::contract::check(&forged).is_err());
        let mut forged = target.clone();
        forged.encoded.blocks.predicates.get_mut("p").unwrap().test = ir::blocks::Test::Encoded;
        assert!(ir::contract::check(&forged).is_err());
    }
}

#[test]
fn blocks_canonical_key_demand_cannot_be_replaced_by_positions() {
    let mut target = cq_structure_selection::compile_with(&block_source(), block_advice()).unwrap();
    target.encoded.blocks.reads.get_mut("canon:out").unwrap().fields.clear();
    assert!(ir::contract::check(&target).is_err());
    let mut target = cq_structure_selection::compile_with(&block_source(), block_advice()).unwrap();
    target.encoded.blocks.reads.get_mut("v").unwrap().fields.get_mut("x").unwrap().action = ir::blocks::ReadAction::BorrowedScoped;
    assert!(ir::contract::check(&target).is_err());
}

#[test]
fn blocks_join_and_index_boundaries_reject_before_printing() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a: Alias [17], b: Alias [17]);
        relation S(c: Alias [17]);
        r = pipe rename R@0 {a -> x, b -> y};
        s = let rename S@1 {c -> x};
        j = pipe probe first [x] r,s;
        return canon j as answer(x,y);
    }};
    cq_structure_selection::compile(&source).unwrap();
    assert!(cq_structure_selection::compile_with(&source, block_advice()).is_err());
    // A forged target with complete source declarations still fails the language
    // checker: changing the consumer is not a printer fallback.
    let mut target = cq_structure_selection::compile(&source).unwrap();
    let selected = cq_structure_selection::compile_with(&block_source(), block_advice()).unwrap();
    target.encoded = selected.encoded;
    target.encoded.blocks.predicates.clear();
    target.encoded.blocks.reads.clear();
    target.structures.entries.push(syn::parse_quote!(
        r repr = positions { R lineage { x -> a, y -> b } } owned [];
    ));
    assert!(ir::contract::check(&target).is_err());
}

#[test]
fn blocks_base_group_keys_and_aggregates_reject_before_printing() {
    for (group, head) in [
        (quote!(group valuations by [x] { n: usize = count(); }), quote!(answer(x,n))),
        (quote!(group valuations by [] { n: u64 = sum(y); }), quote!(answer(n))),
    ] {
        let source: physical::Program = syn::parse2(quote! { physical {
            relation R(a: Alias [17], b: Alias [17]);
            r = pipe rename R@0 {a -> x, b -> y};
            valuations = let canon project r keep {x,y};
            grouped = pipe #group;
            return dedup grouped as #head;
        }}).unwrap();
        cq_structure_selection::compile(&source).unwrap();
        assert!(cq_structure_selection::compile_with(&source, block_advice()).is_err());
    }
}

#[test]
fn blocks_groups_accept_owned_computed_operands() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a: Alias [17], b: Alias [17]);
        r = pipe rename R@0 {a -> x, b -> y};
        v = pipe bind r with value: u64 = (compute(x));
        valuations = let canon project v keep {x,y,value};
        grouped = pipe group valuations by [] { total: u64 = sum(value); };
        return grouped as answer(total);
    }};
    let selected = cq_structure_selection::compile_with(&source, block_advice()).unwrap();
    cq_structure_selection::check(&source, &selected).unwrap();
    assert_eq!(selected.encoded.blocks.reads["v"].fields.len(), 1);
    assert!(!selected.encoded.blocks.reads.contains_key("grouped"));
}

#[test]
fn blocks_restricted_source_literals_reject_before_printing() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a: Alias [17], b: Alias [17]);
        r = let canon rename R@0 {a -> x} constants {b = 7};
        return r as answer(x);
    }};
    cq_structure_selection::compile(&source).unwrap();
    assert!(cq_structure_selection::compile_with(&source, block_advice()).is_err());
}

#[test]
fn blocks_host_set_requires_an_already_discharged_premise() {
    let mut source = block_source();
    source.program.host_sets.push(syn::parse_quote!(host_set R;));
    cq_structure_selection::compile(&source).unwrap();
    assert!(cq_structure_selection::compile_with(&source, block_advice()).is_err());
    source.program.host_sets[0] = syn::parse_quote!(host_set R discharged;);
    let selected = cq_structure_selection::compile_with(&source, block_advice()).unwrap();
    cq_structure_selection::check(&source, &selected).unwrap();
}

#[test]
fn blocks_external_consumer_preserves_witness_values_and_owned_computations() {
    let source: physical::Program = syn::parse_quote! { physical {
        relation R(a: Alias [17], b: Alias [17]);
        capture { let group: u64 = 0; }
        r = pipe rename R@0 {a -> x, b -> y};
        v = pipe bind r with value: u64 = (compute(y));
        out = let canon project v keep {x,y,value};
        return out as rows(group,x,y,value);
    }};
    let demand = ir::OutputConsumer {
        original_consumer: "aggregate".into(),
        head: ["group", "x", "y", "value"].map(String::from).into(),
        value_reads: ["group", "value"].map(String::from).into(),
    };
    let selected = cq_structure_selection::compile_with_consumer(&source, block_advice(), Some(&demand)).unwrap();
    assert_eq!(selected.physical, source);
    assert!(selected.physical.program.host_sets.is_empty());
    assert!(selected.physical.program.consumer_canonical.is_none());
    let reads = &selected.encoded.blocks.reads;
    assert!(!reads.contains_key("rows"));
    let witnesses = &reads["output:rows:carrier"];
    assert_eq!(witnesses.purpose, ir::encoded::ReadPurpose::PreserveCarrier);
    assert_eq!(witnesses.consumer, ir::encoded::Consumer::Relational("aggregate".into()));
    assert_eq!(witnesses.fields.keys().map(String::as_str).collect::<Vec<_>>(), ["x", "y"]);
    assert_eq!(witnesses.fields["x"].action, ir::blocks::ReadAction::Ordinal);
    let logical = &reads["output:rows:read"];
    assert_eq!(logical.purpose, ir::encoded::ReadPurpose::OpaqueRustRead);
    assert_eq!(logical.consumer, ir::encoded::Consumer::OpaqueRust("aggregate".into()));
    assert!(logical.fields.is_empty());
    assert_eq!(reads["v"].fields.keys().map(String::as_str).collect::<Vec<_>>(), ["y"]);
    assert_eq!(syn::parse2::<ir::Program>(selected.to_token_stream()).unwrap(), selected);

    // Reading a base key requires a borrowing contract, not an ordinal
    // substitution or an owned temporary passed to a borrowed-value consumer.
    let mut reads_key = demand.clone();
    reads_key.value_reads.push("x".into());
    assert!(cq_structure_selection::compile_with_consumer(&source, block_advice(), Some(&reads_key)).is_err());
    let mut forged = selected.clone();
    forged.consumer = Some(reads_key);
    assert!(ir::contract::check(&forged).is_err());
}
