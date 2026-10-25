use cq_lang_sequence::{CanonToken, Capability, Catalog, CodecToken, LevelNode, Machine, Program, ProviderToken, Term};
use cq_sequence_plan::{Advice, KeyPartitionPolicy, cost, decompose, decompose_with_key_partitions};
use cq_sequence_rewrite_passes::key_partition;
use quote::{ToTokens, quote};

fn machine(parts: Option<usize>) -> Machine {
    Machine {
        name: syn::parse_quote!(tasks), layout: None, items: 1,
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
        },
    }
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
    assert!(!automatic.trace.iter().any(|a| matches!(a, Advice::KeyPartition(k) if k.name == "provider")));
}

#[test]
fn routing_setup_keeps_small_providers_serial_and_amortizes_large_builds() {
    for rows in [0usize, 1, 4, 4096] {
        let mut source = provider_program();
        source.evidence = syn::parse2(quote!(evidence { count R = #rows; count L = 1; })).unwrap();
        let machine = machine(Some(4));
        let automatic = decompose(&source, &machine, &catalog()).unwrap();
        assert_eq!(
            automatic.trace.iter().any(|a| matches!(a, Advice::KeyPartition(k) if k.name == "provider")),
            rows == 4096,
            "rows={rows}: {}", automatic.program.to_token_stream(),
        );
        replay(&source, &automatic);
        if rows <= 4 {
            assert!(!automatic.program.to_token_stream().to_string().contains("Sharded < Index >"));
        }
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
