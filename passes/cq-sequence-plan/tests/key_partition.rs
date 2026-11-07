#[macro_use]
mod support;
use structures::{Program as Structures, physical::Program as Physical};
use cq_lang_sequence::{Catalog, CodecToken, LevelNode, Machine, Program, Term, structures};
use cq_sequence_plan::rewrites::key_partition;
use cq_sequence_plan::{
    Advice, KeyPartitionPolicy, cost, decompose, decompose_with_key_partitions,
};
use quote::{ToTokens, quote};
use structures::Capability;

fn machine(parts: Option<usize>) -> Machine {
    Machine {
        gpu: None,
        name: syn::parse_quote!(tasks),
        layout: None,
        items: 1,
        key_partition_threshold: 65536,
        key_partition_cache_threshold: 6 * 1024 * 1024,
        root: LevelNode {
            name: syn::parse_quote!(host),
            host_rust: true,
            collaborative: false,
            dynamic: false,
            width: None,
            shared_bytes: None,
            children: vec![LevelNode {
                name: syn::parse_quote!(pool),
                host_rust: true,
                collaborative: false,
                dynamic: true,
                width: parts,
                shared_bytes: None,
                children: vec![LevelNode {
                    name: syn::parse_quote!(task),
                    host_rust: true,
                    collaborative: false,
                    dynamic: false,
                    width: None,
                    shared_bytes: None,
                    children: vec![],
                }],
            }],
        },
    }
}

fn catalog() -> Catalog {
    let levels = vec![
        vec!["host".into()],
        vec!["host".into(), "pool".into()],
        vec!["host".into(), "pool".into(), "task".into()],
    ];
    let declared: structures::Providers = syn::parse_quote!(providers {
        index = Index { reads [exact], build inferred };
        sharded = ::cq_storage::sharded::Sharded<_> { reads [], build typed, routed 0, shared };
        sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared };
    });
    let [index, sharded, sort] = <[_; 3]>::try_from(declared.declarations).unwrap();
    let mut catalog = Catalog {
        codecs: vec![CodecToken {
            codec: None,
            codec_id: None,
            ordered: true,
            levels: levels.clone(),
        }],
        ..Catalog::default()
    };
    let routed = sharded.compose(index.instance()).unwrap();
    catalog.register(index, levels.clone());
    catalog.declarations.push(sharded);
    catalog.providers.push(cq_lang_sequence::CatalogEntry {
        provider: routed,
        levels: levels.clone(),
    });
    catalog.register(sort, levels);
    catalog
}

/// The `index` declaration a test widens; programs re-declare it with
/// [`declare_as`] (PL-Catalog).
fn index(catalog: &mut Catalog) -> &mut Vec<Capability> {
    &mut catalog
        .declarations
        .iter_mut()
        .find(|declaration| declaration.token == "index")
        .unwrap()
        .facts
        .reads
}

/// PL-Catalog: declare every token the catalog declares exactly as it does.
fn declare_as(structures: &mut structures::Structures, catalog: &Catalog) {
    for declaration in &mut structures.providers.declarations {
        if let Some(declared) = catalog.declaration(&declaration.token) {
            *declaration = declared.clone();
        }
    }
}

// TPC-H Q1-shaped keyed aggregates retain line identity in the complete
// valuation, then canonicalize and aggregate within key-disjoint partitions.
fn grouped(global: bool) -> (Physical, Structures, Program) {
    let keys = if global {
        quote!()
    } else {
        quote!(returnflag, linestatus)
    };
    let head = if global {
        quote!()
    } else {
        quote!(returnflag, linestatus,)
    };
    let accumulator = if global {
        quote!()
    } else {
        quote!(totals group = @group_map as ::std::collections::HashMap<_, (i64, i64, usize,)>;)
    };
    let (physical, mut structures, program) = fixture! {
        physical {
            relation Lineitem(orderkey:i64, linenumber:i32, returnflag:i32, linestatus:i32, quantity:i64, extendedprice:i64);
            lineitem = let canon rename Lineitem {
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
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; group_map = ::std::collections::HashMap<_, _, ::cq_storage::HashState> { reads [], build typed }; }
            lineitem host scan = @reader as Reader [];
            lineitem rows = @sort as ::cq_storage::dedup::Sort;
            lineitem canon = rows;
            #accumulator
        }
        sequence {
            input = store rows lineitem;
            result = store group totals canon input;
        }
    };
    structures.structures.declare(&[]).unwrap();
    (physical, structures, program)
}

fn named_grouped(global: bool) -> (Physical, Structures, Program) {
    let (mut physical, mut structures, mut source) = grouped(global);
    source.sequence.definitions.insert(
        1,
        cq_lang_sequence::Definition {
            name: syn::parse_quote!(distinct),
            term: syn::parse_quote!(canon input),
        },
    );
    source.sequence.definitions.insert(
        2,
        cq_lang_sequence::Definition {
            name: syn::parse_quote!(alias),
            term: syn::parse_quote!(distinct),
        },
    );
    source.sequence.definitions.last_mut().unwrap().term =
        syn::parse_quote!(store group totals alias);
    (physical, structures, source)
}

fn policy(name: &str, select: bool) -> KeyPartitionPolicy {
    KeyPartitionPolicy {
        name: syn::parse_str(name).unwrap(),
        select,
    }
}

fn replay(physical: &Physical, structures: &Structures, source: &Program, planned: &cq_sequence_plan::Planned) {
    let (mut replayed_physical, mut replayed_structures, mut replayed) =
        (physical.clone(), structures.clone(), source.clone());
    for advice in &planned.trace {
        let parsed: Advice = syn::parse2(advice.to_token_stream()).unwrap();
        assert_eq!(*advice, parsed);
        (replayed_physical, replayed_structures, replayed) =
            parsed.apply(&replayed_physical, &replayed_structures, &replayed).unwrap();
    }
    assert_eq!(replayed_physical, planned.physical);
    assert_eq!(replayed_structures, planned.structures);
    assert_eq!(replayed, planned.program);
}

#[test]
fn grouped_tpch_plan_prints_fixed_partition_and_replays() {
    let (mut physical, mut structures, source) = grouped(false);
    let machine = machine(Some(8));
    let planned = decompose(&physical, &structures, &source, &machine, &catalog()).unwrap();
    let printed = planned.program.to_token_stream().to_string();
    assert!(
        printed.contains("partition_k [returnflag , linestatus] (8)"),
        "{printed}"
    );
    assert!(
        printed.contains("flatten group totals canon partition_k"),
        "{printed}"
    );
    assert!(
        printed.contains("__cq_key_input_result = store input"),
        "{printed}"
    );
    assert_eq!(
        planned.program,
        syn::parse2(planned.program.to_token_stream()).unwrap()
    );
    cq_lang_sequence::check_grading(&planned.physical, &planned.structures, &planned.program, &machine).unwrap();
    replay(&physical, &structures, &source, &planned);
}

#[test]
fn cost_selects_streamed_collectives_and_advice_can_force_or_forbid() {
    let (mut physical, mut structures, source) = grouped(false);
    let machine = machine(Some(8));
    let forced =
        decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &[policy("result", true)])
            .unwrap();
    let forbidden =
        decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &[policy("result", false)])
            .unwrap();
    assert!(
        forced
            .trace
            .iter()
            .any(|a| matches!(a, Advice::KeyPartition(_)))
    );
    assert!(
        !forbidden
            .trace
            .iter()
            .any(|a| matches!(a, Advice::KeyPartition(_)))
    );
    assert!(
        cost(&forced.physical, &forced.structures, &forced.program, &machine)
            .unwrap()
            .better_than(&cost(&forbidden.physical, &forbidden.structures, &forbidden.program, &machine).unwrap())
    );
    let automatic = decompose(&physical, &structures, &source, &machine, &catalog()).unwrap();
    assert!(
        automatic
            .trace
            .iter()
            .any(|a| matches!(a, Advice::KeyPartition(_)))
    );
    replay(&physical, &structures, &source, &forced);
    replay(&physical, &structures, &source, &forbidden);
}

#[test]
fn known_small_collectives_pay_setup_and_large_collectives_amortize_it() {
    for global in [false, true] {
        for rows in [0usize, 1, 4, 4096] {
            let (mut physical, mut structures, mut source) = grouped(global);
            source.evidence = syn::parse2(quote!(evidence from authored("passes/cq-sequence-plan/tests/key_partition.rs fixture") { count Lineitem = #rows; })).unwrap();
            let planned = decompose(&physical, &structures, &source, &machine(Some(8)), &catalog()).unwrap();
            assert_eq!(
                planned
                    .trace
                    .iter()
                    .any(|a| matches!(a, Advice::KeyPartition(_))),
                rows == 4096,
                "global={global}, rows={rows}: {}",
                planned.program.to_token_stream(),
            );
            replay(&physical, &structures, &source, &planned);
        }
    }
}

#[test]
fn grouped_named_canon_moves_inside_routed_parts_and_replays() {
    for global in [false, true] {
        let (mut physical, mut structures, source) = named_grouped(global);
        let machine = machine(Some(8));
        let planned = decompose(&physical, &structures, &source, &machine, &catalog()).unwrap();
        assert!(
            planned
                .trace
                .iter()
                .any(|a| matches!(a, Advice::KeyPartition(k) if k.name == "result"))
        );
        let distinct = planned
            .program
            .sequence
            .definitions
            .iter()
            .find(|d| d.name == "distinct")
            .unwrap();
        assert!(
            matches!(&distinct.term, Term::Ref(name) if name == "input"),
            "named Canon must not execute before routing: {}",
            planned.program.to_token_stream()
        );
        assert!(
            planned
                .program
                .to_token_stream()
                .to_string()
                .contains("group totals canon partition_k")
        );
        replay(&physical, &structures, &source, &planned);
    }
}

#[test]
fn group_partition_preserves_shared_and_stored_canonical_producers() {
    for stored in [false, true] {
        let (mut physical, mut structures, mut source) = named_grouped(false);
        if stored {
            source.sequence.definitions[1].term = syn::parse_quote!(store canon input);
        } else {
            source.sequence.definitions.insert(
                3,
                cq_lang_sequence::Definition {
                    name: syn::parse_quote!(other_consumer),
                    term: syn::parse_quote!(store distinct),
                },
            );
        }
        let original = source.sequence.definitions[1].term.clone();
        let (target_physical, target_structures, target) = key_partition(&physical, &structures, &source, quote!(result, staged, [returnflag, linestatus], 8)).unwrap();
        assert_eq!(target.sequence.definitions[1].term, original);
        assert!(
            !target
                .to_token_stream()
                .to_string()
                .contains("group totals canon partition_k")
        );
    }
}

#[test]
fn force_rejects_missing_width_and_unknown_advice() {
    for ((physical, structures, source), machine, policies) in [
        (grouped(false), machine(None), vec![policy("result", true)]),
        (
            grouped(false),
            machine(Some(8)),
            vec![policy("missing", false)],
        ),
        (
            grouped(false),
            machine(Some(8)),
            vec![policy("result", true), policy("result", false)],
        ),
    ] {
        assert!(decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &policies).is_err());
    }
}

#[test]
fn checked_advice_rejects_invalid_keys_counts_and_collisions() {
    let (mut physical, mut structures, source) = grouped(false);
    for arguments in [
        quote!(result, staged, [returnflag], 0),
        quote!(result, staged, [quantity], 8),
        quote!(result, input, [returnflag], 8),
        quote!(result, staged, [missing], 8),
        quote!(result, staged, [], 8),
        quote!(missing, staged, [returnflag], 8),
        quote!(input, staged, [returnflag], 8),
    ] {
        assert!(
            key_partition(&physical, &structures, &source, arguments.clone()).is_err(),
            "{arguments}"
        );
    }
}

#[test]
fn canon_partition_and_machine_width_are_explicit() {
    let (mut physical, mut structures, source) = fixture! {
        physical {
            relation R(a:i32,b:i32);
            r = let canon rename R {a -> a,b -> b};
            return r as answer(a,b);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            r host scan = @reader as Reader [];
            r rows = @sort as ::cq_storage::dedup::Sort; r canon = rows;
        }
        sequence { input = store rows r; result = store canon input; }
    };
    for parts in [1, 3, 16] {
        let planned = decompose_with_key_partitions(&physical, &structures, &source, &machine(Some(parts)), &catalog(), &[policy("result", true)])
        .unwrap();
        assert!(planned.trace.iter().any(
            |a| matches!(a, Advice::KeyPartition(k) if k.parts == parts && k.keys.len() == 2)
        ));
        replay(&physical, &structures, &source, &planned);
        let unpartitioned = decompose_with_key_partitions(&physical, &structures, &source, &machine(Some(parts)), &catalog(), &[policy("result", false)])
        .unwrap();
        let routed_cost = cost(&planned.physical, &planned.structures, &planned.program, &machine(Some(parts))).unwrap();
        let global_cost = cost(&unpartitioned.physical, &unpartitioned.structures, &unpartitioned.program, &machine(Some(parts))).unwrap();
        assert_eq!(
            routed_cost.better_than(&global_cost),
            parts >= 4,
            "flattened tiles still require global Canon: routed={routed_cost}; global={global_cost}",
        );
        let automatic = decompose(&physical, &structures, &source, &machine(Some(parts)), &catalog()).unwrap();
        assert_eq!(
            automatic
                .trace
                .iter()
                .any(|a| matches!(a, Advice::KeyPartition(_))),
            routed_cost.better_than(&global_cost),
            "routing must consume bandwidth: parts={parts}",
        );
        replay(&physical, &structures, &source, &automatic);
    }
}

fn provider_program() -> (Physical, Structures, Program) {
    fixture! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L {a -> a};
            r = let rename R {a -> a,b -> b};
            joined = let canon probe first [a] l,r;
            return joined as answer(a,b);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; index = Index { reads [exact], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @reader as Reader []; r host scan = @reader as Reader []; r = @index as Index [a];
            joined rows = @sort as ::cq_storage::dedup::Sort; joined canon = rows;
        }
        sequence {
            provider = store rows r;
            input = store rows l;
            result = store canon flatten lookup r [a] input;
        }
    }
}

#[test]
fn provider_store_retains_parts_and_records_routed_provider_type() {
    let (mut physical, mut structures, source) = provider_program();
    let planned = decompose_with_key_partitions(&physical, &structures, &source, &machine(Some(4)), &catalog(), &[policy("provider", true)])
    .unwrap();
    let provider = planned
        .program
        .sequence
        .definitions
        .iter()
        .find(|d| d.name == "provider")
        .unwrap();
    assert!(
        matches!(&provider.term, Term::Store { inner, .. } if matches!(inner.as_ref(), Term::PartitionK { parts: 4, .. }))
    );
    assert!(
        planned.structures
            .to_token_stream()
            .to_string()
            .contains("Sharded < Index >")
    );
    replay(&physical, &structures, &source, &planned);
    assert!(key_partition(&physical, &structures, &source, quote!(provider, staged, [b], 4)).is_err());
    let automatic = decompose(&physical, &structures, &source, &machine(Some(4)), &catalog()).unwrap();
    assert!(
        automatic
            .trace
            .iter()
            .any(|a| matches!(a, Advice::KeyPartition(k)
        if k.name == "provider" && k.threshold == Some(65536)
            && k.cache_threshold == Some(6 * 1024 * 1024)))
    );
}

#[test]
fn provider_choice_does_not_use_cardinality_evidence() {
    for rows in [0usize, 1, 4, 4096, 65535, 65536, 65537] {
        let (mut physical, mut structures, mut source) = provider_program();
        source.evidence = syn::parse2(quote!(evidence from authored("passes/cq-sequence-plan/tests/key_partition.rs fixture") { count R = #rows; count L = 1; })).unwrap();
        let machine = machine(Some(4));
        let automatic = decompose(&physical, &structures, &source, &machine, &catalog()).unwrap();
        assert!(
            automatic
                .trace
                .iter()
                .any(|a| matches!(a, Advice::KeyPartition(k)
            if k.name == "provider" && k.threshold == Some(machine.key_partition_threshold)
                && k.cache_threshold == Some(machine.key_partition_cache_threshold)))
        );
        assert!(
            automatic.structures
                .to_token_stream()
                .to_string()
                .contains("Sharded < Index >")
        );
        replay(&physical, &structures, &source, &automatic);
    }
}

#[test]
fn force_does_not_override_an_authored_grade() {
    let (mut physical, mut structures, mut source) = grouped(false);
    source.graded = Some(syn::parse_quote!(graded tasks { result at host; }));
    assert!(
        decompose_with_key_partitions(
            &physical, &structures, &source,
            &machine(Some(8)),
            &catalog(),
            &[policy("result", true)]
        )
        .is_err()
    );
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
    let (mut physical, mut structures, source) = grouped(true);
    let machine = machine(Some(8));
    let forced =
        decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &[policy("result", true)])
            .unwrap();
    let forbidden =
        decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &[policy("result", false)])
            .unwrap();
    let definition = forced
        .program
        .sequence
        .definitions
        .iter()
        .find(|d| d.name == "result")
        .unwrap();
    assert!(
        matches!(&definition.term, Term::Store { inner, .. } if matches!(inner.as_ref(), Term::Group(_, _)))
    );
    assert!(
        forced
            .trace
            .iter()
            .any(|advice| matches!(advice, Advice::KeyPartition(k) if k.keys.len() == 6))
    );
    let facts = cq_lang_sequence::check_grading(&forced.physical, &forced.structures, &forced.program, &machine).unwrap();
    assert_eq!(facts.shapes["result"], vec![cq_lang_sequence::Len::Lit(1)]);
    assert_eq!(facts.lengths["result"], cq_lang_sequence::Len::Lit(1));
    assert!(
        cost(&forced.physical, &forced.structures, &forced.program, &machine)
            .unwrap()
            .better_than(&cost(&forbidden.physical, &forbidden.structures, &forbidden.program, &machine).unwrap())
    );
    assert!(
        decompose(&physical, &structures, &source, &machine, &catalog())
            .unwrap()
            .trace
            .iter()
            .any(|a| matches!(a, Advice::KeyPartition(_)))
    );
    assert!(
        !forbidden
            .trace
            .iter()
            .any(|a| matches!(a, Advice::KeyPartition(_)))
    );
    replay(&physical, &structures, &source, &forced);
}

#[test]
fn globally_unique_aggregates_use_tiles_without_canonicalizers() {
    let (mut physical, mut structures, mut source) = grouped(true);
    source.evidence = syn::parse_quote!(evidence from authored("passes/cq-sequence-plan/tests/key_partition.rs fixture") { trusted_key Lineitem [orderkey, linenumber]; });
    let machine = machine(Some(8));
    let planned =
        decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &[policy("result", true)])
            .unwrap();
    let result = planned
        .program
        .sequence
        .definitions
        .iter()
        .find(|d| d.name == "result")
        .unwrap();
    assert!(matches!(&result.term, Term::Store { inner, .. }
        if matches!(inner.as_ref(), Term::Group(_, input)
            if matches!(input.as_ref(), Term::Tile { n: 8, .. }))));
    assert!(
        planned
            .trace
            .iter()
            .any(|a| matches!(a, Advice::KeyPartition(k) if k.keys.is_empty()))
    );
    let facts = cq_lang_sequence::check_grading(&planned.physical, &planned.structures, &planned.program, &machine).unwrap();
    assert_eq!(facts.shapes["result"], vec![cq_lang_sequence::Len::Lit(1)]);
    let forbidden =
        decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &[policy("result", false)])
            .unwrap();
    assert!(
        cost(&planned.physical, &planned.structures, &planned.program, &machine)
            .unwrap()
            .better_than(&cost(&forbidden.physical, &forbidden.structures, &forbidden.program, &machine).unwrap())
    );
    replay(&physical, &structures, &source, &planned);
}

#[test]
fn global_partial_advice_roundtrips_and_rejects_zero_tile_width() {
    let advice: Advice = syn::parse_str("key_partition(result, staged, [], 8)").unwrap();
    assert_eq!(advice, syn::parse2(advice.to_token_stream()).unwrap());
    let (physical, structures, source) = grouped(true);
    assert!(key_partition(&physical, &structures, &source, quote!(result, staged, [], 0)).is_err());
}

#[test]
fn empty_global_partial_plans_keep_the_singleton_identity_shape() {
    for unique in [false, true] {
        let (mut physical, mut structures, mut source) = grouped(true);
        source.evidence = if unique {
            syn::parse_quote!(evidence from authored("passes/cq-sequence-plan/tests/key_partition.rs fixture") { count Lineitem = 0; trusted_key Lineitem [orderkey, linenumber]; })
        } else {
            syn::parse_quote!(evidence from authored("passes/cq-sequence-plan/tests/key_partition.rs fixture") { count Lineitem = 0; })
        };
        let machine = machine(Some(8));
        let planned =
            decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &[policy("result", true)])
                .unwrap();
        let facts = cq_lang_sequence::check_grading(&planned.physical, &planned.structures, &planned.program, &machine).unwrap();
        assert_eq!(facts.shapes["result"], vec![cq_lang_sequence::Len::Lit(1)]);
        assert_eq!(facts.lengths["result"], cq_lang_sequence::Len::Lit(1));
        replay(&physical, &structures, &source, &planned);
    }
}

fn selected_provider_parts(
    term: &Term,
    workers: usize,
    rows: usize,
    host_rows: usize,
    input: &str,
    counted_input: &mut bool,
) -> usize {
    use cq_lang_sequence::{Guard, Len};
    match term {
        Term::Store { inner, .. } => {
            selected_provider_parts(inner, workers, rows, host_rows, input, counted_input)
        }
        Term::Choose(arms) => {
            let (_, selected) = arms
                .iter()
                .find(|(guard, _)| match guard {
                    Guard::WorkersLt(bound) => workers < *bound,
                    Guard::Lt(Len::Input(name), threshold) if name == input => {
                        *counted_input = true;
                        rows < *threshold
                    }
                    Guard::Lt(Len::Input(name), threshold) if name == "R" => host_rows < *threshold,
                    Guard::Else => true,
                    _ => panic!("unexpected guard: {guard:?}"),
                })
                .unwrap();
            selected_provider_parts(selected, workers, rows, host_rows, input, counted_input)
        }
        Term::PartitionK { parts, .. } => *parts,
        _ => panic!("provider cover required"),
    }
}

#[test]
fn unknown_provider_records_nested_worker_cache_and_published_count_guards() {
    use cq_lang_sequence::{Guard, Len};
    let (mut physical, mut structures, source) = provider_program();
    let mut machine = machine(Some(4));
    machine.key_partition_threshold = 17;
    machine.key_partition_cache_threshold = 31;
    let planned = decompose(&physical, &structures, &source, &machine, &catalog()).unwrap();
    let advice = planned
        .trace
        .iter()
        .find_map(|advice| match advice {
            Advice::KeyPartition(advice) if advice.name == "provider" => Some(advice),
            _ => None,
        })
        .expect("unknown-cardinality provider must retain runtime advice");
    assert_eq!(advice.threshold, Some(17));
    assert_eq!(advice.cache_threshold, Some(31));
    let provider = planned
        .program
        .sequence
        .definitions
        .iter()
        .find(|d| d.name == "provider")
        .unwrap();
    let Term::Store { inner, .. } = &provider.term else {
        panic!("provider Store required")
    };
    let Term::Choose(arms) = inner.as_ref() else {
        panic!("runtime Choose required")
    };
    assert_eq!(arms.len(), 3);
    assert_eq!(arms[0].0, Guard::WorkersLt(2));
    assert_eq!(
        arms[1].0,
        Guard::Lt(Len::Input(advice.input.to_string()), 17)
    );
    assert_eq!(arms[2].0, Guard::Else);
    let Term::Choose(single_worker) = &arms[0].1 else {
        panic!("single-worker cache choice required")
    };
    assert_eq!(single_worker.len(), 3);
    assert_eq!(single_worker[0].0, Guard::Lt(Len::Input("R".into()), 31));
    assert_eq!(
        single_worker[1].0,
        Guard::Lt(Len::Input(advice.input.to_string()), 31)
    );
    assert_eq!(single_worker[2].0, Guard::Else);
    for (term, parts) in [
        (&single_worker[0].1, 1),
        (&single_worker[1].1, 1),
        (&single_worker[2].1, 4),
        (&arms[1].1, 1),
        (&arms[2].1, 4),
    ] {
        assert_eq!(
            *term,
            Term::PartitionK {
                keys: advice.keys.clone(),
                parts,
                inner: Box::new(Term::Ref(advice.input.clone())),
            }
        );
    }
    for (workers, rows, host_rows, expected_parts, expected_count) in [
        (0, 31, 31, 4, true),
        (1, 0, 0, 1, false),
        (1, 17, 17, 1, false),
        (1, 30, 30, 1, false),
        (1, 17, 31, 1, true),
        (1, 30, 31, 1, true),
        (1, 31, 31, 4, true),
        (1, 32, 32, 4, true),
        (1, usize::MAX, usize::MAX, 4, true),
        (2, 16, 16, 1, true),
        (2, 17, 17, 4, true),
        (2, 18, 18, 4, true),
        (8, 16, 16, 1, true),
        (8, 17, 17, 4, true),
    ] {
        let mut counted = false;
        assert_eq!(
            selected_provider_parts(
                &provider.term,
                workers,
                rows,
                host_rows,
                &advice.input.to_string(),
                &mut counted,
            ),
            expected_parts
        );
        assert_eq!(counted, expected_count);
    }
    let facts = cq_lang_sequence::check_grading(&planned.physical, &planned.structures, &planned.program, &machine).unwrap();
    assert_eq!(facts.shapes["provider"][0], Len::Unknown(None));
    assert!(
        planned
            .program
            .graded
            .as_ref()
            .unwrap()
            .grades
            .iter()
            .any(|grade| grade.name == advice.input && grade.level.as_slice() == ["host"])
    );
    replay(&physical, &structures, &source, &planned);
    assert_eq!(
        planned.program,
        syn::parse2(planned.program.to_token_stream()).unwrap()
    );
    assert!(cq_lang_sequence::check_grading(&planned.physical, &planned.structures, &planned.program, &self::machine(Some(8))).is_err());
}

#[test]
fn cache_policy_matches_equation_at_cutoffs_and_every_fixed_width() {
    let (mut physical, mut structures, source) = provider_program();
    for parts in [1usize, 3, 16] {
        for cache in [9usize, 17, 31] {
            let advice: Advice = syn::parse2(quote!(
                key_partition(provider, published, [a], #parts, threshold 17, cache #cache)
            ))
            .unwrap();
            assert_eq!(advice, syn::parse2(advice.to_token_stream()).unwrap());
            let (routed_physical, routed_structures, routed) = advice.apply(&physical, &structures, &source).unwrap();
            let provider = routed
                .sequence
                .definitions
                .iter()
                .find(|d| d.name == "provider")
                .unwrap();
            for workers in [0, 1, 2, 8] {
                for rows in [0, 8, 9, 16, 17, 18, 30, 31, 32, usize::MAX] {
                    let expected = if rows >= 17 && (workers > 1 || rows >= cache) {
                        parts
                    } else {
                        1
                    };
                    for host_rows in [rows, usize::MAX] {
                        assert_eq!(
                            selected_provider_parts(
                                &provider.term,
                                workers,
                                rows,
                                host_rows,
                                "published",
                                &mut false,
                            ),
                            expected
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn provider_force_forbid_and_authored_grade_override_runtime_choice() {
    let (mut physical, mut structures, source) = provider_program();
    for select in [true, false] {
        let planned = decompose_with_key_partitions(&physical, &structures, &source, &machine(Some(4)), &catalog(), &[policy("provider", select)])
        .unwrap();
        let provider = planned
            .program
            .sequence
            .definitions
            .iter()
            .find(|d| d.name == "provider")
            .unwrap();
        let Term::Store { inner, .. } = &provider.term else {
            panic!("provider Store required")
        };
        assert!(!matches!(inner.as_ref(), Term::Choose(_)));
        if select {
            assert!(matches!(inner.as_ref(), Term::PartitionK { parts: 4, .. }));
            assert!(
                planned
                    .trace
                    .iter()
                    .any(|a| matches!(a, Advice::KeyPartition(k)
                if k.name == "provider" && k.threshold.is_none() && k.cache_threshold.is_none()))
            );
        } else {
            assert!(
                !planned
                    .trace
                    .iter()
                    .any(|a| matches!(a, Advice::KeyPartition(k) if k.name == "provider"))
            );
            assert!(
                !planned.structures
                    .to_token_stream()
                    .to_string()
                    .contains("Sharded < Index >")
            );
        }
        replay(&physical, &structures, &source, &planned);
    }
    let mut authored = source.clone();
    authored.graded = Some(syn::parse_quote!(graded tasks { provider at host; }));
    let planned = decompose(&physical, &structures, &authored, &machine(Some(4)), &catalog()).unwrap();
    assert!(
        !planned
            .trace
            .iter()
            .any(|a| matches!(a, Advice::KeyPartition(k) if k.name == "provider"))
    );
}

#[test]
fn adaptive_provider_threshold_is_positive_and_provider_only() {
    let (mut physical, mut structures, source) = provider_program();
    let mut invalid = machine(Some(4));
    invalid.key_partition_threshold = 0;
    assert!(decompose(&physical, &structures, &source, &invalid, &catalog()).is_err());
    invalid.key_partition_threshold = 17;
    invalid.key_partition_cache_threshold = 0;
    assert!(decompose(&physical, &structures, &source, &invalid, &catalog()).is_err());
    assert!(
        key_partition(
            &physical, &structures, &source,
            quote!(provider, published, [a], 4, threshold 17, cache 0)
        )
        .is_err()
    );
    assert!(key_partition(&physical, &structures, &source, quote!(provider, published, [a], 4, cache 31)).is_err());
    assert!(key_partition(&physical, &structures, &source, quote!(provider, published, [a], 4, threshold 0)).is_err());
    assert!(key_partition(&physical, &structures, &source, quote!(provider, published, [a], 0, threshold 17)).is_err());
    let (physical, structures, source) = grouped(false);
    assert!(
        key_partition(
            &physical, &structures, &source,
            quote!(result, published, [returnflag, linestatus], 4, threshold 17)
        )
        .is_err()
    );
    assert!(
        syn::parse_str::<Advice>("key_partition(provider, published, [a], 4, cutoff 17)").is_err()
    );
    for recipe in [
        "key_partition(provider, published, [a], 4, threshold 17, threshold 31)",
        "key_partition(provider, published, [a], 4, threshold 17, cache 31, cache 63)",
    ] {
        assert!(syn::parse_str::<Advice>(recipe).is_err());
    }
    let reordered: Advice =
        syn::parse_str("key_partition(provider, published, [a], 4, cache 31, threshold 17)")
            .unwrap();
    assert_eq!(reordered, syn::parse2(reordered.to_token_stream()).unwrap());
}

#[test]
fn exact_membership_provider_supports_runtime_partition_choice() {
    let (mut physical, mut structures, source) = fixture! {
        physical {
            relation L(a:i32); relation R(a:i32);
            l = pipe rename L {a -> a};
            r = let rename R {a -> a};
            j = let canon semi eager [a] l,r;
            return j as answer(a);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; index = Index { reads [exact], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @reader as Reader []; r host scan = @reader as Reader [];
            r = @index as Index [a];
            j rows = @sort as ::cq_storage::dedup::Sort; j canon = rows;
        }
        sequence {
            provider = store rows r;
            input = store rows l;
            result = store canon compact j lookup r [a] input;
        }
    };
    let mut membership_catalog = catalog();
    *index(&mut membership_catalog) = vec![Capability::Membership];
    let mut source = source;
    declare_as(&mut structures.structures, &membership_catalog);
    let planned = decompose(&physical, &structures, &source, &machine(Some(4)), &membership_catalog).unwrap();
    assert!(
        planned
            .trace
            .iter()
            .any(|a| matches!(a, Advice::KeyPartition(k)
        if k.name == "provider" && k.threshold == Some(65536)))
    );
    cq_lang_sequence::check_grading(&planned.physical, &planned.structures, &planned.program, &machine(Some(4))).unwrap();
    replay(&physical, &structures, &source, &planned);
}

#[test]
fn scheduled_range_obligation_keeps_native_provider() {
    let (mut physical, mut structures, mut source) = provider_program();
    source.sequence.definitions.last_mut().unwrap().term =
        syn::parse_quote!(store tile 2 canon flatten lookup r [a] input);
    let mut catalog = catalog();
    // Native cursors support the authored distributed lookup. The routed
    // wrapper remains exact-only: STR-Routed grants no Range.
    index(&mut catalog).push(Capability::Range);
    declare_as(&mut structures.structures, &catalog);
    let machine = machine(Some(4));
    let planned = decompose(&physical, &structures, &source, &machine, &catalog).unwrap();
    let facts = cq_lang_sequence::check_grading(&planned.physical, &planned.structures, &planned.program, &machine).unwrap();
    assert_eq!(
        facts.capabilities[&("r".into(), vec!["a".into()])],
        Capability::Range
    );
    assert!(
        !planned
            .trace
            .iter()
            .any(|a| matches!(a, Advice::KeyPartition(k)
        if k.name == "provider"))
    );
    assert!(
        !planned.structures
            .to_token_stream()
            .to_string()
            .contains("Sharded < Index >")
    );
    replay(&physical, &structures, &source, &planned);
    assert!(
        decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog, &[policy("provider", true)],)
            .is_err()
    );
}

#[test]
fn smallest_candidate_directories_keep_their_native_two_level_providers() {
    let (physical, structures) = structure_fixture! {
        physical {
            relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
            l = pipe rename L {a -> a,b -> b};
            r = let rename R {b -> b,c -> c};
            s = let rename S {a -> a,c -> c};
            candidate = pipe probe smallest [b] l,r;
            checked = let canon semi eager [a,c] candidate,s;
            return checked as answer(a,b,c);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; index = Index { reads [exact], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @reader as Reader []; r host scan = @reader as Reader []; s host scan = @reader as Reader [];
            r = @index as Index [b][c]; s = @index as Index [a][c];
            checked rows = @sort as ::cq_storage::dedup::Sort; checked canon = rows;
        }
    };
    let mut available = catalog();
    index(&mut available).push(Capability::Membership);
    let mut structures = structures;
    declare_as(&mut structures.structures, &available);
    let source = cq_sequence_lowering::compile(&physical, &structures).unwrap();
    let (selected_physical, selected) = cq_sequence_plan::rewrites::select_from_catalog_preserving(
        &physical, &structures,
        &available,
        &["r".to_owned(), "s".to_owned()].into_iter().collect(),
    )
    .unwrap();
    assert_eq!(selected_physical, physical);
    assert_eq!(selected, structures);
    let machine = machine(Some(4));
    let planned = decompose(&physical, &structures, &source, &machine, &available).unwrap();
    assert!(
        !planned.structures
            .to_token_stream()
            .to_string()
            .contains("Sharded")
    );
    replay(&physical, &structures, &source, &planned);
    for name in ["r", "s"] {
        assert!(
            decompose_with_key_partitions(&physical, &structures, &source, &machine, &available, &[policy(name, true)],)
                .is_err()
        );
    }
}

#[test]
fn filtered_provider_retains_valid_single_level_routing() {
    let (mut physical, mut structures, source) = fixture! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32,unused:i32);
            l = pipe rename L {a -> a};
            r = let rename R {a -> a,b -> b,unused -> unused};
            joined = pipe probe first [a] l,r;
            answer = let canon project joined keep {a,b};
            return answer as answer(a,b);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; index = Index { reads [exact], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @reader as Reader []; r host scan = @reader as Reader [];
            r = @index as Index [a] payload [a,b] constants {};
            answer rows = @sort as ::cq_storage::dedup::Sort; answer canon = rows;
        }
        sequence {
            provider = store rows r;
            input = store rows l;
            joined = flatten lookup r[a] input;
            result = store canon map answer joined;
        }
    };
    let machine = machine(Some(4));
    let planned = decompose(&physical, &structures, &source, &machine, &catalog()).unwrap();
    assert!(planned.trace.iter().any(
        |advice| matches!(advice, Advice::KeyPartition(partition) if partition.name == "provider")
    ));
    replay(&physical, &structures, &source, &planned);
    assert!(
        decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &[policy("provider", true)],)
            .is_ok()
    );
}

#[test]
fn provider_routing_requires_flat_carried_valuations() {
    let (mut physical, mut structures, mut source) = provider_program();
    source.sequence.definitions[0].term = syn::parse_quote!(store tile 2 rows r);
    let machine = machine(Some(4));
    let planned = decompose(&physical, &structures, &source, &machine, &catalog()).unwrap();
    assert!(
        !planned
            .trace
            .iter()
            .any(|a| matches!(a, Advice::KeyPartition(k)
        if k.name == "provider"))
    );
    replay(&physical, &structures, &source, &planned);
    assert!(
        decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &[policy("provider", true)],)
            .is_err()
    );
}

#[test]
fn routed_catalog_requires_a_routed_entry_and_scheduled_read_levels() {
    let (mut physical, mut structures, mut source) = provider_program();
    source.graded = Some(syn::parse_quote!(graded tasks { result at host; }));
    for restriction in 0..2 {
        let mut catalog = catalog();
        match restriction {
            0 => {
                catalog.providers.remove(1);
            }
            _ => {
                catalog.providers[1].levels =
                    vec![vec!["host".into(), "pool".into(), "task".into()]]
            }
        }
        let planned = decompose(&physical, &structures, &source, &machine(Some(4)), &catalog).unwrap();
        assert!(
            !planned
                .trace
                .iter()
                .any(|a| matches!(a, Advice::KeyPartition(k)
            if k.name == "provider")),
            "restriction {restriction}"
        );
        replay(&physical, &structures, &source, &planned);
    }
}

#[test]
fn task_flatten_decomposition_does_not_introduce_unnamed_separated_stores() {
    let (mut physical, mut structures, mut source) = provider_program();
    source.sequence.definitions.insert(
        2,
        syn::parse_quote!(
            expanded = flatten lookup r [a] input;
        ),
    );
    source.sequence.definitions.last_mut().unwrap().term = syn::parse_quote!(store canon expanded);
    let mut catalog = catalog();
    index(&mut catalog).push(Capability::Range);
    declare_as(&mut structures.structures, &catalog);
    let machine = machine(Some(4));
    let planned =
        decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog, &[policy("result", false)])
            .unwrap();
    assert!(
        !planned
            .trace
            .iter()
            .any(|a| matches!(a, Advice::Segment { .. }))
    );
    cq_lang_sequence::check_grading(&planned.physical, &planned.structures, &planned.program, &machine).unwrap();
    replay(&physical, &structures, &source, &planned);
}

#[test]
fn provider_rewrite_preserves_shared_flat_rows_and_published_length_guards() {
    let (mut physical, mut structures, source) = fixture! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L {a -> a};
            r = let rename R {a -> a,b -> b};
            selected = let project r keep {a};
            joined = pipe probe first [a] l,r;
            final_rows = let canon semi eager [a] joined,selected;
            return final_rows as answer(a,b);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; index = Index { reads [exact], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; }
            l host scan = @reader as Reader []; r host scan = @reader as Reader [];
            r = @index as Index [a]; selected = @index as Index [a];
            final_rows rows = @sort as ::cq_storage::dedup::Sort; final_rows canon = rows;
            r rows = @vec as ::std::vec::Vec<_>;
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
    let (routed_physical, routed_structures, routed) = key_partition(&physical, &structures, &source, quote!(provider, published, [a], 4, threshold 17)).unwrap();
    let derived = routed
        .sequence
        .definitions
        .iter()
        .find(|d| d.name == "derived")
        .unwrap();
    assert_eq!(
        derived.term,
        syn::parse_quote!(store map selected published)
    );
    let same_rows = routed
        .sequence
        .definitions
        .iter()
        .find(|d| d.name == "same_rows")
        .unwrap();
    assert_eq!(
        same_rows.term,
        syn::parse_quote!(choose [len published < 17 => published, else => published])
    );
    // A second provider can now publish the shared rows without accidentally
    // receiving the first provider's new nested cover level.
    let (twice_physical, twice_structures, twice) = key_partition(
        &routed_physical, &routed_structures, &routed,
        quote!(derived, derived_input, [a], 4, threshold 17),
    )
    .unwrap();
    let input = twice
        .sequence
        .definitions
        .iter()
        .find(|d| d.name == "derived_input")
        .unwrap();
    assert_eq!(input.term, syn::parse_quote!(store map selected published));
    let facts = cq_lang_sequence::contract::check(&twice_physical, &twice_structures, &twice).unwrap();
    assert_eq!(facts.shapes["derived_input"].len(), 1);
    assert_eq!(facts.shapes["same_rows"].len(), 1);
    assert_eq!(twice, syn::parse2(twice.to_token_stream()).unwrap());

    let mut catalog = catalog();
    index(&mut catalog).push(Capability::Membership);
    let mut source = source;
    declare_as(&mut structures.structures, &catalog);
    let machine = machine(Some(4));
    let planned = decompose(&physical, &structures, &source, &machine, &catalog).unwrap();
    assert!(
        planned
            .trace
            .iter()
            .any(|a| matches!(a, Advice::KeyPartition(k)
        if k.name == "derived"))
    );
    cq_lang_sequence::check_grading(&planned.physical, &planned.structures, &planned.program, &machine).unwrap();
    replay(&physical, &structures, &source, &planned);
}

#[test]
fn physical_provider_is_partitioned_once_across_publication_aliases() {
    let (mut physical, mut structures, mut source) = provider_program();
    source.sequence.definitions.insert(
        1,
        syn::parse_quote!(
            provider_alias = store provider;
        ),
    );
    let machine = machine(Some(4));
    for policies in [
        vec![],
        vec![policy("provider", true)],
        vec![policy("provider_alias", true)],
    ] {
        let planned =
            decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &policies).unwrap();
        let routed: Vec<_> = planned
            .trace
            .iter()
            .filter_map(|advice| match advice {
                Advice::KeyPartition(advice)
                    if advice.name == "provider" || advice.name == "provider_alias" =>
                {
                    Some(advice)
                }
                _ => None,
            })
            .collect();
        assert_eq!(routed.len(), 1);
        assert_eq!(
            routed[0].name,
            policies
                .first()
                .map_or(syn::parse_quote!(provider_alias), |policy| policy
                    .name
                    .clone(),)
        );
        assert_eq!(
            planned.structures
                .to_token_stream()
                .to_string()
                .matches("Sharded < Index >")
                .count(),
            1
        );
        cq_lang_sequence::check_grading(&planned.physical, &planned.structures, &planned.program, &machine).unwrap();
        replay(&physical, &structures, &source, &planned);
    }
    for forbidden in ["provider", "provider_alias"] {
        let planned = decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &[policy(forbidden, false)])
        .unwrap();
        assert!(
            !planned.structures
                .to_token_stream()
                .to_string()
                .contains("Sharded")
        );
        replay(&physical, &structures, &source, &planned);
    }
    assert!(
        decompose_with_key_partitions(
            &physical, &structures, &source,
            &machine,
            &catalog(),
            &[policy("provider", true), policy("provider_alias", true)]
        )
        .is_err()
    );
    assert!(
        decompose_with_key_partitions(
            &physical, &structures, &source,
            &machine,
            &catalog(),
            &[policy("provider", true), policy("provider_alias", false)]
        )
        .is_err()
    );

    let (once_physical, once_structures, once) = key_partition(&physical, &structures, &source, quote!(provider, raw_input, [a], 4, threshold 17)).unwrap();
    assert!(
        key_partition(
            &once_physical, &once_structures, &once,
            quote!(provider_alias, second_input, [a], 4, threshold 17),
        )
        .is_err()
    );
}

#[test]
fn selected_provider_publication_precedes_every_keyed_consumer() {
    let (mut physical, mut structures, mut source) = provider_program();
    source.sequence.definitions.insert(
        2,
        syn::parse_quote!(
            early_read = store flatten lookup r [a] input;
        ),
    );
    source.sequence.definitions.insert(
        3,
        syn::parse_quote!(
            late_alias = store provider;
        ),
    );
    let machine = machine(Some(4));
    let planned = decompose(&physical, &structures, &source, &machine, &catalog()).unwrap();
    let selected: Vec<_> = planned
        .trace
        .iter()
        .filter_map(|advice| match advice {
            Advice::KeyPartition(advice)
                if advice.name == "provider" || advice.name == "late_alias" =>
            {
                Some(advice)
            }
            _ => None,
        })
        .collect();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].name, "provider");
    replay(&physical, &structures, &source, &planned);
    assert!(
        decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &[policy("late_alias", true)])
            .is_err()
    );
    assert!(
        key_partition(
            &physical, &structures, &source,
            quote!(late_alias, late_input, [a], 4, threshold 17),
        )
        .is_err()
    );
}

#[test]
fn unique_keyed_groups_stream_tiles_through_single_use_valuation_spines() {
    let (mut physical, mut structures, mut source) = grouped(false);
    physical.program.definitions.insert(
        1,
        syn::parse_quote!(
            selected = pipe select lineitem if (quantity > 0);
        ),
    );
    let totals = physical
        .program
        .definitions
        .iter_mut()
        .find(|definition| definition.name == "totals")
        .unwrap();
    let cq_lang_sequence::structures::physical::Operator::Group(group) = &mut totals.operator
    else {
        panic!("group fixture")
    };
    group.input = syn::parse_quote!(selected);
    source.evidence = syn::parse_quote!(evidence from authored("passes/cq-sequence-plan/tests/key_partition.rs fixture") { trusted_key Lineitem [orderkey, linenumber]; });
    source.sequence.definitions = vec![
        syn::parse_quote!(input = rows lineitem;),
        syn::parse_quote!(filtered = compact selected input;),
        syn::parse_quote!(valuations = map selected filtered;),
        syn::parse_quote!(result = store group totals valuations;),
    ];
    let (rewritten_physical, rewritten_structures, rewritten) = key_partition(&physical, &structures, &source, quote!(result, unused, [], 8)).unwrap();
    assert_eq!(
        rewritten.sequence.definitions.len(),
        source.sequence.definitions.len()
    );
    assert_eq!(
        rewritten.sequence.definitions[0].term,
        syn::parse_quote!(tile 8 rows lineitem)
    );
    assert_eq!(
        rewritten.sequence.definitions[1].term,
        source.sequence.definitions[1].term
    );
    assert_eq!(
        rewritten.sequence.definitions[2].term,
        source.sequence.definitions[2].term
    );
    assert_eq!(
        cq_lang_sequence::contract::check(&rewritten_physical, &rewritten_structures, &rewritten)
            .unwrap()
            .shapes["result"]
            .len(),
        1
    );
    let planned = decompose(&physical, &structures, &source, &machine(Some(8)), &catalog()).unwrap();
    assert!(
        planned
            .trace
            .iter()
            .any(|advice| matches!(advice, Advice::KeyPartition(k)
        if k.name == "result" && k.keys.is_empty()))
    );
    assert!(
        !planned
            .program
            .sequence
            .definitions
            .iter()
            .any(|definition| definition
                .name
                .to_string()
                .starts_with("__cq_key_input_result"))
    );
    assert!(
        !planned
            .program
            .to_token_stream()
            .to_string()
            .contains("partition_k")
    );
    let facts = cq_lang_sequence::check_grading(&planned.physical, &planned.structures, &planned.program, &machine(Some(8))).unwrap();
    for view in ["input", "filtered", "valuations"] {
        assert_eq!(
            facts.levels[view],
            vec!["host".to_owned(), "pool".to_owned(), "task".to_owned()]
        );
    }
    replay(&physical, &structures, &source, &planned);
}

#[test]
fn streaming_partial_group_preserves_existing_stores_and_rejects_shared_pipes() {
    let (mut physical, mut structures, mut source) = grouped(false);
    source.evidence = syn::parse_quote!(evidence from authored("passes/cq-sequence-plan/tests/key_partition.rs fixture") { trusted_key Lineitem [orderkey, linenumber]; });
    source.sequence.definitions.last_mut().unwrap().term =
        syn::parse_quote!(store group totals input);
    let (rewritten_physical, rewritten_structures, rewritten) = key_partition(&physical, &structures, &source, quote!(result, unused, [], 8)).unwrap();
    assert_eq!(
        rewritten.sequence.definitions[0],
        source.sequence.definitions[0]
    );
    assert_eq!(
        rewritten.sequence.definitions[1].term,
        syn::parse_quote!(store group totals tile 8 input)
    );
    source.sequence.definitions[0].term = syn::parse_quote!(rows lineitem);
    source
        .sequence
        .definitions
        .insert(1, syn::parse_quote!(other = map lineitem input;));
    assert!(key_partition(&physical, &structures, &source, quote!(result, unused, [], 8)).is_err());
    let (physical, structures, source) = grouped(false);
    assert!(key_partition(&physical, &structures, &source, quote!(result, unused, [], 8)).is_err());
}

#[test]
fn total_rust_partial_group_streams_without_an_extra_host_publication() {
    let (mut physical, mut structures, mut source) = grouped(false);
    physical.program.definitions.insert(
        1,
        syn::parse_quote!(
            mapped = pipe bind lineitem with extra:i64 = (opaque_fn(*quantity));
        ),
    );
    let totals = physical
        .program
        .definitions
        .iter_mut()
        .find(|definition| definition.name == "totals")
        .unwrap();
    let cq_lang_sequence::structures::physical::Operator::Group(group) = &mut totals.operator
    else {
        panic!("group fixture")
    };
    group.input = syn::parse_quote!(mapped);
    source.evidence = syn::parse_quote!(evidence from authored("passes/cq-sequence-plan/tests/key_partition.rs fixture") { trusted_key Lineitem [orderkey, linenumber]; });
    source.sequence.definitions = vec![
        syn::parse_quote!(input = store rows lineitem;),
        syn::parse_quote!(valuations = map mapped input;),
        syn::parse_quote!(result = store group totals valuations;),
    ];
    let (target_physical, target_structures, target) = key_partition(&physical, &structures, &source, quote!(result, host_parts, [], 8)).unwrap();
    assert_eq!(
        target.sequence.definitions.len(),
        source.sequence.definitions.len()
    );
    assert_eq!(
        target.sequence.definitions[1].term,
        syn::parse_quote!(map mapped tile 8 input)
    );
    assert_eq!(
        target.sequence.definitions[2].term,
        syn::parse_quote!(store group totals valuations)
    );
    assert_eq!(target_physical, physical);

    let machine = machine(Some(8));
    let planned =
        decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &[policy("result", true)])
            .unwrap();
    let partial = planned
        .trace
        .iter()
        .find_map(|advice| match advice {
            Advice::KeyPartition(advice) if advice.name == "result" => Some(advice),
            _ => None,
        })
        .unwrap();
    assert!(partial.keys.is_empty());
    let facts = cq_lang_sequence::check_grading(&planned.physical, &planned.structures, &planned.program, &machine).unwrap();
    assert!(
        !planned
            .program
            .sequence
            .definitions
            .iter()
            .any(|definition| definition.name == partial.input)
    );
    assert_eq!(facts.shapes["result"].len(), 1);
    replay(&physical, &structures, &source, &planned);
}

#[test]
fn keyed_group_cost_compares_partial_map_merge_with_key_routing() {
    let machine = machine(Some(8));
    for groups in [1usize, 4096] {
        let (mut physical, mut structures, mut source) = grouped(false);
        source.sequence.definitions.last_mut().unwrap().term =
            syn::parse_quote!(store group totals input);
        source.evidence = syn::parse2(quote!(evidence from authored("passes/cq-sequence-plan/tests/key_partition.rs fixture") {
            trusted_key Lineitem [orderkey, linenumber];
            count Lineitem = 4096;
            count totals = #groups;
        }))
        .unwrap();
        let (partial_physical, partial_structures, mut partial) = key_partition(&physical, &structures, &source, quote!(result, unused, [], 8)).unwrap();
        partial.graded =
            Some(syn::parse_quote!(graded tasks { input at host; result at host.pool; }));
        let (routed_physical, routed_structures, mut routed) = key_partition(&physical, &structures, &source, quote!(result, staged, [returnflag, linestatus], 8)).unwrap();
        routed.graded = Some(syn::parse_quote!(graded tasks {
            input at host; staged at host; result at host.pool;
        }));
        let partial_cost = cost(&partial_physical, &partial_structures, &partial, &machine).unwrap();
        let routed_cost = cost(&routed_physical, &routed_structures, &routed, &machine).unwrap();
        assert_eq!(
            partial_cost.better_than(&routed_cost),
            groups == 1,
            "groups={groups}: partial={partial_cost}, routed={routed_cost}"
        );
        assert_eq!(routed_cost.better_than(&partial_cost), groups == 4096);
        let planned =
            decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &[policy("result", true)])
                .unwrap();
        let chosen = planned
            .trace
            .iter()
            .find_map(|advice| match advice {
                Advice::KeyPartition(advice) if advice.name == "result" => Some(advice),
                _ => None,
            })
            .unwrap();
        assert_eq!(chosen.keys.is_empty(), groups == 1);
        replay(&physical, &structures, &source, &planned);
    }
}

#[test]
fn unknown_partial_map_merge_is_symbolic_not_free() {
    let (mut physical, mut structures, mut source) = grouped(false);
    source.evidence = syn::parse_quote!(evidence from authored("passes/cq-sequence-plan/tests/key_partition.rs fixture") { trusted_key Lineitem [orderkey, linenumber]; });
    let planned = decompose(&physical, &structures, &source, &machine(Some(8)), &catalog()).unwrap();
    assert!(
        planned
            .trace
            .iter()
            .any(|advice| matches!(advice, Advice::KeyPartition(k)
        if k.name == "result" && k.keys.is_empty()))
    );
    let cost = cost(&planned.physical, &planned.structures, &planned.program, &machine(Some(8)))
        .unwrap()
        .to_string();
    assert_eq!(
        planned.program.sequence.output,
        Some(cq_lang_sequence::OutputProtocol::Reserve)
    );
    assert!(cost.contains("8*scan(group(totals))"), "{cost}");
    let (physical, structures, unproved_source) = grouped(false);
    let unproved = decompose(&physical, &structures, &unproved_source, &machine(Some(8)), &catalog()).unwrap();
    assert!(
        unproved
            .trace
            .iter()
            .any(|advice| matches!(advice, Advice::KeyPartition(k)
        if k.name == "result" && !k.keys.is_empty()))
    );
}

#[test]
fn adaptive_provider_publication_preserves_task_owned_pipe_execution() {
    // A row-dependent comparison places the native provider at the pool.
    // A constant bound is a host descriptor and would make the authored task
    // pipe illegal before adaptive routing is considered.
    let (mut physical, mut structures, source) = fixture! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L {a -> a};
            r = pipe rename R {a -> a,b -> b};
            selected = let select r if (b > a);
            joined = let canon probe first [a] l,selected;
            return joined as answer(a,b);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; index = Index { reads [exact], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @reader as Reader []; r host scan = @reader as Reader [];
            selected = @index as Index [a];
            joined rows = @sort as ::cq_storage::dedup::Sort; joined canon = rows;
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
        if forced {
            policies.push(policy("provider", true));
        }
        let planned =
            decompose_with_key_partitions(&physical, &structures, &source, &machine, &catalog(), &policies).unwrap();
        let partition = planned
            .trace
            .iter()
            .find_map(|advice| match advice {
                Advice::KeyPartition(advice) if advice.name == "provider" => Some(advice),
                _ => None,
            })
            .unwrap();
        let producer = planned
            .trace
            .iter()
            .find_map(|advice| match advice {
                Advice::Publish { name, producer } if *name == partition.input => Some(producer),
                _ => None,
            })
            .expect("below-host raw producer needs a checked host publication");
        let raw_input = planned
            .program
            .sequence
            .definitions
            .iter()
            .find(|d| d.name == partition.input)
            .unwrap();
        assert_eq!(raw_input.term, syn::parse_quote!(store #producer));
        let producer_definition = planned
            .program
            .sequence
            .definitions
            .iter()
            .find(|d| d.name == *producer)
            .unwrap();
        assert!(!matches!(producer_definition.term, Term::Store { .. }));
        let facts = cq_lang_sequence::check_grading(&planned.physical, &planned.structures, &planned.program, &machine).unwrap();
        let task = vec!["host".to_owned(), "pool".to_owned(), "task".to_owned()];
        assert_eq!(facts.levels["task_pipe"], task);
        assert_eq!(facts.levels[&producer.to_string()], task);
        assert_eq!(
            facts.levels[&partition.input.to_string()],
            vec!["host".to_owned()]
        );
        replay(&physical, &structures, &source, &planned);
    }
}

fn single_worker_arms(program: &Program) -> &[(cq_lang_sequence::Guard, Term)] {
    let provider = program
        .sequence
        .definitions
        .iter()
        .find(|d| d.name == "provider")
        .unwrap();
    let Term::Store { inner, .. } = &provider.term else {
        panic!("provider Store required")
    };
    let Term::Choose(arms) = inner.as_ref() else {
        panic!("worker choice required")
    };
    assert_eq!(arms[0].0, cq_lang_sequence::Guard::WorkersLt(2));
    let Term::Choose(arms) = &arms[0].1 else {
        panic!("cache choice required")
    };
    arms
}

#[test]
fn filtered_provider_uses_host_layout_bound_before_demanding_input_count() {
    use cq_lang_sequence::{Guard, Len};
    let (mut physical, mut structures, source) = fixture! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L {a -> a};
            r = pipe rename R {a -> a,b -> b};
            selected = let select r if (b > a);
            joined = let canon probe first [a] l,selected;
            return joined as answer(a,b);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; index = Index { reads [exact], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @reader as Reader []; r host scan = @reader as Reader [];
            selected = @index as Index [a];
            joined rows = @sort as ::cq_storage::dedup::Sort; joined canon = rows;
        }
        sequence {
            r = store rows r;
            producer = compact selected flatten tile 64 r;
            provider = store producer;
            input = store rows l;
            result = store canon flatten lookup selected [a] input;
        }
    };
    let (routed_physical, routed_structures, routed) = key_partition(
        &physical, &structures, &source,
        quote!(provider, published, [a], 4, threshold 17, cache 31),
    )
    .unwrap();
    let arms = single_worker_arms(&routed);
    assert_eq!(arms.len(), 3);
    assert_eq!(arms[0].0, Guard::Lt(Len::Input("R".into()), 31));
    assert_eq!(arms[1].0, Guard::Lt(Len::Input("published".into()), 31));
    let provider = &routed
        .sequence
        .definitions
        .iter()
        .find(|d| d.name == "provider")
        .unwrap()
        .term;
    for (rows, host_rows, expected, counted) in
        [(17, 30, 1, false), (17, 31, 1, true), (31, 31, 4, true)]
    {
        let mut demanded = false;
        assert_eq!(
            selected_provider_parts(provider, 1, rows, host_rows, "published", &mut demanded),
            expected
        );
        assert_eq!(demanded, counted);
    }
    let planned = decompose(&physical, &structures, &source, &machine(Some(4)), &catalog()).unwrap();
    replay(&physical, &structures, &source, &planned);
}

#[test]
fn expanding_provider_join_requires_exact_count_without_a_host_shortcut() {
    use cq_lang_sequence::{Guard, Len};
    let (mut physical, mut structures, source) = fixture! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L {a -> a};
            r = let rename R {a -> a,b -> b};
            joined = let probe first [a] l,r;
            final_rows = let canon probe first [a] l,joined;
            return final_rows as answer(a,b);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; index = Index { reads [exact], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @reader as Reader []; r host scan = @reader as Reader [];
            r = @index as Index [a]; joined = @index as Index [a];
            final_rows rows = @sort as ::cq_storage::dedup::Sort; final_rows canon = rows;
        }
        sequence {
            right = store rows r;
            input = store rows l;
            provider = store flatten lookup r [a] input;
            result = store canon flatten lookup joined [a] input;
        }
    };
    let mut mapped = source.clone();
    mapped.sequence.definitions[2].term = syn::parse_quote!(store flatten map joined input);
    let mut merged = mapped.clone();
    let mut merged_physical = physical.clone();
    let mut merged_structures = structures.clone();
    merged_physical.program.definitions[2].operator = syn::parse_quote!(merge [a] l,r);
    merged_structures.structures
        .entries
        .push(syn::parse_quote!(l host result(joined) = @ordered as Ordered [a];));
    merged_structures.structures
        .declare(&[syn::parse_quote!(
            ordered = Ordered { reads [exact, range, membership, ordered], build inferred };
        )])
        .unwrap();
    for (physical, structures, source) in [
        (physical.clone(), structures.clone(), source),
        (physical, structures, mapped),
        (merged_physical, merged_structures, merged),
    ] {
        let (routed_physical, routed_structures, routed) = key_partition(
            &physical, &structures, &source,
            quote!(provider, published, [a], 4, threshold 17, cache 31),
        )
        .unwrap();
        let arms = single_worker_arms(&routed);
        assert_eq!(arms.len(), 2);
        assert_eq!(arms[0].0, Guard::Lt(Len::Input("published".into()), 31));
        assert_eq!(arms[1].0, Guard::Else);
    }
}

#[test]
fn host_bound_omits_existing_and_new_sequence_name_collisions() {
    use cq_lang_sequence::{Guard, Len};
    let (mut physical, mut structures, mut source) = provider_program();
    source
        .sequence
        .definitions
        .insert(0, syn::parse_quote!(R = store rows l;));
    let (routed_physical, routed_structures, routed) = key_partition(
        &physical, &structures, &source,
        quote!(provider, published, [a], 4, threshold 17, cache 31),
    )
    .unwrap();
    let arms = single_worker_arms(&routed);
    assert_eq!(arms.len(), 2);
    assert_eq!(arms[0].0, Guard::Lt(Len::Input("published".into()), 31));
    let (physical, structures, source) = provider_program();
    let (routed_physical, routed_structures, routed) = key_partition(
        &physical, &structures, &source,
        quote!(provider, R, [a], 4, threshold 17, cache 31),
    )
    .unwrap();
    let arms = single_worker_arms(&routed);
    assert_eq!(arms.len(), 2);
    assert_eq!(arms[0].0, Guard::Lt(Len::Input("R".into()), 31));
}

#[test]
fn support_equal_duplicate_rows_never_supply_a_host_cardinality_bound() {
    use cq_lang_sequence::{Guard, Len};
    let (mut physical, mut structures, mut source) = provider_program();
    physical
        .program
        .definitions
        .insert(2, syn::parse_quote!(duplicates = let union r,r;));
    physical.program.definitions[3] =
        syn::parse_quote!(first = pipe probe first [a] l,r;);
    physical
        .program
        .definitions
        .push(syn::parse_quote!(joined = let canon probe first [a,b] first,duplicates;));
    structures.structures
        .entries
        .push(syn::parse_quote!(r rows = @rows as Rows;));
    structures.structures
        .entries
        .push(syn::parse_quote!(duplicates = @index as Index [a,b];));
    structures.structures
        .declare(&[syn::parse_quote!(
            rows = Rows { reads [], build inferred, accounting rows };
        )])
        .unwrap();
    source.sequence.definitions[0].term = syn::parse_quote!(store r);
    source
        .sequence
        .definitions
        .insert(0, syn::parse_quote!(r = store union [rows r, rows r];));
    source
        .sequence
        .definitions
        .insert(2, syn::parse_quote!(duplicates = store provider;));
    source.sequence.definitions.last_mut().unwrap().term = syn::parse_quote!(store canon flatten lookup duplicates [a,b]
            flatten lookup r [a] input);
    let mut replayed_rows = source.clone();
    replayed_rows.sequence.definitions[1].term = syn::parse_quote!(store rows r);
    for source in [source, replayed_rows] {
        let facts = cq_lang_sequence::contract::check(&physical, &structures, &source).unwrap();
        assert_eq!(
            facts.results["provider"], "r",
            "support equality is not an occurrence bound"
        );
        let (routed_physical, routed_structures, routed) = key_partition(
            &physical, &structures, &source,
            quote!(provider, published, [a], 4, threshold 17, cache 31),
        )
        .unwrap();
        let arms = single_worker_arms(&routed);
        assert_eq!(arms.len(), 2);
        assert_eq!(arms[0].0, Guard::Lt(Len::Input("published".into()), 31));
        assert_eq!(arms[1].0, Guard::Else);
    }
}

#[test]
fn automatic_probe_chain_preserves_filtered_shared_index_contracts() {
    let (physical, structures) = structure_fixture! {
        physical {
            relation P(a:i32); relation PS(a:i32,b:i32); relation S(b:i32,c:i32);
            relation N(c:i32,d:i32); relation L(o:i32,a:i32,b:i32,u:i32); relation O(o:i32,v:i32);
            r0 = pipe rename P {a -> a};
            r1 = let rename PS {a -> a,b -> b};
            r2 = pipe probe first [a] r0,r1;
            r3 = let rename S {b -> b,c -> c};
            r4 = pipe probe first [b] r2,r3;
            r5 = let rename N {c -> c,d -> d};
            r6 = pipe probe first [c] r4,r5;
            r7 = let rename L {o -> o,a -> a,b -> b,u -> u};
            r8 = pipe probe first [a,b] r6,r7;
            r9 = let rename O {o -> o,v -> v};
            r10 = pipe probe first [o] r8,r9;
            r11 = let canon project r10 keep {o,a,b};
            return r11 as answer(o,a,b);
        }
        structures { providers { reader = Reader { reads [exact, range, membership], build inferred }; index = Index { reads [exact], build inferred }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            r0 host scan = @reader as Reader []; r1 host scan = @reader as Reader []; r3 host scan = @reader as Reader [];
            r5 host scan = @reader as Reader []; r7 host scan = @reader as Reader []; r9 host scan = @reader as Reader [];
            r1 = @index as Index [a]; r3 = @index as Index [b];
            r5 = @index as Index [c] payload [c] constants {};
            r7 = @index as Index [a,b] payload [o,a,b] constants {};
            r9 = @index as Index [o] payload [o] constants {};
            r11 rows = @sort as ::cq_storage::dedup::Sort; r11 canon = rows;
        }
    };
    let mut available = catalog();
    index(&mut available).extend([Capability::Range, Capability::Membership]);
    let mut structures = structures;
    declare_as(&mut structures.structures, &available);
    let source = cq_sequence_lowering::compile(&physical, &structures).unwrap();
    let planned = decompose(&physical, &structures, &source, &machine(Some(4)), &available).unwrap();
    assert_eq!(
        planned.physical,
        physical
    );
    replay(&physical, &structures, &source, &planned);
}

#[test]
fn group_canon_removal_consumes_recorded_valuation_uniqueness() {
    for named in [false, true] {
        let (physical, structures, mut source) = if named {
            named_grouped(true)
        } else {
            grouped(true)
        };
        let absent = cq_lang_sequence::contract::check(&physical, &structures, &source).unwrap();
        assert!(!absent.canon_free_groups.contains("result"));
        let advice = Advice::GroupWithoutCanon(syn::parse_quote!(result));
        assert!(advice.apply(&physical, &structures, &source).is_err());
        // Either declared key is an independent legal proof. The checker does
        // not require the producer's preferred key or canonicalizer spelling.
        for trusted_key in [
            quote!(evidence from authored("passes/cq-sequence-plan/tests/key_partition.rs fixture") { trusted_key Lineitem [orderkey, linenumber]; }),
            quote!(evidence from authored("passes/cq-sequence-plan/tests/key_partition.rs fixture") { trusted_key Lineitem [orderkey, linenumber, quantity]; }),
        ] {
            source.evidence = syn::parse2(trusted_key).unwrap();
            let facts = cq_lang_sequence::contract::check(&physical, &structures, &source).unwrap();
            assert!(facts.canon_free_groups.contains("result"));
            let (target_physical, target_structures, target) = advice.apply(&physical, &structures, &source).unwrap();
            cq_lang_sequence::check_rewrite(&physical, &structures, &source, &target_physical, &target_structures, &target).unwrap();
        }
    }
}

#[test]
fn prefix_bounds_keep_the_empty_global_group_identity() {
    for global in [false, true] {
        let (mut physical, mut structures, mut source) = grouped(global);
        source.evidence = syn::parse_quote!(evidence from authored("passes/cq-sequence-plan/tests/key_partition.rs fixture") { count Lineitem = 0; });
        let facts = cq_lang_sequence::contract::check(&physical, &structures, &source).unwrap();
        assert_eq!(facts.prefix_bounds["totals"].rows, Some(u128::from(global)));
        let mut forged = facts.prefix_bounds.clone();
        forged.get_mut("totals").unwrap().rows = Some(0);
        assert_eq!(
            cq_lang_sequence::bounds::check(&physical, &source, &forged).is_err(),
            global
        );
    }
}
