use cq_lang_sequence::{CanonToken, Capability, Catalog, CodecToken, LevelNode, Machine, Program, ProviderToken, Term};
use cq_sequence_plan::{Advice, KeyPartitionPolicy, cost, decompose, decompose_with_key_partitions};
use cq_sequence_rewrite_passes::key_partition;
use quote::{ToTokens, quote};

fn machine(parts: Option<usize>) -> Machine {
    Machine { key_partition_threshold: 65536, name: syn::parse_quote!(tasks), layout: None, items: 1,
    root: LevelNode {
        name: syn::parse_quote!(host), collaborative: false, dynamic: false,
        width: None, shared_bytes: None,
        children: vec![LevelNode {
            name: syn::parse_quote!(pool), collaborative: false, dynamic: true,
            width: parts, shared_bytes: None,
            children: vec![LevelNode {
                name: syn::parse_quote!(task), collaborative: false, dynamic: false,
                width: None, shared_bytes: None, children: vec![],
            }],
        }],
    }, }
}

fn catalog() -> Catalog {
    let levels = vec![vec!["host".into()], vec!["host".into(), "pool".into()], vec!["host".into(), "pool".into(), "task".into()]];
    Catalog {
        providers: vec![
            ProviderToken { ty: syn::parse_quote!(Index), elements: None, capabilities: vec![Capability::Exact], levels: levels.clone() },
            ProviderToken { ty: syn::parse_quote!(::cq_storage::sharded::Sharded<Index>), elements: None, capabilities: vec![Capability::Exact], levels: levels.clone() },
        ],
        codecs: vec![CodecToken { ty: syn::parse_quote!(::cq_storage::sequence::Identity), ordered: true, levels: levels.clone() }],
        canons: vec![CanonToken { ty: syn::parse_quote!(Canonical), levels, max_packed_bits: None }],
    }
}

// TPC-H Q1-shaped keyed aggregates retain line identity in the complete
// valuation, then canonicalize and aggregate within key-disjoint partitions.
fn grouped(global: bool) -> Program {
    let keys = if global { quote!() } else { quote!(returnflag, linestatus) };
    let head = if global { quote!() } else { quote!(returnflag, linestatus,) };
    syn::parse_quote! {
        physical {
            relation Lineitem(orderkey:i64, linenumber:i32, returnflag:i32, linestatus:i32, quantity:i64, extendedprice:i64);
            lineitem = pipe canon rename Lineitem@0 {
                orderkey -> orderkey, linenumber -> linenumber,
                returnflag -> returnflag, linestatus -> linestatus,
                quantity -> quantity, extendedprice -> extendedprice
            };
            totals = pipe group lineitem by [#keys] {
                sum_qty: i64 = sum(quantity);
                sum_base_price: i64 = sum(extendedprice);
                count_order: usize = count();
            };
            return totals as answer(#head sum_qty, sum_base_price, count_order);
        }
        structures { lineitem host scan = Reader []; lineitem canon = Canonical; }
        sequence {
            input = store rows lineitem;
            result = store group totals canon input;
        }
    }
}

fn named_grouped(global: bool) -> Program {
    let mut source = grouped(global);
    source.sequence.definitions.insert(1, cq_lang_sequence::Definition {
        name: syn::parse_quote!(distinct),
        term: syn::parse_quote!(canon input),
    });
    source.sequence.definitions.insert(2, cq_lang_sequence::Definition {
        name: syn::parse_quote!(alias),
        term: syn::parse_quote!(distinct),
    });
    source.sequence.definitions.last_mut().unwrap().term = syn::parse_quote!(store group totals alias);
    source
}

fn policy(name: &str, select: bool) -> KeyPartitionPolicy {
    KeyPartitionPolicy { name: syn::parse_str(name).unwrap(), select }
}

fn replay(source: &Program, planned: &cq_sequence_plan::Planned) {
    let mut replayed = source.clone();
    for advice in &planned.trace {
        let parsed: Advice = syn::parse2(advice.to_token_stream()).unwrap();
        assert_eq!(*advice, parsed);
        replayed = parsed.apply(&replayed).unwrap();
    }
    assert_eq!(replayed, planned.program);
}

#[test]
fn grouped_tpch_plan_prints_fixed_partition_and_replays() {
    let source = grouped(false);
    let machine = machine(Some(8));
    let planned = decompose(&source, &machine, &catalog()).unwrap();
    let printed = planned.program.to_token_stream().to_string();
    println!("Grouped TPC-H Q1-shaped Sequence plan:\n{printed}");
    assert!(printed.contains("partition_k [returnflag , linestatus] (8)"), "{printed}");
    assert!(printed.contains("flatten group totals canon partition_k"), "{printed}");
    assert!(printed.contains("__cq_key_input_result = store input"), "{printed}");
    assert_eq!(planned.program, syn::parse2(planned.program.to_token_stream()).unwrap());
    cq_lang_sequence::check_grading(&planned.program, &machine).unwrap();
    replay(&source, &planned);
}

#[test]
fn cost_selects_streamed_collectives_and_advice_can_force_or_forbid() {
    let source = grouped(false);
    let machine = machine(Some(8));
    let forced = decompose_with_key_partitions(&source, &machine, &catalog(), &[policy("result", true)]).unwrap();
    let forbidden = decompose_with_key_partitions(&source, &machine, &catalog(), &[policy("result", false)]).unwrap();
    assert!(forced.trace.iter().any(|a| matches!(a, Advice::KeyPartition(_))));
    assert!(!forbidden.trace.iter().any(|a| matches!(a, Advice::KeyPartition(_))));
    assert!(cost(&forced.program, &machine).unwrap().better_than(&cost(&forbidden.program, &machine).unwrap()));
    let automatic = decompose(&source, &machine, &catalog()).unwrap();
    assert!(automatic.trace.iter().any(|a| matches!(a, Advice::KeyPartition(_))));
    replay(&source, &forced);
    replay(&source, &forbidden);
}

#[test]
fn known_small_collectives_pay_setup_and_large_collectives_amortize_it() {
    for global in [false, true] {
        for rows in [0usize, 1, 4, 4096] {
            let mut source = grouped(global);
            source.evidence = syn::parse2(quote!(evidence { count Lineitem = #rows; })).unwrap();
            let planned = decompose(&source, &machine(Some(8)), &catalog()).unwrap();
            assert_eq!(
                planned.trace.iter().any(|a| matches!(a, Advice::KeyPartition(_))),
                rows == 4096,
                "global={global}, rows={rows}: {}", planned.program.to_token_stream(),
            );
            replay(&source, &planned);
        }
    }
}

#[test]
fn grouped_named_canon_moves_inside_routed_parts_and_replays() {
    for global in [false, true] {
        let source = named_grouped(global);
        let machine = machine(Some(8));
        let planned = decompose(&source, &machine, &catalog()).unwrap();
        assert!(planned.trace.iter().any(|a| matches!(a, Advice::KeyPartition(k) if k.name == "result")));
        let distinct = planned.program.sequence.definitions.iter().find(|d| d.name == "distinct").unwrap();
        assert!(matches!(&distinct.term, Term::Ref(name) if name == "input"),
            "named Canon must not execute before routing: {}", planned.program.to_token_stream());
        assert!(planned.program.to_token_stream().to_string().contains("group totals canon partition_k"));
        replay(&source, &planned);
    }
}

#[test]
fn group_partition_preserves_shared_and_stored_canonical_producers() {
    for stored in [false, true] {
        let mut source = named_grouped(false);
        if stored {
            source.sequence.definitions[1].term = syn::parse_quote!(store canon input);
        } else {
            source.sequence.definitions.insert(3, cq_lang_sequence::Definition {
                name: syn::parse_quote!(other_consumer),
                term: syn::parse_quote!(store distinct),
            });
        }
        let original = source.sequence.definitions[1].term.clone();
        let target = key_partition(&source, quote!(result, staged, [returnflag, linestatus], 8)).unwrap();
        assert_eq!(target.sequence.definitions[1].term, original);
        assert!(!target.to_token_stream().to_string().contains("group totals canon partition_k"));
    }
}

#[test]
fn force_rejects_missing_width_and_unknown_advice() {
    for (source, machine, policies) in [
        (grouped(false), machine(None), vec![policy("result", true)]),
        (grouped(false), machine(Some(8)), vec![policy("missing", false)]),
        (grouped(false), machine(Some(8)), vec![policy("result", true), policy("result", false)]),
    ] {
        assert!(decompose_with_key_partitions(&source, &machine, &catalog(), &policies).is_err());
    }
}

#[test]
fn checked_advice_rejects_invalid_keys_counts_and_collisions() {
    let source = grouped(false);
    for arguments in [
        quote!(result, staged, [returnflag], 0),
        quote!(result, staged, [quantity], 8),
        quote!(result, input, [returnflag], 8),
        quote!(result, staged, [missing], 8),
        quote!(result, staged, [], 8),
        quote!(missing, staged, [returnflag], 8),
        quote!(input, staged, [returnflag], 8),
    ] {
        assert!(key_partition(&source, arguments.clone()).is_err(), "{arguments}");
    }
}

#[test]
fn canon_partition_and_machine_width_are_explicit() {
    let source: Program = syn::parse_quote! {
        physical {
            relation R(a:i32,b:i32);
            r = pipe canon rename R@0 {a -> a,b -> b};
            return r as answer(a,b);
        }
        structures { r host scan = Reader []; r canon = Canonical; }
        sequence { input = store rows r; result = store canon input; }
    };
    for parts in [1, 3, 16] {
        let planned = decompose_with_key_partitions(&source, &machine(Some(parts)), &catalog(), &[policy("result", true)]).unwrap();
        assert!(planned.trace.iter().any(|a| matches!(a, Advice::KeyPartition(k) if k.parts == parts && k.keys.len() == 2)));
        replay(&source, &planned);
        let unpartitioned = decompose_with_key_partitions(
            &source, &machine(Some(parts)), &catalog(), &[policy("result", false)],
        ).unwrap();
        let routed_cost = cost(&planned.program, &machine(Some(parts))).unwrap();
        let global_cost = cost(&unpartitioned.program, &machine(Some(parts))).unwrap();
        assert_eq!(
            routed_cost.better_than(&global_cost),
            parts >= 4,
            "flattened tiles still require global Canon: routed={routed_cost}; global={global_cost}",
        );
        let automatic = decompose(&source, &machine(Some(parts)), &catalog()).unwrap();
        assert_eq!(
            automatic.trace.iter().any(|a| matches!(a, Advice::KeyPartition(_))),
            parts >= 4,
            "routing must consume bandwidth: parts={parts}",
        );
        replay(&source, &automatic);
    }
}

fn provider_program() -> Program {
    syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            joined = pipe canon probe first [a] l,r;
            return joined as answer(a,b);
        }
        structures { l host scan = Reader []; r host scan = Reader []; r = Index [a]; joined canon = Canonical; }
        sequence {
            provider = store rows r;
            input = store rows l;
            result = store canon flatten lookup r [a] input;
        }
    }
}

#[test]
fn provider_store_retains_parts_and_records_routed_provider_type() {
    let source = provider_program();
    let planned = decompose_with_key_partitions(&source, &machine(Some(4)), &catalog(), &[policy("provider", true)]).unwrap();
    let provider = planned.program.sequence.definitions.iter().find(|d| d.name == "provider").unwrap();
    assert!(matches!(&provider.term, Term::Store { inner, .. } if matches!(inner.as_ref(), Term::PartitionK { parts: 4, .. })));
    assert!(planned.program.to_token_stream().to_string().contains("Sharded < Index >"));
    replay(&source, &planned);
    assert!(key_partition(&source, quote!(provider, staged, [b], 4)).is_err());
    let automatic = decompose(&source, &machine(Some(4)), &catalog()).unwrap();
    assert!(automatic.trace.iter().any(|a| matches!(a, Advice::KeyPartition(k)
        if k.name == "provider" && k.threshold == Some(65536))));
}

#[test]
fn provider_choice_does_not_use_cardinality_evidence() {
    for rows in [0usize, 1, 4, 4096, 65535, 65536, 65537] {
        let mut source = provider_program();
        source.evidence = syn::parse2(quote!(evidence { count R = #rows; count L = 1; })).unwrap();
        let machine = machine(Some(4));
        let automatic = decompose(&source, &machine, &catalog()).unwrap();
        assert!(automatic.trace.iter().any(|a| matches!(a, Advice::KeyPartition(k)
            if k.name == "provider" && k.threshold == Some(machine.key_partition_threshold))));
        assert!(automatic.program.to_token_stream().to_string().contains("Sharded < Index >"));
        replay(&source, &automatic);
    }
}

#[test]
fn force_does_not_override_an_authored_grade() {
    let mut source = grouped(false);
    source.graded = Some(syn::parse_quote!(graded tasks { result at host; }));
    assert!(decompose_with_key_partitions(&source, &machine(Some(8)), &catalog(), &[policy("result", true)]).is_err());
}

#[test]
fn force_and_forbid_policies_roundtrip() {
    for source in ["result, select true", "result, select false"] {
        let policy: KeyPartitionPolicy = syn::parse_str(source).unwrap();
        assert_eq!(policy, syn::parse2(policy.to_token_stream()).unwrap());
    }
    assert!(syn::parse_str::<KeyPartitionPolicy>("result, force true").is_err());
}

#[test]
fn global_groups_route_full_valuations_and_merge_to_one_row() {
    let source = grouped(true);
    let machine = machine(Some(8));
    let forced = decompose_with_key_partitions(&source, &machine, &catalog(), &[policy("result", true)]).unwrap();
    let forbidden = decompose_with_key_partitions(&source, &machine, &catalog(), &[policy("result", false)]).unwrap();
    let definition = forced.program.sequence.definitions.iter().find(|d| d.name == "result").unwrap();
    assert!(matches!(&definition.term, Term::Store { inner, .. } if matches!(inner.as_ref(), Term::Group(_, _))));
    assert!(forced.trace.iter().any(|advice| matches!(advice, Advice::KeyPartition(k) if k.keys.len() == 6)));
    let facts = cq_lang_sequence::check_grading(&forced.program, &machine).unwrap();
    assert_eq!(facts.shapes["result"], vec![cq_lang_sequence::Len::Lit(1)]);
    assert_eq!(facts.lengths["result"], cq_lang_sequence::Len::Lit(1));
    assert!(cost(&forced.program, &machine).unwrap().better_than(&cost(&forbidden.program, &machine).unwrap()));
    assert!(decompose(&source, &machine, &catalog()).unwrap().trace.iter().any(|a| matches!(a, Advice::KeyPartition(_))));
    assert!(!forbidden.trace.iter().any(|a| matches!(a, Advice::KeyPartition(_))));
    println!("Global TPC-H-shaped aggregate plan:\n{}", forced.program.to_token_stream());
    replay(&source, &forced);
}

#[test]
fn globally_unique_aggregates_use_tiles_without_canonicalizers() {
    let mut source = grouped(true);
    source.evidence = syn::parse_quote!(evidence { key Lineitem [orderkey, linenumber]; });
    let machine = machine(Some(8));
    let planned = decompose_with_key_partitions(&source, &machine, &catalog(), &[policy("result", true)]).unwrap();
    let result = planned.program.sequence.definitions.iter().find(|d| d.name == "result").unwrap();
    assert!(matches!(&result.term, Term::Store { inner, .. }
        if matches!(inner.as_ref(), Term::Group(_, input)
            if matches!(input.as_ref(), Term::Tile { n: 8, .. }))));
    assert!(planned.trace.iter().any(|a| matches!(a, Advice::KeyPartition(k) if k.keys.is_empty())));
    let facts = cq_lang_sequence::check_grading(&planned.program, &machine).unwrap();
    assert_eq!(facts.shapes["result"], vec![cq_lang_sequence::Len::Lit(1)]);
    let forbidden = decompose_with_key_partitions(&source, &machine, &catalog(), &[policy("result", false)]).unwrap();
    assert!(cost(&planned.program, &machine).unwrap().better_than(&cost(&forbidden.program, &machine).unwrap()));
    replay(&source, &planned);
}

#[test]
fn global_partial_advice_roundtrips_and_rejects_zero_tile_width() {
    let advice: Advice = syn::parse_str("key_partition(result, staged, [], 8)").unwrap();
    assert_eq!(advice, syn::parse2(advice.to_token_stream()).unwrap());
    assert!(key_partition(&grouped(true), quote!(result, staged, [], 0)).is_err());
}

#[test]
fn empty_global_partial_plans_keep_the_singleton_identity_shape() {
    for unique in [false, true] {
        let mut source = grouped(true);
        source.evidence = if unique {
            syn::parse_quote!(evidence { count Lineitem = 0; key Lineitem [orderkey, linenumber]; })
        } else {
            syn::parse_quote!(evidence { count Lineitem = 0; })
        };
        let machine = machine(Some(8));
        let planned = decompose_with_key_partitions(&source, &machine, &catalog(), &[policy("result", true)]).unwrap();
        let facts = cq_lang_sequence::check_grading(&planned.program, &machine).unwrap();
        assert_eq!(facts.shapes["result"], vec![cq_lang_sequence::Len::Lit(1)]);
        assert_eq!(facts.lengths["result"], cq_lang_sequence::Len::Lit(1));
        replay(&source, &planned);
    }
}

#[test]
fn unknown_provider_uses_published_runtime_count_and_checked_identity_cover() {
    use cq_lang_sequence::{Guard, Len};
    let source = provider_program();
    let mut machine = machine(Some(4));
    machine.key_partition_threshold = 17;
    let planned = decompose(&source, &machine, &catalog()).unwrap();
    let advice = planned.trace.iter().find_map(|advice| match advice {
        Advice::KeyPartition(advice) if advice.name == "provider" => Some(advice),
        _ => None,
    }).expect("unknown-cardinality provider must retain runtime advice");
    assert_eq!(advice.threshold, Some(17));
    let provider = planned.program.sequence.definitions.iter().find(|d| d.name == "provider").unwrap();
    let Term::Store { inner, .. } = &provider.term else { panic!("provider Store required") };
    let Term::Choose(arms) = inner.as_ref() else { panic!("runtime Choose required") };
    assert_eq!(arms.len(), 2);
    assert_eq!(arms[0].0, Guard::Lt(Len::Input(advice.input.to_string()), 17));
    assert_eq!(arms[1].0, Guard::Else);
    for (arm, parts) in arms.iter().zip([1, 4]) {
        assert_eq!(arm.1, Term::PartitionK {
            keys: advice.keys.clone(), parts,
            inner: Box::new(Term::Ref(advice.input.clone())),
        });
    }
    // Interpret the written first-match guards, including the exact boundary.
    for (rows, expected_parts) in [(0, 1), (16, 1), (17, 4), (18, 4)] {
        let selected = arms.iter().find(|(guard, _)| match guard {
            Guard::Lt(Len::Input(name), threshold) => {
                assert_eq!(name, &advice.input.to_string());
                rows < *threshold
            }
            Guard::Else => true,
            _ => panic!("unexpected guard"),
        }).unwrap();
        assert!(matches!(&selected.1, Term::PartitionK { parts, .. } if *parts == expected_parts));
    }
    let facts = cq_lang_sequence::check_grading(&planned.program, &machine).unwrap();
    assert_eq!(facts.shapes["provider"][0], Len::Unknown(None));
    assert!(planned.program.graded.as_ref().unwrap().grades.iter().any(|grade|
        grade.name == advice.input && grade.level.as_slice() == ["host"]));
    replay(&source, &planned);
    assert_eq!(planned.program, syn::parse2(planned.program.to_token_stream()).unwrap());
    assert!(cq_lang_sequence::check_grading(&planned.program, &self::machine(Some(8))).is_err());
}

#[test]
fn provider_force_forbid_and_authored_grade_override_runtime_choice() {
    let source = provider_program();
    for select in [true, false] {
        let planned = decompose_with_key_partitions(
            &source, &machine(Some(4)), &catalog(), &[policy("provider", select)],
        ).unwrap();
        let provider = planned.program.sequence.definitions.iter().find(|d| d.name == "provider").unwrap();
        let Term::Store { inner, .. } = &provider.term else { panic!("provider Store required") };
        assert!(!matches!(inner.as_ref(), Term::Choose(_)));
        if select {
            assert!(matches!(inner.as_ref(), Term::PartitionK { parts: 4, .. }));
            assert!(planned.trace.iter().any(|a| matches!(a, Advice::KeyPartition(k)
                if k.name == "provider" && k.threshold.is_none())));
        } else {
            assert!(!planned.trace.iter().any(|a| matches!(a, Advice::KeyPartition(k) if k.name == "provider")));
            assert!(!planned.program.to_token_stream().to_string().contains("Sharded < Index >"));
        }
        replay(&source, &planned);
    }
    let mut authored = source.clone();
    authored.graded = Some(syn::parse_quote!(graded tasks { provider at host; }));
    let planned = decompose(&authored, &machine(Some(4)), &catalog()).unwrap();
    assert!(!planned.trace.iter().any(|a| matches!(a, Advice::KeyPartition(k) if k.name == "provider")));
}

#[test]
fn adaptive_provider_threshold_is_positive_and_provider_only() {
    let source = provider_program();
    let mut invalid = machine(Some(4));
    invalid.key_partition_threshold = 0;
    assert!(decompose(&source, &invalid, &catalog()).is_err());
    assert!(key_partition(&source, quote!(provider, published, [a], 4, threshold 0)).is_err());
    assert!(key_partition(&source, quote!(provider, published, [a], 0, threshold 17)).is_err());
    assert!(key_partition(&grouped(false),
        quote!(result, published, [returnflag, linestatus], 4, threshold 17)).is_err());
    assert!(syn::parse_str::<Advice>("key_partition(provider, published, [a], 4, cutoff 17)").is_err());
}

#[test]
fn exact_membership_provider_supports_runtime_partition_choice() {
    let source: Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a};
            j = pipe canon semi eager [a] l,r;
            return j as answer(a);
        }
        structures {
            l host scan = Reader []; r host scan = Reader [];
            r = Index [a]; j canon = Canonical;
        }
        sequence {
            provider = store rows r;
            input = store rows l;
            result = store canon compact j lookup r [a] input;
        }
    };
    let mut membership_catalog = catalog();
    for provider in &mut membership_catalog.providers {
        provider.capabilities = vec![Capability::Membership];
    }
    let planned = decompose(&source, &machine(Some(4)), &membership_catalog).unwrap();
    assert!(planned.trace.iter().any(|a| matches!(a, Advice::KeyPartition(k)
        if k.name == "provider" && k.threshold == Some(65536))));
    cq_lang_sequence::check_grading(&planned.program, &machine(Some(4))).unwrap();
    replay(&source, &planned);
}

#[test]
fn scheduled_range_obligation_keeps_native_provider() {
    let mut source = provider_program();
    source.sequence.definitions.last_mut().unwrap().term =
        syn::parse_quote!(store tile 2 canon flatten lookup r [a] input);
    let mut catalog = catalog();
    // Native cursors support the authored distributed lookup. The routed
    // wrapper deliberately remains exact-only: hash routing grants no Range.
    catalog.providers[0].capabilities.push(Capability::Range);
    let machine = machine(Some(4));
    let planned = decompose(&source, &machine, &catalog).unwrap();
    let facts = cq_lang_sequence::check_grading(&planned.program, &machine).unwrap();
    assert_eq!(facts.capabilities[&("r".into(), vec!["a".into()])], Capability::Range);
    assert!(!planned.trace.iter().any(|a| matches!(a, Advice::KeyPartition(k)
        if k.name == "provider")));
    assert!(!planned.program.to_token_stream().to_string().contains("Sharded < Index >"));
    replay(&source, &planned);
    assert!(decompose_with_key_partitions(
        &source, &machine, &catalog, &[policy("provider", true)],
    ).is_err());
}

#[test]
fn provider_routing_requires_flat_carried_valuations() {
    let mut source = provider_program();
    source.sequence.definitions[0].term = syn::parse_quote!(store tile 2 rows r);
    let machine = machine(Some(4));
    let planned = decompose(&source, &machine, &catalog()).unwrap();
    assert!(!planned.trace.iter().any(|a| matches!(a, Advice::KeyPartition(k)
        if k.name == "provider")));
    replay(&source, &planned);
    assert!(decompose_with_key_partitions(
        &source, &machine, &catalog(), &[policy("provider", true)],
    ).is_err());
}

#[test]
fn routed_catalog_requires_capability_elements_and_scheduled_read_levels() {
    let mut source = provider_program();
    source.graded = Some(syn::parse_quote!(graded tasks { result at host; }));
    for restriction in 0..3 {
        let mut catalog = catalog();
        let routed = &mut catalog.providers[1];
        match restriction {
            0 => routed.capabilities = vec![Capability::Membership],
            1 => routed.elements = Some(vec![syn::parse_quote!(String)]),
            _ => routed.levels = vec![vec!["host".into(), "pool".into(), "task".into()]],
        }
        let planned = decompose(&source, &machine(Some(4)), &catalog).unwrap();
        assert!(!planned.trace.iter().any(|a| matches!(a, Advice::KeyPartition(k)
            if k.name == "provider")), "restriction {restriction}");
        replay(&source, &planned);
    }
}

#[test]
fn task_flatten_decomposition_does_not_introduce_unnamed_separated_stores() {
    let mut source = provider_program();
    source.sequence.definitions.insert(2, syn::parse_quote!(
        expanded = flatten lookup r [a] input;
    ));
    source.sequence.definitions.last_mut().unwrap().term = syn::parse_quote!(store canon expanded);
    let mut catalog = catalog();
    catalog.providers[0].capabilities.push(Capability::Range);
    let machine = machine(Some(4));
    let planned = decompose_with_key_partitions(
        &source, &machine, &catalog, &[policy("result", false)],
    ).unwrap();
    assert!(!planned.trace.iter().any(|a| matches!(a, Advice::Segment { .. })));
    cq_lang_sequence::check_grading(&planned.program, &machine).unwrap();
    replay(&source, &planned);
}

#[test]
fn provider_rewrite_preserves_shared_flat_rows_and_published_length_guards() {
    let source: Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            selected = let project r keep {a};
            joined = pipe probe first [a] l,r;
            final_rows = pipe canon semi eager [a] joined,selected;
            return final_rows as answer(a,b);
        }
        structures {
            l host scan = Reader []; r host scan = Reader [];
            r = Index [a]; selected = Index [a]; final_rows canon = Canonical;
            r rows = ::std::vec::Vec<_>;
        }
        sequence {
            provider = store rows r;
            derived = store map selected provider;
            same_rows = choose [len provider < 17 => provider, else => provider];
            input = store rows l;
            expanded = flatten lookup r [a] input;
            result = store canon compact final_rows lookup selected [a] expanded;
        }
    };
    let routed = key_partition(&source,
        quote!(provider, published, [a], 4, threshold 17)).unwrap();
    let derived = routed.sequence.definitions.iter().find(|d| d.name == "derived").unwrap();
    assert_eq!(derived.term, syn::parse_quote!(store map selected published));
    let same_rows = routed.sequence.definitions.iter().find(|d| d.name == "same_rows").unwrap();
    assert_eq!(same_rows.term,
        syn::parse_quote!(choose [len published < 17 => published, else => published]));
    // A second provider can now publish the shared rows without accidentally
    // receiving the first provider's new nested cover level.
    let twice = key_partition(&routed,
        quote!(derived, derived_input, [a], 4, threshold 17)).unwrap();
    let input = twice.sequence.definitions.iter().find(|d| d.name == "derived_input").unwrap();
    assert_eq!(input.term, syn::parse_quote!(store map selected published));
    let facts = cq_lang_sequence::contract::check(&twice).unwrap();
    assert_eq!(facts.shapes["derived_input"].len(), 1);
    assert_eq!(facts.shapes["same_rows"].len(), 1);
    assert_eq!(twice, syn::parse2(twice.to_token_stream()).unwrap());

    let mut catalog = catalog();
    for token in &mut catalog.providers {
        token.capabilities.push(Capability::Membership);
    }
    let machine = machine(Some(4));
    let planned = decompose(&source, &machine, &catalog).unwrap();
    assert!(planned.trace.iter().any(|a| matches!(a, Advice::KeyPartition(k)
        if k.name == "derived")));
    cq_lang_sequence::check_grading(&planned.program, &machine).unwrap();
    replay(&source, &planned);
}

#[test]
fn physical_provider_is_partitioned_once_across_publication_aliases() {
    let mut source = provider_program();
    source.sequence.definitions.insert(1, syn::parse_quote!(
        provider_alias = store provider;
    ));
    let machine = machine(Some(4));
    for policies in [
        vec![],
        vec![policy("provider", true)],
        vec![policy("provider_alias", true)],
    ] {
        let planned = decompose_with_key_partitions(&source, &machine, &catalog(), &policies).unwrap();
        let routed: Vec<_> = planned.trace.iter().filter_map(|advice| match advice {
            Advice::KeyPartition(advice)
                if advice.name == "provider" || advice.name == "provider_alias" => Some(advice),
            _ => None,
        }).collect();
        assert_eq!(routed.len(), 1);
        assert_eq!(routed[0].name, policies.first().map_or(
            syn::parse_quote!(provider_alias), |policy| policy.name.clone(),
        ));
        assert_eq!(planned.program.to_token_stream().to_string().matches("Sharded").count(), 1);
        cq_lang_sequence::check_grading(&planned.program, &machine).unwrap();
        replay(&source, &planned);
    }
    for forbidden in ["provider", "provider_alias"] {
        let planned = decompose_with_key_partitions(
            &source, &machine, &catalog(), &[policy(forbidden, false)],
        ).unwrap();
        assert!(!planned.program.to_token_stream().to_string().contains("Sharded"));
        replay(&source, &planned);
    }
    assert!(decompose_with_key_partitions(&source, &machine, &catalog(),
        &[policy("provider", true), policy("provider_alias", true)]).is_err());
    assert!(decompose_with_key_partitions(&source, &machine, &catalog(),
        &[policy("provider", true), policy("provider_alias", false)]).is_err());

    let once = key_partition(&source,
        quote!(provider, raw_input, [a], 4, threshold 17)).unwrap();
    let error = key_partition(&once,
        quote!(provider_alias, second_input, [a], 4, threshold 17)).unwrap_err();
    assert!(error.to_string().contains("physical provider is already routed"));
}

#[test]
fn selected_provider_publication_precedes_every_keyed_consumer() {
    let mut source = provider_program();
    source.sequence.definitions.insert(2, syn::parse_quote!(
        early_read = store flatten lookup r [a] input;
    ));
    source.sequence.definitions.insert(3, syn::parse_quote!(
        late_alias = store provider;
    ));
    let machine = machine(Some(4));
    let planned = decompose(&source, &machine, &catalog()).unwrap();
    let selected: Vec<_> = planned.trace.iter().filter_map(|advice| match advice {
        Advice::KeyPartition(advice) if advice.name == "provider" || advice.name == "late_alias" => Some(advice),
        _ => None,
    }).collect();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].name, "provider");
    replay(&source, &planned);
    let error = decompose_with_key_partitions(
        &source, &machine, &catalog(), &[policy("late_alias", true)],
    ).unwrap_err();
    assert!(error.into_iter().any(|error| error.to_string().contains("must precede every keyed read")));
    let error = key_partition(&source,
        quote!(late_alias, late_input, [a], 4, threshold 17)).unwrap_err();
    assert!(error.to_string().contains("must precede every keyed read"));
}

#[test]
fn unique_keyed_groups_stream_tiles_through_single_use_valuation_spines() {
    let mut source = grouped(false);
    source.structures.physical.program.definitions.insert(1, syn::parse_quote!(
        selected = pipe select lineitem if (quantity > 0);
    ));
    let totals = source.structures.physical.program.definitions.iter_mut()
        .find(|definition| definition.name == "totals").unwrap();
    let cq_lang_sequence::structures::physical::Operator::Group(group) = &mut totals.operator
        else { panic!("group fixture") };
    group.input = syn::parse_quote!(selected);
    source.evidence = syn::parse_quote!(evidence { key Lineitem [orderkey, linenumber]; });
    source.sequence.definitions = vec![
        syn::parse_quote!(input = rows lineitem;),
        syn::parse_quote!(filtered = compact selected input;),
        syn::parse_quote!(valuations = map selected filtered;),
        syn::parse_quote!(result = store group totals valuations;),
    ];
    let rewritten = key_partition(&source, quote!(result, unused, [], 8)).unwrap();
    assert_eq!(rewritten.sequence.definitions.len(), source.sequence.definitions.len());
    assert_eq!(rewritten.sequence.definitions[0].term, syn::parse_quote!(tile 8 rows lineitem));
    assert_eq!(rewritten.sequence.definitions[1].term, source.sequence.definitions[1].term);
    assert_eq!(rewritten.sequence.definitions[2].term, source.sequence.definitions[2].term);
    assert_eq!(cq_lang_sequence::contract::check(&rewritten).unwrap().shapes["result"].len(), 1);
    let planned = decompose(&source, &machine(Some(8)), &catalog()).unwrap();
    assert!(planned.trace.iter().any(|advice| matches!(advice, Advice::KeyPartition(k)
        if k.name == "result" && k.keys.is_empty())));
    assert!(!planned.program.sequence.definitions.iter()
        .any(|definition| definition.name.to_string().starts_with("__cq_key_input_result")));
    assert!(!planned.program.to_token_stream().to_string().contains("partition_k"));
    let facts = cq_lang_sequence::check_grading(&planned.program, &machine(Some(8))).unwrap();
    for view in ["input", "filtered", "valuations"] {
        assert_eq!(facts.levels[view], vec!["host".to_owned(), "pool".to_owned(), "task".to_owned()]);
    }
    replay(&source, &planned);
}

#[test]
fn streaming_partial_group_preserves_existing_stores_and_rejects_shared_pipes() {
    let mut source = grouped(false);
    source.evidence = syn::parse_quote!(evidence { key Lineitem [orderkey, linenumber]; });
    source.sequence.definitions.last_mut().unwrap().term = syn::parse_quote!(store group totals input);
    let rewritten = key_partition(&source, quote!(result, unused, [], 8)).unwrap();
    assert_eq!(rewritten.sequence.definitions[0], source.sequence.definitions[0]);
    assert_eq!(rewritten.sequence.definitions[1].term,
        syn::parse_quote!(store group totals tile 8 input));
    source.sequence.definitions[0].term = syn::parse_quote!(rows lineitem);
    source.sequence.definitions.insert(1, syn::parse_quote!(other = map lineitem input;));
    assert!(key_partition(&source, quote!(result, unused, [], 8)).is_err());
    assert!(key_partition(&grouped(false), quote!(result, unused, [], 8)).is_err());
}

#[test]
fn opaque_partial_group_publishes_host_owned_parts_without_totality_claims() {
    let mut source = grouped(false);
    source.structures.physical.program.definitions.insert(1, syn::parse_quote!(
        mapped = pipe bind lineitem with extra:i64 = (opaque_fn(*quantity));
    ));
    let totals = source.structures.physical.program.definitions.iter_mut()
        .find(|definition| definition.name == "totals").unwrap();
    let cq_lang_sequence::structures::physical::Operator::Group(group) = &mut totals.operator
        else { panic!("group fixture") };
    group.input = syn::parse_quote!(mapped);
    source.evidence = syn::parse_quote!(evidence { key Lineitem [orderkey, linenumber]; });
    source.sequence.definitions = vec![
        syn::parse_quote!(input = store rows lineitem;),
        syn::parse_quote!(valuations = map mapped input;),
        syn::parse_quote!(result = store group totals valuations;),
    ];
    let target = key_partition(&source, quote!(result, host_parts, [], 8)).unwrap();
    assert_eq!(target.sequence.definitions[1], source.sequence.definitions[1]);
    assert_eq!(target.sequence.definitions[2].term,
        syn::parse_quote!(store tile 8 valuations));
    assert_eq!(target.sequence.definitions[3].term,
        syn::parse_quote!(store group totals host_parts));
    assert_eq!(target.structures.physical, source.structures.physical);

    let machine = machine(Some(8));
    let planned = decompose_with_key_partitions(
        &source, &machine, &catalog(), &[policy("result", true)],
    ).unwrap();
    let partial = planned.trace.iter().find_map(|advice| match advice {
        Advice::KeyPartition(advice) if advice.name == "result" => Some(advice),
        _ => None,
    }).unwrap();
    assert!(partial.keys.is_empty());
    let facts = cq_lang_sequence::check_grading(&planned.program, &machine).unwrap();
    assert_eq!(facts.levels["valuations"], vec!["host".to_owned()]);
    assert_eq!(facts.levels[&partial.input.to_string()], vec!["host".to_owned()]);
    assert_eq!(facts.shapes[&partial.input.to_string()].len(), 2);
    assert_eq!(facts.shapes["result"].len(), 1);
    let result = planned.program.sequence.definitions.iter().find(|d| d.name == "result").unwrap();
    let input = &partial.input;
    assert_eq!(result.term, syn::parse_quote!(store group totals #input));
    replay(&source, &planned);
}

#[test]
fn keyed_group_cost_compares_partial_map_merge_with_key_routing() {
    let machine = machine(Some(8));
    for groups in [1usize, 4096] {
        let mut source = grouped(false);
        source.sequence.definitions.last_mut().unwrap().term = syn::parse_quote!(store group totals input);
        source.evidence = syn::parse2(quote!(evidence {
            key Lineitem [orderkey, linenumber];
            count Lineitem = 4096;
            count totals = #groups;
        })).unwrap();
        let mut partial = key_partition(&source, quote!(result, unused, [], 8)).unwrap();
        partial.graded = Some(syn::parse_quote!(graded tasks { input at host; result at host.pool; }));
        let mut routed = key_partition(&source,
            quote!(result, staged, [returnflag, linestatus], 8)).unwrap();
        routed.graded = Some(syn::parse_quote!(graded tasks {
            input at host; staged at host; result at host.pool;
        }));
        let partial_cost = cost(&partial, &machine).unwrap();
        let routed_cost = cost(&routed, &machine).unwrap();
        assert_eq!(partial_cost.better_than(&routed_cost), groups == 1,
            "groups={groups}: partial={partial_cost}, routed={routed_cost}");
        assert_eq!(routed_cost.better_than(&partial_cost), groups == 4096);
        let planned = decompose_with_key_partitions(
            &source, &machine, &catalog(), &[policy("result", true)],
        ).unwrap();
        let chosen = planned.trace.iter().find_map(|advice| match advice {
            Advice::KeyPartition(advice) if advice.name == "result" => Some(advice),
            _ => None,
        }).unwrap();
        assert_eq!(chosen.keys.is_empty(), groups == 1);
        replay(&source, &planned);
    }
}

#[test]
fn unknown_partial_map_merge_is_symbolic_not_free() {
    let mut source = grouped(false);
    source.evidence = syn::parse_quote!(evidence { key Lineitem [orderkey, linenumber]; });
    let planned = decompose(&source, &machine(Some(8)), &catalog()).unwrap();
    assert!(planned.trace.iter().any(|advice| matches!(advice, Advice::KeyPartition(k)
        if k.name == "result" && k.keys.is_empty())));
    let cost = cost(&planned.program, &machine(Some(8))).unwrap().to_string();
    assert_eq!(planned.program.sequence.output, Some(cq_lang_sequence::OutputProtocol::Reserve));
    assert!(cost.contains("8*scan(group(totals))"), "{cost}");
    let unproved = decompose(&grouped(false), &machine(Some(8)), &catalog()).unwrap();
    assert!(unproved.trace.iter().any(|advice| matches!(advice, Advice::KeyPartition(k)
        if k.name == "result" && !k.keys.is_empty())));
}

#[test]
fn adaptive_provider_publication_preserves_task_owned_pipe_execution() {
    // A row-dependent comparison places the native provider at the pool.
    // A constant bound is a host descriptor and would make the authored task
    // pipe illegal before adaptive routing is considered.
    let source: Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {a -> a,b -> b};
            selected = let select r if (b > a);
            joined = pipe canon probe first [a] l,selected;
            return joined as answer(a,b);
        }
        structures {
            l host scan = Reader []; r host scan = Reader [];
            selected = Index [a]; joined canon = Canonical;
        }
        sequence {
            raw = store rows r;
            task_pipe = map r raw;
            provider = store compact selected task_pipe;
            input = store rows l;
            result = store canon flatten lookup selected [a] input;
        }
        graded tasks { task_pipe at host.pool.task; }
    };
    let machine = machine(Some(4));
    for forced in [false, true] {
        let mut policies = vec![policy("result", false)];
        if forced { policies.push(policy("provider", true)); }
        let planned = decompose_with_key_partitions(&source, &machine, &catalog(), &policies).unwrap();
        let partition = planned.trace.iter().find_map(|advice| match advice {
            Advice::KeyPartition(advice) if advice.name == "provider" => Some(advice),
            _ => None,
        }).unwrap();
        let producer = planned.trace.iter().find_map(|advice| match advice {
            Advice::Publish { name, producer } if *name == partition.input => Some(producer),
            _ => None,
        }).expect("below-host raw producer needs a checked host publication");
        let raw_input = planned.program.sequence.definitions.iter()
            .find(|d| d.name == partition.input).unwrap();
        assert_eq!(raw_input.term, syn::parse_quote!(store #producer));
        let producer_definition = planned.program.sequence.definitions.iter()
            .find(|d| d.name == *producer).unwrap();
        assert!(!matches!(producer_definition.term, Term::Store { .. }));
        let facts = cq_lang_sequence::check_grading(&planned.program, &machine).unwrap();
        let task = vec!["host".to_owned(), "pool".to_owned(), "task".to_owned()];
        assert_eq!(facts.levels["task_pipe"], task);
        assert_eq!(facts.levels[&producer.to_string()], task);
        assert_eq!(facts.levels[&partition.input.to_string()], vec!["host".to_owned()]);
        replay(&source, &planned);
    }
}
