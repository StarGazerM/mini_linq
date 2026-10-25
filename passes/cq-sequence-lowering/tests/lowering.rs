use cq_lang_sequence::{Capability, LevelNode, Machine, OutputProtocol, Program, Term};
use cq_planners_plan::Planner;
use cq_sequence_lowering::{check, compile};

fn accept(source: &cq_lang_structures::Program) -> Program {
    let target = compile(source).unwrap();
    assert!(target.graded.is_none());
    assert_eq!(target.sequence.output, Some(OutputProtocol::Count));
    cq_lang_sequence::contract::check(&target).unwrap();
    cq_lang_sequence::check_grading(
        &target,
        &Machine {
            layout: None,
            name: syn::parse_quote!(sequential),
            items: 1,
            key_partition_threshold: 65536,
            root: LevelNode {
                shared_bytes: None,
                name: syn::parse_quote!(host),
                collaborative: false,
                dynamic: false,
                width: None,
                children: vec![],
            },
        },
    )
    .unwrap();
    check(source, &target).unwrap();
    target
}

fn lower_cq(source: &cq_testkit::cq::Program, label: &str) {
    for planner in Planner::ALL {
        let result = (|| -> syn::Result<_> {
            let ra = cq_planners_plan::compile(source, planner)?;
            let ix = cq_index_policy::compile(&ra)?;
            let physical = cq_physical_lowering::compile(&ix)?;
            cq_structure_selection::compile(&physical)
        })()
        .unwrap_or_else(|error| panic!("{label} {planner:?}: {error}"));
        accept(&result);
    }
}

#[test]
fn every_authored_scenario_and_scenario_family() {
    for scenario in cq_testkit::SCENARIOS {
        accept(&scenario.parse_structures().unwrap());
        lower_cq(&scenario.parse_cq().unwrap(), scenario.name);
    }
    for case in cq_testkit::CASES.iter().chain(cq_testkit::blocks::CASES) {
        lower_cq(&case.parse_cq().unwrap(), case.name);
    }
}

#[test]
fn every_workload_through_every_planner() {
    for case in cq_workloads::catalog::all() {
        let source = cq_workloads::case(case.name).unwrap().module().unwrap();
        lower_cq(&source, case.name);
    }
}

#[test]
fn probe_shape_and_structural_checker_reject_mutations() {
    let source: cq_lang_structures::Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            j = pipe probe first [a] l,r;
            out = pipe canon project j keep {b};
            return out as answer(b);
        } structures {
            l host scan = Reader []; r host scan = Reader [];
            r = Provider [a]; out canon = Canon;
        }
    };
    let target = accept(&source);
    let expected: Term = syn::parse_quote!(flatten lookup r [a] l);
    assert_eq!(target.sequence.definitions[2].term, expected);
    let analysis = cq_lang_sequence::contract::check(&target).unwrap();
    assert_eq!(
        analysis.capabilities[&("r".into(), vec!["a".into()])],
        Capability::Exact
    );
    let mut changed = target.clone();
    let Term::Flatten(inner) = changed.sequence.definitions[2].term.clone() else {
        panic!()
    };
    changed.sequence.definitions[2].term = *inner;
    assert!(check(&source, &changed).is_err());
    let mut changed = target.clone();
    changed.sequence.output = Some(OutputProtocol::Reserve);
    assert!(check(&source, &changed).is_err());
    let mut changed = target;
    changed.sequence.definitions.swap(0, 1);
    assert!(check(&source, &changed).is_err());
}

#[test]
fn output_name_is_fresh_and_return_canon_is_preserved() {
    let source: cq_lang_structures::Program = syn::parse_quote! {
        physical {
            relation R(a:i32);
            __sequence_output = pipe rename R@0 {a -> a};
            return canon __sequence_output as answer(a);
        } structures {
            __sequence_output host scan = Reader [];
            __sequence_output head = Canon;
        }
    };
    let target = accept(&source);
    let last = target.sequence.definitions.last().unwrap();
    assert_eq!(last.name, "__sequence_output_");
    assert!(
        matches!(&last.term, Term::Store { capacity: None, inner } if matches!(inner.as_ref(), Term::Canon(_)))
    );
}

#[test]
fn merge_uses_lookup_with_both_ordered_provider_obligations() {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = let rename R@1 {a -> a,b -> b};
            j = pipe canon merge [a] l,r;
            return j as answer(a,b);
        }
    };
    let source = cq_structure_selection::compile(&physical).unwrap();
    let target = accept(&source);
    assert!(matches!(&target.sequence.definitions[2].term,
        Term::Canon(inner) if matches!(inner.as_ref(), Term::Flatten(_))));
    let analysis = cq_lang_sequence::contract::check(&target).unwrap();
    for provider in ["l", "r"] {
        assert_eq!(
            analysis.capabilities[&(provider.into(), vec!["a".into()])],
            Capability::Ordered,
        );
    }
    assert_eq!(analysis.results["j"], "j");
}

#[test]
fn rewritten_claw_preserves_ordered_stream_providers() {
    use quote::ToTokens;
    let source = cq_workloads::case("claw").unwrap().module().unwrap();
    for plan in 0..Planner::ALL.len() {
        for choices in 0..16 {
            let pipeline = cq_workloads::pipelines::Pipeline {
                plan,
                merge: choices & 1 != 0,
                canon: choices & 2 != 0,
                smallest: choices & 4 != 0,
                lazy: choices & 8 != 0,
                ..Default::default()
            };
            let physical = pipeline.physical(&source).unwrap();
            let structures = match cq_structure_selection::compile(&physical) {
                Ok(structures) => structures,
                Err(error) => {
                    assert!(
                        error
                            .to_string()
                            .ends_with(cq_workloads::pipelines::SMALLEST_LAYOUT_REJECTION)
                    );
                    continue;
                }
            };
            compile(&structures).unwrap_or_else(|error| {
                panic!("{pipeline:?}: {error}\n{}", structures.to_token_stream())
            });
        }
    }
}

#[test]
fn groups_lower_to_segmented_folds_after_complete_valuation_canon() {
    let physical = syn::parse_quote! {
        physical {
            relation R(k:i32, witness:i32, value:i64, anonymous:i32);
            raw = pipe rename R@0 {k -> k, witness -> witness, value -> value, anonymous -> anonymous};
            valuations = pipe canon project raw keep {k, witness, value};
            grouped = pipe group valuations by [k] {
                n: usize = count();
                total: i64 = sum(value);
                low: Option<i64> = min(value);
                high: Option<i64> = max(value);
            };
            return grouped as answer(k,n,total,low,high);
        }
    };
    let source = cq_structure_selection::compile(&physical).unwrap();
    let target = accept(&source);
    assert_eq!(
        target.sequence.definitions[1].term,
        syn::parse_quote!(canon map valuations raw)
    );
    assert_eq!(
        target.sequence.definitions[2].term,
        syn::parse_quote!(group grouped valuations)
    );
    let mut changed = target;
    changed.sequence.definitions[2].term = syn::parse_quote!(map grouped valuations);
    assert!(check(&source, &changed).is_err());
    assert!(
        cq_lang_sequence::contract::check(&changed)
            .unwrap_err()
            .to_string()
            .contains("BE-Group")
    );
}

#[test]
fn certified_valuation_set_needs_no_redundant_group_input_canon() {
    let physical = syn::parse_quote! {
        physical {
            relation R(k:i32, witness:i32);
            host_set R;
            r = pipe rename R@0 {k -> k, witness -> witness};
            grouped = pipe group r by [k] { n: usize = count(); };
            return grouped as answer(k,n);
        }
    };
    let source = cq_structure_selection::compile(&physical).unwrap();
    let target = accept(&source);
    assert_eq!(
        target.sequence.definitions[0].term,
        syn::parse_quote!(rows r)
    );
    assert_eq!(
        target.sequence.definitions[1].term,
        syn::parse_quote!(group grouped r)
    );
}

#[test]
fn projected_membership_probe_lowers_to_flat_compact_before_expansion() {
    let physical = syn::parse_quote! {
        physical {
            relation L(a:i32,b:i32); relation R(a:i32,unused:i32); relation T(b:i32,c:i32);
            l = pipe rename L@0 {a -> a,b -> b};
            r = let rename R@1 {a -> a,unused -> unused};
            t = let rename T@2 {b -> b,c -> c};
            member = pipe probe first [a] l,r;
            expanded = pipe probe first [b] member,t;
            out = pipe canon project expanded keep {a,b,c};
            return out as answer(a,b,c);
        }
    };
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let target = accept(&structures);
    let facts = cq_lang_sequence::contract::check(&target).unwrap();
    assert!(
        facts
            .structures
            .membership
            .contains(&("r".into(), vec!["a".into()]))
    );
    assert_eq!(
        target.sequence.definitions[3].term,
        syn::parse_quote!(compact member lookup r [a] l)
    );
    assert_eq!(
        target.sequence.definitions[4].term,
        syn::parse_quote!(flatten lookup t [b] member)
    );
    assert_eq!(facts.results["member"], "member");
    assert_eq!(
        facts.capabilities[&("r".into(), vec!["a".into()])],
        Capability::Membership
    );
}
