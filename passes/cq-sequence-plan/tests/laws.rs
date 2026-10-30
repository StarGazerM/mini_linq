use cq_lang_sequence::{
    Analysis, CanonToken, Capability, Catalog, CodecToken, Guard, Len, LevelNode, Machine,
    OutputProtocol, Program, ProviderToken, Term, check_grading, check_rewrite,
};
use cq_sequence_plan::{Advice, Planned, decompose};
use proc_macro2::TokenStream;
use quote::quote;

// Target descriptions are local data: planner laws must not depend on target crates.
fn node(
    name: &str,
    collaborative: bool,
    dynamic: bool,
    width: Option<usize>,
    children: Vec<LevelNode>,
) -> LevelNode {
    LevelNode {
        shared_bytes: None,
        name: syn::parse_str(name).unwrap(),
        host_rust: matches!(name, "host" | "pool" | "task"),
        collaborative,
        dynamic,
        width,
        children,
    }
}

fn machines() -> Vec<Machine> {
    vec![
        Machine {
            gpu: None,
            layout: None,
            name: syn::parse_quote!(sequential),
            items: 1,
            key_partition_threshold: 65536,
            key_partition_cache_threshold: 6 * 1024 * 1024,
            root: node("host", false, false, None, vec![]),
        },
        Machine {
            gpu: None,
            layout: None,
            name: syn::parse_quote!(tasks),
            items: 1,
            key_partition_threshold: 65536,
            key_partition_cache_threshold: 6 * 1024 * 1024,
            root: node(
                "host",
                false,
                false,
                None,
                vec![node(
                    "pool",
                    false,
                    true,
                    Some(8),
                    vec![node("task", false, false, None, vec![])],
                )],
            ),
        },
        Machine {
            gpu: None,
            layout: None,
            name: syn::parse_quote!(gpu),
            items: 4,
            key_partition_threshold: 65536,
            key_partition_cache_threshold: 6 * 1024 * 1024,
            root: node(
                "host",
                false,
                false,
                None,
                vec![node(
                    "device",
                    false,
                    true,
                    None,
                    vec![node(
                        "block",
                        true,
                        false,
                        Some(32),
                        vec![node("lane", false, false, None, vec![])],
                    )],
                )],
            ),
        },
    ]
}

fn catalog(machine: &Machine) -> Catalog {
    fn paths(node: &LevelNode, mut path: Vec<String>, result: &mut Vec<Vec<String>>) {
        path.push(node.name.to_string());
        result.push(path.clone());
        for child in &node.children {
            paths(child, path.clone(), result);
        }
    }
    let mut levels = Vec::new();
    paths(&machine.root, Vec::new(), &mut levels);
    Catalog {
        providers: vec![ProviderToken {
            supports: Vec::new(),
            ty: syn::parse_quote!(Index),
            elements: None,
            capabilities: vec![
                Capability::Exact,
                Capability::Range,
                Capability::Membership,
                Capability::Ordered,
            ],
            levels: levels.clone(),
        }],
        codecs: vec![CodecToken {
            ty: syn::parse_quote!(Identity),
            ordered: true,
            levels: levels.clone(),
        }],
        canons: vec![CanonToken {
            ty: syn::parse_quote!(::cq_storage::dedup::Sort),
            levels,
            max_packed_bits: None,
        }],
    }
}

fn scan(sequence: TokenStream) -> Program {
    syn::parse2(quote! {
        physical {
            relation R(a:i32);
            r = pipe canon rename R@0 {a -> a};
            return r as answer(a);
        }
        structures { r host scan = Reader []; r canon = sort; }
        sequence { #sequence }
    })
    .unwrap()
}

fn join() -> Program {
    syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            j = pipe canon probe first [a] l,r;
            return j as answer(a,b);
        }
        structures {
            l host scan = Reader [];
            r host scan = Reader [];
            r = Index [a];
            j canon = sort;
        }
        sequence {
            provider = store rows r;
            input = store rows l;
            result = store flatten lookup r [a] input;
        }
    }
}

fn filtered_multi_join() -> Program {
    let structures = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,rtag:i32); relation S(a:i32,stag:i32);
            relation Q(a:i32,qtag:i32);
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {a -> a,rtag -> rtag};
            s = pipe rename S@2 {a -> a,stag -> stag};
            q = pipe rename Q@3 {a -> a,qtag -> qtag};
            qf = let point select q if (qtag = 5);
            rf = let point select r if (rtag = 7);
            sf = let point select s if (stag = 9);
            jq = pipe probe first [a] l,qf;
            jr = pipe probe first [a] jq,rf;
            joined = pipe probe first [a] jr,sf;
            result = pipe canon project joined keep {a};
            return result as answer(a);
        }
        structures {
            l host scan = Reader []; r host scan = Reader []; s host scan = Reader [];
            q host scan = Reader []; q host result(qf) = Reader [qtag]; qf = Index [a];
            r host result(rf) = Reader [rtag]; s host result(sf) = Reader [stag];
            rf = Index [a]; sf = Index [a]; result canon = sort;
        }
    };
    cq_sequence_lowering::compile(&structures).unwrap()
}

fn reorderable_multi_join() -> Program {
    let structures = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32); relation S(a:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a};
            s = let rename S@2 {a -> a};
            jr = pipe probe first [a] l,r;
            result = pipe canon probe first [a] jr,s;
            return result as answer(a);
        }
        structures {
            l host scan = Reader []; r host scan = Reader []; s host scan = Reader [];
            r = Index [a]; s = Index [a]; result canon = sort;
        }
    };
    cq_sequence_lowering::compile(&structures).unwrap()
}

fn replay(source: &Program, machine: &Machine, planned: &Planned) -> Analysis {
    let mut current = source.clone();
    for (index, advice) in planned.trace.iter().enumerate() {
        let next = advice
            .apply(&current)
            .unwrap_or_else(|error| panic!("trace step {index} {advice:?}: {error}"));
        check_rewrite(&current, &next)
            .unwrap_or_else(|error| panic!("rewrite law at step {index} {advice:?}: {error}"));
        current = next;
    }
    assert_eq!(
        current, planned.program,
        "trace must reconstruct the exact returned syntax"
    );
    check_rewrite(source, &current).expect("the complete trace preserves the source");
    check_grading(&current, machine).expect("the final replay must satisfy machine grading")
}

#[test]
fn decomposition_preserves_join_order_with_count_evidence() {
    let source = reorderable_multi_join();
    let mut estimated = source.clone();
    estimated.evidence = syn::parse_quote!(evidence {
        count L = 1; count R = 10; count S = 900;
    });
    for machine in machines() {
        let available = catalog(&machine);
        let ordinary = decompose(&source, &machine, &available).unwrap();
        let advised = decompose(&estimated, &machine, &available).unwrap();
        assert_eq!(
            ordinary.program.structures.physical,
            source.structures.physical
        );
        assert_eq!(
            advised.program.structures.physical,
            source.structures.physical
        );
        assert_eq!(advised.program.evidence, estimated.evidence);
        replay(&source, &machine, &ordinary);
        replay(&estimated, &machine, &advised);
        assert_eq!(
            decompose(&estimated, &machine, &available).unwrap(),
            advised
        );
    }
}

#[test]
fn decomposition_preserves_authored_sequence_and_join_order() {
    let source = filtered_multi_join();
    let authored = Advice::Store(syn::parse_quote!(jr)).apply(&source).unwrap();
    for machine in machines() {
        let available = catalog(&machine);
        let planned = decompose(&authored, &machine, &available).unwrap();
        assert_eq!(
            planned.program.structures.physical,
            authored.structures.physical
        );
        assert!(matches!(
            planned
                .program
                .sequence
                .definitions
                .iter()
                .find(|definition| definition.name == "jr")
                .unwrap()
                .term,
            Term::Store { .. }
        ));
        replay(&authored, &machine, &planned);

        let decomposed = decompose(&source, &machine, &available).unwrap();
        assert_eq!(
            decomposed.program.structures.physical,
            source.structures.physical
        );
        replay(&source, &machine, &decomposed);
    }
}

#[test]
fn pl_closure_replays_every_intermediate_rewrite_and_final_grading() {
    for machine in machines() {
        for source in [scan(quote!(input = rows r; result = input;)), join()] {
            let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
            assert!(
                !planned.trace.is_empty(),
                "unauthored scheduling needs advice"
            );
            let analysis = replay(&source, &machine, &planned);
            assert_eq!(
                analysis.levels.len(),
                planned.program.sequence.definitions.len()
            );
            assert!(
                analysis.output.is_some(),
                "planning must complete the output protocol"
            );
        }
    }
}

#[test]
fn pl_det_repeated_inputs_produce_identical_programs_and_traces() {
    for machine in machines() {
        let catalog = catalog(&machine);
        for source in [scan(quote!(input = rows r; result = input;)), join()] {
            let before = source.clone();
            let first = decompose(&source, &machine, &catalog).unwrap();
            for _ in 0..4 {
                assert_eq!(decompose(&source, &machine, &catalog).unwrap(), first);
            }
            assert_eq!(source, before, "planning must not mutate its input");
        }
    }
}

#[test]
fn pl_precedence_keeps_authored_grades_while_filling_missing_grades() {
    for machine in machines() {
        let mut source = scan(quote!(input = store rows r; result = store input;));
        let machine_name = &machine.name;
        source.graded =
            Some(syn::parse2(quote!(graded #machine_name { result at host; })).unwrap());
        let authored = source.graded.as_ref().unwrap().grades[0].clone();
        let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
        let grades = &planned.program.graded.as_ref().unwrap().grades;
        assert_eq!(
            grades
                .iter()
                .filter(|grade| grade.name == authored.name)
                .collect::<Vec<_>>(),
            vec![&authored]
        );
        assert!(grades.iter().any(|grade| grade.name == "input"));
        assert!(
            !planned
                .trace
                .iter()
                .any(|advice| matches!(advice, Advice::Grade { name, .. } if name == "result"))
        );
        replay(&source, &machine, &planned);
    }
}

#[test]
fn pl_precedence_keeps_authored_tile_width_and_items() {
    for machine in machines() {
        let source = scan(quote! {
            input = store rows r;
            result = store tile 7 items 3 input;
        });
        let authored = source.sequence.definitions[1].term.clone();
        let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
        assert_eq!(planned.program.sequence.definitions[1].term, authored);
        assert!(!planned.trace.iter().any(|advice| matches!(advice,
            Advice::Tile { name, .. } | Advice::Partition { name, .. } | Advice::Segment { name, .. }
                if name == "result"
        )));
        replay(&source, &machine, &planned);
    }
}

#[test]
fn pl_precedence_keeps_each_authored_output_protocol_and_capacity() {
    for machine in machines() {
        for protocol in [
            OutputProtocol::Count,
            OutputProtocol::Reserve,
            OutputProtocol::Bounded,
        ] {
            let mut source = scan(quote! {
                input = store rows r;
                result = store capacity 123 input;
            });
            source.sequence.output = Some(protocol.clone());
            let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
            assert_eq!(planned.program.sequence.output, Some(protocol));
            assert!(matches!(
                planned.program.sequence.definitions.last().unwrap().term,
                cq_lang_sequence::Term::Store {
                    capacity: Some(123),
                    ..
                }
            ));
            assert!(
                !planned
                    .trace
                    .iter()
                    .any(|advice| matches!(advice, Advice::Output(_)))
            );
            replay(&source, &machine, &planned);
        }
    }
}

#[test]
fn pl_precedence_fully_authored_schedule_is_an_exact_fixed_point() {
    for machine in machines() {
        let mut source = scan(quote! {
            input = store rows r;
            result = store tile 7 items 3 input;
            output reserve;
        });
        let machine_name = &machine.name;
        let publication = machine
            .root
            .children
            .first()
            .map(|level| &level.name)
            .unwrap_or(&machine.root.name);
        source.graded = Some(
            syn::parse2(quote! {
                graded #machine_name { input at host; result at #publication; }
            })
            .unwrap(),
        );
        let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
        assert_eq!(planned.program, source);
        assert!(
            planned.trace.is_empty(),
            "fully authored decisions need no replacement advice"
        );
        replay(&source, &machine, &planned);
    }
}

#[test]
fn pl_precedence_rejects_replacing_an_authored_machine() {
    for machine in machines() {
        let mut source = scan(quote!(result = store rows r;));
        source.graded = Some(syn::parse_quote!(graded other_machine { result at host; }));
        let error = decompose(&source, &machine, &catalog(&machine))
            .unwrap_err()
            .to_string();
        assert!(error.contains("PL-Precedence"), "{error}");
        assert!(error.contains("authored machine"), "{error}");
    }
}

#[test]
fn catalog_rejection_identifies_the_unmet_provider_obligation() {
    let source = join();
    for machine in machines() {
        let mut available = catalog(&machine);
        decompose(&source, &machine, &available)
            .expect("the complete local catalog discharges the lookup");
        available.providers.clear();
        let error = decompose(&source, &machine, &available)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("unmet catalog obligation Provider"),
            "{error}"
        );
        assert!(error.contains("provider: \"r\""), "{error}");
        assert!(error.contains("keys: [\"a\"]"), "{error}");
    }
}

#[test]
fn catalog_rejection_identifies_the_unmet_codec_obligation() {
    let source = join();
    for machine in machines() {
        let mut available = catalog(&machine);
        available.codecs.clear();
        let error = decompose(&source, &machine, &available)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unmet catalog obligation Codec"), "{error}");
        assert!(error.contains("columns: [\"a\"]"), "{error}");
    }
}

#[test]
fn catalog_rejection_identifies_the_unmet_canon_obligation() {
    let source = scan(quote!(input = store rows r; result = store canon input;));
    for machine in machines() {
        let mut available = catalog(&machine);
        decompose(&source, &machine, &available)
            .expect("the complete local catalog discharges canonicalization");
        available.canons.clear();
        let error = decompose(&source, &machine, &available)
            .unwrap_err()
            .to_string();
        assert!(error.contains("unmet catalog obligation Canon"), "{error}");
        assert!(error.contains("result: \"result\""), "{error}");
    }
}

#[test]
fn no_provider_catalog_still_supports_a_simple_scan() {
    let source = scan(quote!(input = rows r; result = input;));
    for machine in machines() {
        let mut available = catalog(&machine);
        available.providers.clear();
        available.canons.clear();
        assert_eq!(available.codecs[0].ty, syn::parse_quote!(Identity));
        let planned = decompose(&source, &machine, &available).unwrap();
        replay(&source, &machine, &planned);
    }
}

#[test]
fn catalog_provider_scope_is_the_read_level_not_the_host_store() {
    let machine = machines().pop().unwrap();
    let source = join();
    let mut available = catalog(&machine);
    available.providers[0].levels = vec![vec!["host".into()]];
    let planned = decompose(&source, &machine, &available).unwrap();
    let analysis = replay(&source, &machine, &planned);
    assert_eq!(analysis.levels["result"], ["host"]);
    // An unauthored read can move to host; an explicitly authored descendant
    // read remains a catalog error rather than silently changing its grade.
    let authored =
        cq_sequence_plan::rewrites::grade(&source, quote!(gpu, result at device)).unwrap();
    let error = decompose(&authored, &machine, &available)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("unmet catalog obligation Provider"),
        "{error}"
    );
    available.providers[0].levels = vec![vec!["host".into(), "device".into(), "block".into()]];
    let planned = decompose(&source, &machine, &available).unwrap();
    let analysis = replay(&source, &machine, &planned);
    assert_eq!(analysis.levels["provider"], ["host"]);
    assert!(!analysis.dispatches.is_empty());
}

#[test]
fn host_views_are_not_materialized_before_an_existing_host_store() {
    for machine in machines() {
        let source = scan(quote! {
            input = rows r;
            retained = store input;
            result = store retained;
        });
        let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
        assert!(matches!(
            planned.program.sequence.definitions[0].term,
            cq_lang_sequence::Term::Rows(_)
        ));
        assert!(
            !planned
                .trace
                .iter()
                .any(|advice| matches!(advice, Advice::Store(name) if name == "input"))
        );
        replay(&source, &machine, &planned);
    }
}

#[test]
fn distinct_union_drivers_publish_in_deterministic_separate_dispatches() {
    let source: Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32); relation P(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {a -> a};
            p = let rename P@2 {a -> a,b -> b};
            left = pipe probe first [a] l,p;
            right = pipe probe first [a] r,p;
            joined = let union left,right;
            answer = pipe canon project joined keep {a,b};
            return answer as answer(a,b);
        }
        structures {
            l host scan = Reader []; r host scan = Reader [];
            p host scan = Reader []; p = Index [a];
            joined rows = Index; answer canon = sort;
        }
        sequence {
            l = store rows l; r = store rows r; p = store rows p;
            left = flatten lookup p [a] l;
            right = flatten lookup p [a] r;
            joined = store union [left,right];
            result = store canon map answer joined;
        }
    };
    let machine = machines().pop().unwrap();
    let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
    assert_eq!(
        planned,
        decompose(&source, &machine, &catalog(&machine)).unwrap()
    );
    let analysis = replay(&source, &machine, &planned);
    assert_eq!(analysis.levels["joined"], ["host"]);
    assert_eq!(analysis.levels["left"], ["host"]);
    assert_eq!(analysis.levels["right"], ["host"]);
    for (arm, driver) in [("left", "l"), ("right", "r")] {
        assert!(
            planned
                .trace
                .iter()
                .any(|advice| matches!(advice, Advice::Publish { name, .. } if name == arm))
        );
        let dispatch = analysis
            .dispatches
            .iter()
            .find(|dispatch| dispatch.inner_stores.iter().any(|name| name == arm))
            .unwrap();
        assert_eq!(dispatch.outer_store, driver);
        assert!(!dispatch.members.iter().any(|name| name == "joined"));
    }
}

#[test]
fn rust_without_identity_facts_and_its_feeds_stay_host() {
    let source: Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32); relation S(b:i32,c:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            joined = pipe probe first [a] l,r;
            selected = pipe guard joined if (opaque(b));
            s = let rename S@2 {b -> b,c -> c};
            after = pipe probe first [b] selected,s;
            answer = pipe canon project after keep {a,b,c};
            return answer as answer(a,b,c);
        }
        structures {
            l host scan = Reader []; r host scan = Reader [];
            s host scan = Reader []; r = Index [a]; s = Index [b];
            answer canon = sort;
        }
        sequence {
            l = rows l; r = store rows r;
            joined = flatten lookup r [a] l;
            selected = map selected joined;
            s = store rows s;
            after = flatten lookup s [b] selected;
            result = store canon map answer after;
        }
    };
    let machine = machines().pop().unwrap();
    let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
    let analysis = replay(&source, &machine, &planned);
    for name in ["l", "joined", "selected"] {
        assert_eq!(analysis.levels[name], ["host"], "{name}");
    }
    assert!(
        planned
            .trace
            .iter()
            .any(|advice| matches!(advice, Advice::Store(name) if name == "selected"))
    );
    assert!(
        analysis
            .dispatches
            .iter()
            .any(|dispatch| dispatch.outer_store == "selected")
    );
}

#[test]
fn relational_string_predicates_do_not_imply_host_placement() {
    let source: Program = syn::parse_quote! {
        physical {
            relation R(a:i32,s:String);
            r = pipe rename R@0 {a -> a,s -> s};
            selected = pipe select r if (s = "green");
            numeric = pipe project selected keep {a};
            answer = pipe canon project numeric keep {a};
            return answer as answer(a);
        }
        structures { r host scan = Reader []; answer canon = sort; }
        sequence {
            input = rows r;
            selected = compact selected input;
            numeric = map numeric selected;
            result = store canon map answer numeric;
        }
    };
    let machine = machines().pop().unwrap();
    let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
    let analysis = replay(&source, &machine, &planned);
    assert!(analysis.levels["selected"].len() > 1);
    assert!(analysis.levels["numeric"].len() > 1);
}

#[test]
fn host_string_filter_uploads_projected_index_before_device_lookups() {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32); relation P(a:i32,s:String);
            p = pipe rename P@0 {a -> a,s -> s};
            filtered = pipe guard p if (s.contains("green"));
            selected = let project filtered keep {a};
            l = pipe rename L@1 {a -> a};
            joined = pipe probe first [a] l,selected;
            answer = pipe canon project joined keep {a};
            return answer as answer(a);
        }
    };
    let machine = machines().pop().unwrap();
    let mut available = catalog(&machine);
    available.providers[0].elements = Some(vec![syn::parse_quote!(i32)]);
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let structures = cq_sequence_plan::select_from_catalog(&structures, &available).unwrap();
    let facts = cq_lang_sequence::structures::contract::check(&structures).unwrap();
    assert_eq!(facts.layouts["selected"], ["a"]);
    let mut source = cq_sequence_lowering::compile(&structures).unwrap();
    // The target records the projected key's device representation; the
    // unrepresented string payload remains available only to the host guard.
    source.representation.identity.insert("a".into());
    let planned = decompose(&source, &machine, &available).unwrap();
    let analysis = replay(&source, &machine, &planned);
    assert_eq!(analysis.levels["__sequence_host_selected"], ["host"]);
    assert_eq!(analysis.levels["selected"], ["host", "device"]);
    assert_eq!(analysis.levels["joined"], ["host", "device", "block"]);
    assert!(
        analysis
            .dispatches
            .iter()
            .all(|dispatch| !dispatch.members.iter().any(|name| name == "selected"))
    );
    assert!(
        analysis
            .dispatches
            .iter()
            .any(|dispatch| dispatch.members.iter().any(|name| name == "joined"))
    );
}

#[test]
fn automatic_h09_keeps_retained_dependencies_in_their_read_extent() {
    // The GPU suite imports this shape: Part's opaque predicate is written
    // before Orders and the composite Lineitem/PartSupp join, not moved there.
    let physical = syn::parse_quote! {
        physical {
            relation Part(part_key:i32,part_name:String);
            relation Orders(order_key:i32);
            relation Lineitem(order_key:i32,part_key:i32,supplier_key:i32);
            relation PartSupp(part_key:i32,supplier_key:i32);
            relation Supplier(supplier_key:i32,nation_key:i32);
            relation Nation(nation_key:i32);
            p = pipe rename Part@0 {part_key -> part_key,part_name -> part_name};
            green = pipe guard p if (part_name.contains("green"));
            o = let rename Orders@2 {order_key -> order_key};
            po = pipe probe first [] green,o;
            l = let rename Lineitem@3 {
                order_key -> order_key,part_key -> part_key,supplier_key -> supplier_key
            };
            pol = pipe probe first [order_key,part_key] po,l;
            ps = let rename PartSupp@4 {part_key -> part_key,supplier_key -> supplier_key};
            valid = pipe probe first [part_key,supplier_key] pol,ps;
            s = let rename Supplier@5 {supplier_key -> supplier_key,nation_key -> nation_key};
            supplied = pipe probe first [supplier_key] valid,s;
            n = let rename Nation@6 {nation_key -> nation_key};
            joined = pipe probe first [nation_key] supplied,n;
            answer = pipe canon project joined keep {order_key,part_key,supplier_key};
            return answer as answer(order_key,part_key,supplier_key);
        }
    };
    let machine = machines().pop().unwrap();
    let mut available = catalog(&machine);
    available.providers[0].elements = Some(vec![syn::parse_quote!(i32)]);
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let structures = cq_sequence_plan::select_from_catalog(&structures, &available).unwrap();
    let mut source = cq_sequence_lowering::compile(&structures).unwrap();
    source.evidence = syn::parse_quote!(evidence {
        count Part = 200000; count Orders = 1500000; count Lineitem = 6001215;
        count PartSupp = 800000; count Supplier = 10000; count Nation = 25;
    });
    let planned = decompose(&source, &machine, &available).unwrap();
    assert_eq!(
        planned.program.structures.physical,
        source.structures.physical
    );
    let analysis = replay(&source, &machine, &planned);
    assert_eq!(analysis.levels["p"], ["host"]);
    assert_eq!(analysis.levels["green"], ["host"]);
    assert!(!analysis.dispatches.is_empty());
}

#[test]
fn host_predicate_publishes_a_retained_lookup_without_moving_its_producer() {
    let source: Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            joined = let probe first [a] l,r;
            selected = pipe guard joined if (opaque(b));
            answer = pipe canon project selected keep {a,b};
            return answer as answer(a,b);
        }
        structures {
            l host scan = Reader []; r host scan = Reader [];
            r = Index [a]; joined rows = Index; answer canon = sort;
        }
        sequence {
            l = rows l; r = store rows r;
            joined = store flatten lookup r [a] l;
            selected = map selected joined;
            result = store canon map answer selected;
        }
    };
    let machine = machines().pop().unwrap();
    let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
    let analysis = replay(&source, &machine, &planned);
    assert_eq!(analysis.levels["joined"], ["host"]);
    assert_eq!(analysis.levels["selected"], ["host"]);
    assert_eq!(
        analysis.levels["__sequence_publish_joined"],
        ["host", "device", "block"]
    );
    assert!(
        planned
            .trace
            .iter()
            .any(|advice| matches!(advice, Advice::Publish { name, .. } if name == "joined"))
    );
    assert!(
        planned
            .trace
            .iter()
            .any(|advice| matches!(advice, Advice::Store(name) if name == "l"))
    );
}

#[test]
fn host_choice_publishes_counts_even_when_arms_do_not_read_the_counted_rows() {
    let machine = machines().pop().unwrap();
    for length in [
        Len::Input("counted".into()),
        Len::Add(
            Box::new(Len::Input("counted".into())),
            Box::new(Len::Add(
                Box::new(Len::Input("counted".into())),
                Box::new(Len::Lit(1)),
            )),
        ),
    ] {
        let mut source = join();
        let mut counted = source.sequence.definitions.last().unwrap().clone();
        counted.name = syn::parse_quote!(counted);
        source.sequence.definitions.insert(2, counted);
        let Term::Store { inner, .. } = &mut source.sequence.definitions.last_mut().unwrap().term
        else {
            unreachable!()
        };
        **inner = Term::Choose(vec![
            (Guard::Lt(length, 10), *inner.clone()),
            (Guard::Else, *inner.clone()),
        ]);
        source.graded = Some(syn::parse_quote!(graded gpu { result at host; }));
        let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
        let analysis = replay(&source, &machine, &planned);
        assert_eq!(analysis.levels["result"], ["host"]);
        assert_eq!(analysis.levels["counted"], ["host"]);
        assert_eq!(
            analysis.levels["__sequence_publish_counted"],
            ["host", "device", "block"]
        );
        assert!(
            planned
                .trace
                .iter()
                .any(|advice| matches!(advice, Advice::Publish { name, .. } if name == "counted"))
        );
    }
}

#[test]
fn retained_row_comparisons_publish_before_provider_lookup() {
    let machine = machines().pop().unwrap();
    for (condition, row_bound) in [
        (quote!(value < bound), true),
        (quote!(value = bound), true),
        (quote!(value < 10), false),
    ] {
        let source: Program = syn::parse2(quote! {
            physical {
                relation Driver(a:i32);
                relation Provider(a:i32,value:i32,bound:i32);
                driver = pipe rename Driver@0 {a -> a};
                provider = pipe rename Provider@1 {a -> a,value -> value,bound -> bound};
                selected = let select provider if (#condition);
                present = pipe semi eager [a] driver,selected;
                answer = pipe canon project present keep {a};
                return answer as answer(a);
            }
            structures {
                driver host scan = Reader [];
                provider host scan = Reader [];
                selected = Index [a];
                answer canon = sort;
            }
            sequence {
                input = rows driver;
                provider_input = rows provider;
                index = store compact selected provider_input;
                present = compact present lookup selected [a] input;
                result = store canon map answer present;
            }
        })
        .unwrap();
        let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
        let analysis = replay(&source, &machine, &planned);
        assert_eq!(analysis.levels["provider_input"], ["host"]);
        if row_bound {
            assert_eq!(analysis.levels["index"], ["host", "device"]);
            // outer_store names the input domain; the retained result is a
            // publication in inner_stores, not the dispatch's source.
            assert!(
                analysis
                    .dispatches
                    .iter()
                    .any(|dispatch| { dispatch.inner_stores.iter().any(|store| store == "index") })
            );
            assert!(
                planned
                    .trace
                    .iter()
                    .any(|advice| matches!(advice, Advice::Grade { name, level, .. }
                    if name == "index" && level.len() == 2))
            );
        } else {
            assert_eq!(analysis.levels["index"], ["host"]);
        }
    }
}

fn expanding_resource_fixture() -> Program {
    syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            relation S(b:i32,c:i32); relation T(c:i32,d:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            s = let rename S@2 {b -> b,c -> c};
            t = let rename T@3 {c -> c,d -> d};
            j = pipe probe first [a] l,r;
            k = pipe probe first [b] j,s;
            m = pipe canon probe first [c] k,t;
            return m as answer(a,b,c,d);
        } structures {
            l host scan = Reader []; r host scan = Reader [];
            s host scan = Reader []; t host scan = Reader [];
            r = Index [a]; s = Index [b]; t = Index [c];
            m canon = sort;
        } sequence {
            l = rows l; r = store rows r; s = store rows s; t = store rows t;
            j = flatten lookup r [a] l;
            k = flatten lookup s [b] j;
            m = canon flatten lookup t [c] k;
            out = store m;
        }
    }
}

#[test]
fn resource_bounded_planning_separates_expanding_stages_with_checked_stores() {
    let source = expanding_resource_fixture();
    let mut machine = machines().pop().unwrap();
    machine.root.children[0].children[0].shared_bytes = Some(1);
    machine.layout = Some(|shape| {
        Ok(shape
            .chains
            .iter()
            .flatten()
            .filter(|allocation| {
                matches!(
                    allocation,
                    cq_lang_sequence::ResourceAllocation::Payload { .. }
                )
            })
            .count())
    });
    let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
    let analysis = replay(&source, &machine, &planned);
    assert!(
        planned
            .trace
            .iter()
            .any(|advice| matches!(advice, Advice::StoreLevel(name) if name == "j" || name == "k"))
    );
    assert!(!analysis.resources.is_empty());
    assert!(
        analysis
            .resources
            .values()
            .all(|resource| resource.required_bytes <= 1)
    );
}

#[test]
fn resource_recovery_prefers_smaller_items_before_smaller_tiles() {
    let source = expanding_resource_fixture();
    let mut machine = machines().pop().unwrap();
    machine.root.children[0].children[0].shared_bytes = Some(1);
    machine.layout = Some(|shape| Ok(shape.items));
    let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
    let analysis = replay(&source, &machine, &planned);
    assert!(
        analysis
            .resources
            .values()
            .all(|resource| resource.shape.items == 1)
    );
    assert!(planned.trace.iter().any(|step| matches!(
        step,
        Advice::Partition {
            n: 32,
            items: None,
            ..
        }
    )));
    assert!(
        !planned
            .trace
            .iter()
            .any(|step| matches!(step, Advice::StoreLevel(_)))
    );
}

#[test]
fn resource_recovery_uses_smaller_machine_derived_tiles() {
    let source = expanding_resource_fixture();
    let mut machine = machines().pop().unwrap();
    machine.root.children[0].children[0].shared_bytes = Some(16);
    machine.layout = Some(|shape| Ok(shape.tile.unwrap_or(1)));
    let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
    replay(&source, &machine, &planned);
    assert!(planned.trace.iter().any(|step| matches!(
        step,
        Advice::Partition {
            n: 16,
            items: None,
            ..
        }
    )));
    assert!(
        !planned
            .trace
            .iter()
            .any(|step| matches!(step, Advice::StoreLevel(_)))
    );
}

#[test]
fn resource_rejection_reports_every_candidate_and_nested_diagnostic() {
    let source = expanding_resource_fixture();
    let mut machine = machines().pop().unwrap();
    machine.root.children[0].children[0].shared_bytes = Some(1);
    machine.layout = Some(|_| {
        let mut errors = syn::Error::new(proc_macro2::Span::call_site(), "first layout failure");
        errors.combine(syn::Error::new(
            proc_macro2::Span::call_site(),
            "second layout failure",
        ));
        Err(errors)
    });
    let errors = decompose(&source, &machine, &catalog(&machine))
        .unwrap_err()
        .into_iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(errors[0].contains("stage=0 tile=Some(32) items=Some(4)"));
    assert!(errors[1].contains("second layout failure"));
    assert!(
        errors
            .iter()
            .any(|error| error.contains("stage=0 tile=Some(32) items=Some(1)"))
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("stage=0 tile=Some(16) items=Some(1)"))
    );
    assert!(errors.iter().any(|error| error.contains("stage=1")));
    assert!(errors.iter().any(|error| error.contains("stage=2")));
}

#[test]
fn resource_recovery_splits_only_one_boundary_when_two_dispatches_fit() {
    let source = expanding_resource_fixture();
    let mut machine = machines().pop().unwrap();
    machine.root.children[0].children[0].shared_bytes = Some(2);
    machine.layout = Some(|shape| {
        Ok(shape
            .chains
            .iter()
            .flatten()
            .filter(|allocation| {
                matches!(
                    allocation,
                    cq_lang_sequence::ResourceAllocation::Payload { .. }
                )
            })
            .count())
    });
    let planned = decompose(&source, &machine, &catalog(&machine)).unwrap();
    let analysis = replay(&source, &machine, &planned);
    assert_eq!(
        planned
            .trace
            .iter()
            .filter(|step| matches!(step, Advice::StoreLevel(_)))
            .count(),
        1
    );
    assert!(
        analysis
            .resources
            .values()
            .all(|usage| usage.required_bytes <= 2)
    );
}
