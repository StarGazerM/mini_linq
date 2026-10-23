use cq_lang_sequence::{
    Analysis, CanonToken, Capability, Catalog, CodecToken, LevelNode, Machine, OutputProtocol,
    Program, ProviderToken, check_grading, check_rewrite,
};
use cq_sequence_plan::{Advice, Planned, plan};
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
        name: syn::parse_str(name).unwrap(),
        collaborative,
        dynamic,
        width,
        children,
    }
}

fn machines() -> Vec<Machine> {
    vec![
        Machine {
            name: syn::parse_quote!(sequential),
            items: 1,
            root: node("host", false, false, None, vec![]),
        },
        Machine {
            name: syn::parse_quote!(tasks),
            items: 1,
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
            name: syn::parse_quote!(gpu),
            items: 4,
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
            ty: syn::parse_quote!(Canonical),
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
        structures { r host scan = Reader []; r canon = Canonical; }
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
            j canon = Canonical;
        }
        sequence {
            provider = store rows r;
            input = store rows l;
            result = store flatten lookup r [a] input;
        }
    }
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
fn pl_closure_replays_every_intermediate_rewrite_and_final_grading() {
    for machine in machines() {
        for source in [scan(quote!(input = rows r; result = input;)), join()] {
            let planned = plan(&source, &machine, &catalog(&machine)).unwrap();
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
            let first = plan(&source, &machine, &catalog).unwrap();
            for _ in 0..4 {
                assert_eq!(plan(&source, &machine, &catalog).unwrap(), first);
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
        let planned = plan(&source, &machine, &catalog(&machine)).unwrap();
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
        let planned = plan(&source, &machine, &catalog(&machine)).unwrap();
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
            let planned = plan(&source, &machine, &catalog(&machine)).unwrap();
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
        let planned = plan(&source, &machine, &catalog(&machine)).unwrap();
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
        let error = plan(&source, &machine, &catalog(&machine))
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
        plan(&source, &machine, &available)
            .expect("the complete local catalog discharges the lookup");
        available.providers.clear();
        let error = plan(&source, &machine, &available).unwrap_err().to_string();
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
        let error = plan(&source, &machine, &available).unwrap_err().to_string();
        assert!(error.contains("unmet catalog obligation Codec"), "{error}");
        assert!(error.contains("columns: [\"a\"]"), "{error}");
    }
}

#[test]
fn catalog_rejection_identifies_the_unmet_canon_obligation() {
    let source = scan(quote!(input = store rows r; result = store canon input;));
    for machine in machines() {
        let mut available = catalog(&machine);
        plan(&source, &machine, &available)
            .expect("the complete local catalog discharges canonicalization");
        available.canons.clear();
        let error = plan(&source, &machine, &available).unwrap_err().to_string();
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
        let planned = plan(&source, &machine, &available).unwrap();
        replay(&source, &machine, &planned);
    }
}

#[test]
fn catalog_provider_scope_is_the_read_level_not_the_host_store() {
    let machine = machines().pop().unwrap();
    let source = join();
    let mut available = catalog(&machine);
    available.providers[0].levels = vec![vec!["host".into()]];
    let error = plan(&source, &machine, &available).unwrap_err().to_string();
    assert!(
        error.contains("unmet catalog obligation Provider"),
        "{error}"
    );
    available.providers[0].levels = vec![vec!["host".into(), "device".into(), "block".into()]];
    let planned = plan(&source, &machine, &available).unwrap();
    replay(&source, &machine, &planned);
}

#[test]
fn host_views_are_not_materialized_before_an_existing_host_store() {
    for machine in machines() {
        let source = scan(quote! {
            input = rows r;
            retained = store input;
            result = store retained;
        });
        let planned = plan(&source, &machine, &catalog(&machine)).unwrap();
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
            joined rows = Index; answer canon = Canonical;
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
    let planned = plan(&source, &machine, &catalog(&machine)).unwrap();
    assert_eq!(
        planned,
        plan(&source, &machine, &catalog(&machine)).unwrap()
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
fn opaque_predicate_and_feeds_stay_host_before_total_lookup_dispatch() {
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
            answer canon = Canonical;
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
    let planned = plan(&source, &machine, &catalog(&machine)).unwrap();
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
fn string_predicate_and_numeric_projection_publish_at_host() {
    let source: Program = syn::parse_quote! {
        physical {
            relation R(a:i32,s:String);
            r = pipe rename R@0 {a -> a,s -> s};
            selected = pipe select r if (s = "green");
            numeric = pipe project selected keep {a};
            answer = pipe canon project numeric keep {a};
            return answer as answer(a);
        }
        structures { r host scan = Reader []; answer canon = Canonical; }
        sequence {
            input = rows r;
            selected = compact selected input;
            numeric = map numeric selected;
            result = store canon map answer numeric;
        }
    };
    let machine = machines().pop().unwrap();
    let planned = plan(&source, &machine, &catalog(&machine)).unwrap();
    let analysis = replay(&source, &machine, &planned);
    for name in ["input", "selected", "numeric"] {
        assert_eq!(analysis.levels[name], ["host"], "{name}");
    }
    assert!(
        analysis
            .dispatches
            .iter()
            .any(|dispatch| dispatch.outer_store == "numeric")
    );
}

#[test]
fn host_string_filter_uploads_projected_index_before_device_lookups() {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32); relation P(a:i32,s:String);
            p = pipe rename P@0 {a -> a,s -> s};
            selected = let select p if (#[total] {s = s} (s.contains("green")));
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
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let planned = plan(&source, &machine, &available).unwrap();
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
