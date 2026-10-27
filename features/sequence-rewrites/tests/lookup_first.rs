use std::collections::{BTreeMap, BTreeSet};

use cq_lang_sequence::{
    Capability, Catalog, Program, ProviderToken, Term, check_rewrite, structures,
};
use cq_sequence_plan::rewrites::{LookupFirst, lookup_first, lookup_first_advice, lookup_plan};
use quote::{ToTokens, quote};
use structures::physical::Operator;

fn chain() -> Program {
    let structures = syn::parse_quote! {
        physical {
            relation R(a:i32); relation S(a:i32,b:i32); relation T(b:i32,c:i32);
            r = pipe rename R@0 {a -> a};
            s = let rename S@1 {a -> a,b -> b};
            t = let rename T@2 {b -> b,c -> c};
            rs = pipe probe first [a] r,s;
            answer = pipe canon probe first [b] rs,t;
            return answer as result(a,b,c);
        }
        structures {
            r host scan = Reader []; s host scan = Reader []; t host scan = Reader [];
            s = Index [a]; t = Index [b]; answer canon = Canon;
        }
    };
    cq_sequence_lowering::compile(&structures).unwrap()
}

fn triangle() -> Program {
    let structures = syn::parse_quote! {
        physical {
            relation R(a:i32,b:i32); relation S(b:i32,c:i32); relation T(a:i32,c:i32);
            r = pipe rename R@0 {a -> a,b -> b};
            s = let rename S@1 {b -> b,c -> c};
            t = let rename T@2 {a -> a,c -> c};
            rs = pipe probe first [b] r,s;
            answer = pipe canon probe first [a,c] rs,t;
            return answer as result(a,b,c);
        }
        structures {
            r host scan = Reader []; s host scan = Reader []; t host scan = Reader [];
            s = Index [b]; t = Index [a,c]; answer canon = Canon;
        }
    };
    cq_sequence_lowering::compile(&structures).unwrap()
}

fn catalog() -> Catalog {
    Catalog {
        providers: vec![ProviderToken {
            supports: Vec::new(),
            ty: syn::parse_quote!(Index),
            capabilities: vec![Capability::Exact, Capability::Range, Capability::Membership],
            levels: vec![vec!["host".into()]],
            elements: None,
        }],
        codecs: Vec::new(),
        canons: Vec::new(),
    }
}

#[test]
fn largest_root_uses_evidence_and_deepest_parent_ties() {
    let structures = syn::parse_quote! {
        physical {
            relation R(a:i32); relation S(a:i32); relation T(a:i32);
            r = pipe rename R@0 {a -> a};
            s = let rename S@1 {a -> a};
            t = let rename T@2 {a -> a};
            rs = pipe probe first [a] r,s;
            answer = pipe canon probe first [a] rs,t;
            return answer as result(a);
        }
        structures {
            r host scan = Reader []; s host scan = Reader []; t host scan = Reader [];
            s = Index [a]; t = Index [a]; answer canon = Canon;
        }
    };
    let mut source = cq_sequence_lowering::compile(&structures).unwrap();
    let first = lookup_plan(&source).unwrap().unwrap();
    assert_eq!(
        first
            .order
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["r", "s", "t"]
    );
    assert_eq!(first.parents, [None, Some(0), Some(1)]);
    source.evidence = syn::parse_quote!(evidence { count T = 900; count R = 1; count S = 10; });
    let largest = lookup_plan(&source).unwrap().unwrap();
    assert_eq!(largest.order[0], "t");
    assert_eq!(largest.order[1], "s");
    assert_eq!(largest.parents, [None, Some(0), Some(1)]);
}

fn projected_join() -> Program {
    let structures = syn::parse_quote! {
        physical {
            relation R(a:i32); relation S(a:i32,b:i32);
            r = pipe rename R@0 {a -> a};
            s = let rename S@1 {a -> a,b -> b};
            joined = pipe probe first [a] r,s;
            answer = pipe canon project joined keep {a};
            return answer as result(a);
        }
        structures {
            r host scan = Reader []; s host scan = Reader [];
            s = Index [a] membership payload [a] constants {};
            answer canon = Canon;
        }
    };
    let mut source = cq_sequence_lowering::compile(&structures).unwrap();
    source.evidence = syn::parse_quote!(evidence { count R = 1; count S = 100; });
    source
}

#[test]
fn largest_root_recovers_exclusively_consumed_projected_producer() {
    let source = projected_join();
    assert_eq!(lookup_plan(&source).unwrap().unwrap().order[0], "s");
    let advice = lookup_first_advice(&source, &catalog()).unwrap().unwrap();
    let target = lookup_first(&source, advice.to_token_stream()).unwrap();
    assert!(
        target
            .structures
            .physical
            .program
            .definitions
            .iter()
            .any(|definition| {
                definition.name == "s"
                    && matches!(definition.mode, structures::physical::Mode::Pipe(_))
            })
    );
    assert!(!target.structures.structures.entries.iter().any(|entry| {
        entry.name == "s" && matches!(entry.structure, structures::Structure::Built { .. })
    }));
    assert!(target.structures.structures.entries.iter().any(|entry| {
        entry.name == "r" && matches!(entry.structure, structures::Structure::Built { .. })
    }));
    let inputs = BTreeMap::from([
        ("r".into(), vec![row(&[("a", 1)]), row(&[("a", 1)])]),
        (
            "s".into(),
            vec![
                row(&[("a", 1), ("b", 2)]),
                row(&[("a", 1), ("b", 3)]),
                row(&[("a", 4), ("b", 5)]),
            ],
        ),
    ]);
    assert_eq!(evaluate(&target, &inputs), evaluate(&source, &inputs));
    let mut empty = inputs.clone();
    empty.insert("r".into(), Vec::new());
    assert_eq!(evaluate(&target, &empty), evaluate(&source, &empty));
    check_rewrite(&source, &target).unwrap();
    assert_eq!(
        target,
        lookup_first(&source, advice.to_token_stream()).unwrap()
    );
}

#[test]
fn projected_selection_recovers_its_original_scan_and_predicate() {
    let mut structures = projected_join().structures;
    structures.physical.program.definitions[1] =
        syn::parse_quote!(raw = pipe rename S@1 {a -> a,b -> b};);
    structures
        .physical
        .program
        .definitions
        .insert(2, syn::parse_quote!(s = let select raw if (b > 0);));
    structures.structures.entries[1] = syn::parse_quote!(raw host scan = Reader [];);
    let mut source = cq_sequence_lowering::compile(&structures).unwrap();
    source.evidence = syn::parse_quote!(evidence { count R = 1; count S = 100; });
    let advice = lookup_first_advice(&source, &catalog()).unwrap().unwrap();
    assert_eq!(advice.order.as_ref().unwrap()[0], "s");
    let target = lookup_first(&source, advice.to_token_stream()).unwrap();
    assert_eq!(
        source.structures.physical.program.definitions[2].operator,
        target.structures.physical.program.definitions[2].operator,
    );
    assert!(target.structures.structures.entries.iter().any(|entry| {
        entry.name == "raw"
            && matches!(&entry.structure, structures::Structure::Host { reader, .. } if reader == "scan")
    }));
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn independently_consumed_projected_index_cannot_become_scan_root() {
    let mut structures = projected_join().structures;
    structures
        .physical
        .program
        .definitions
        .insert(2, syn::parse_quote!(filtered = let semi eager [a] r,s;));
    structures.physical.program.definitions[3] =
        syn::parse_quote!(joined = pipe probe first [a] filtered,s;);
    structures
        .structures
        .entries
        .push(syn::parse_quote!(filtered rows = ::std::vec::Vec<_>;));
    let mut source = cq_sequence_lowering::compile(&structures).unwrap();
    source.evidence = syn::parse_quote!(evidence { count R = 1; count S = 100; });
    assert_ne!(lookup_plan(&source).unwrap().unwrap().order[0], "s");
    assert!(lookup_first(&source, quote!(using Index, order [s,filtered])).is_err());
}

#[test]
fn bridge_forward_store_and_provider_precede_expansion() {
    let source = chain();
    let target = lookup_first(&source, quote!(using Index, transfer [s,t])).unwrap();
    check_rewrite(&source, &target).unwrap();
    let mut changed_adapter = target.clone();
    let adapter = changed_adapter
        .structures
        .structures
        .entries
        .iter_mut()
        .find(|entry| {
            entry.name == "r" && matches!(entry.structure, structures::Structure::Host { .. })
        })
        .unwrap();
    *adapter = syn::parse_quote!(r host scan = Other [];);
    assert!(check_rewrite(&source, &changed_adapter).is_err());
    let definitions = &target.sequence.definitions;
    let forward = definitions
        .iter()
        .position(|d| d.name.to_string().starts_with("__lookup_forward"))
        .unwrap();
    assert!(
        matches!(&definitions[forward].term, Term::Store { inner, .. } if matches!(inner.as_ref(), Term::Compact(_, lookup) if matches!(lookup.as_ref(), Term::Lookup { .. })))
    );
    let expand = definitions
        .iter()
        .position(|d| matches!(d.term, Term::Flatten(_)))
        .unwrap();
    assert!(forward < expand);
    let text = target.to_token_stream().to_string();
    assert!(text.contains("compact"));
    assert!(text.contains("lookup"));
    assert_eq!(
        syn::parse2::<Program>(target.to_token_stream()).unwrap(),
        target
    );
}

#[test]
fn retained_leaves_replace_only_obsolete_host_keyed_fallbacks() {
    let structures = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32); relation T(b:i32);
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {a -> a,b -> b};
            t = pipe rename T@2 {b -> b};
            filtered = let semi eager [b] t,r;
            lr = pipe probe first [a] l,r;
            joined = pipe canon probe first [b] lr,filtered;
            return joined as answer(a,b);
        }
        structures {
            l host scan = Reader []; r host scan = Reader [];
            t host scan = Reader []; filtered = Index [b];
            r host filtered = Reader [b]; r host lr = Reader [a]; joined canon = Canon;
        }
    };
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let target = lookup_first(&source, quote!(using Index)).unwrap();
    check_rewrite(&source, &target).unwrap();
    assert!(
        target
            .structures
            .structures
            .entries
            .iter()
            .any(|entry| entry.name == "r"
                && matches!(entry.structure, structures::Structure::Built { .. }))
    );
    assert!(
        !target
            .structures
            .structures
            .entries
            .iter()
            .any(|entry| entry.name == "r"
                && matches!(&entry.structure,
            structures::Structure::Host { reader, .. } if reader != "scan"))
    );
}

#[test]
fn authored_lazy_host_reads_are_not_replaced_by_retention() {
    let structures = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32,b:i32); relation T(a:i32,b:i32);
            l = pipe rename L@0 {a -> a};
            r = pipe rename R@1 {a -> a,b -> b};
            t = let rename T@2 {a -> a,b -> b};
            filtered = pipe semi lazy [a] l,r;
            expanded = pipe probe first [a] filtered,t;
            joined = pipe canon probe first [a,b] expanded,r;
            return joined as answer(a,b);
        }
        structures {
            l host scan = Reader []; r host scan = Reader [];
            t host scan = Reader []; t = Index [a];
            r host filtered = Reader [a]; r host joined = Reader [a,b];
            joined canon = Canon;
        }
    };
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    assert!(lookup_first_advice(&source, &catalog()).unwrap().is_none());
    assert!(lookup_first(&source, quote!(using Index)).is_err());
}

#[test]
fn predicate_transfer_filters_before_building_bridge_provider() {
    let structures = syn::parse_quote! {
        physical {
            relation R(a:i32); relation S(a:i32,b:i32); relation T(b:i32,c:i32);
            r = pipe rename R@0 {a -> a};
            s = let rename S@1 {a -> a,b -> b};
            raw = pipe rename T@2 {b -> b,c -> c};
            t = let select raw if (c = 7);
            rs = pipe probe first [a] r,s;
            answer = pipe canon probe first [b] rs,t;
            return answer as result(a,b,c);
        }
        structures {
            r host scan = Reader []; s host scan = Reader []; raw host scan = Reader [];
            s = Index [a]; t = Index [b]; answer canon = Canon;
        }
    };
    let mut source = cq_sequence_lowering::compile(&structures).unwrap();
    source.evidence = syn::parse_quote!(evidence { count R = 1000; selectivity t = 1 / 100; });
    let plan = lookup_plan(&source).unwrap().unwrap();
    assert!(plan.transfer.iter().any(|name| name == "t"));
    let target = lookup_first(&source, quote!(using Index)).unwrap();
    let first = target
        .structures
        .physical
        .program
        .definitions
        .iter()
        .find(|d| d.name.to_string().starts_with("__lookup_forward"))
        .unwrap();
    assert!(matches!(&first.operator, Operator::Semi(_, k) if k.left == "s" && k.right == "t"));
    assert!(
        target
            .structures
            .structures
            .entries
            .iter()
            .any(|entry| entry.name == first.name
                && matches!(entry.structure, structures::Structure::Built { .. }))
    );
}

#[test]
fn existing_membership_skips_duplicate_forward_and_lookup_without_retaining_scan_root() {
    let structures = syn::parse_quote! {
        physical {
            relation L(b:i32); relation R(a:i32,b:i32); relation S(a:i32);
            l = pipe rename L@0 {b -> b};
            raw = pipe rename R@1 {a -> a,b -> b};
            s = let rename S@2 {a -> a};
            r = let semi eager [a] raw,s;
            lr = pipe probe first [b] l,r;
            answer = pipe canon probe first [a] lr,s;
            return answer as result(a,b);
        }
        structures {
            l host scan = Reader []; raw host scan = Reader []; s host scan = Reader [];
            r = Index [b]; s = Index [a]; answer canon = Canon;
        }
    };
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let advice = quote!(using Index, order [l,r,s], transfer [r,s], early []);
    let target = lookup_first(&source, advice.clone()).unwrap();
    check_rewrite(&source, &target).unwrap();
    let definitions = &target.structures.physical.program.definitions;
    let forwards: Vec<_> = definitions
        .iter()
        .filter(|definition| definition.name.to_string().starts_with("__lookup_forward"))
        .collect();
    assert_eq!(forwards.len(), 1);
    assert!(matches!(&forwards[0].operator, Operator::Semi(_, join)
        if join.left == "l" && join.right == "r"));
    assert!(matches!(
        forwards[0].mode,
        structures::physical::Mode::Pipe(_)
    ));
    assert!(!target.structures.structures.entries.iter().any(|entry| {
        entry.name == forwards[0].name
            && matches!(entry.structure, structures::Structure::Rows { .. })
    }));
    assert!(!definitions.iter().any(|definition| {
        definition.name.to_string().starts_with("__lookup_lookup")
            && matches!(&definition.operator, Operator::Semi(_, join) if join.right == "r")
    }));
    assert_eq!(
        target
            .sequence
            .definitions
            .iter()
            .find(|definition| definition.name == "l")
            .unwrap()
            .term,
        source
            .sequence
            .definitions
            .iter()
            .find(|definition| definition.name == "l")
            .unwrap()
            .term,
    );
    assert!(!target.structures.structures.entries.iter().any(|entry| {
        entry.name == "l" && matches!(entry.structure, structures::Structure::Rows { .. })
    }));
    let inputs = BTreeMap::from([
        (
            "l".into(),
            vec![row(&[("b", 1)]), row(&[("b", 2)]), row(&[("b", 9)])],
        ),
        (
            "raw".into(),
            vec![
                row(&[("a", 1), ("b", 1)]),
                row(&[("a", 1), ("b", 1)]),
                row(&[("a", 2), ("b", 2)]),
            ],
        ),
        ("s".into(), vec![row(&[("a", 1)]), row(&[("a", 1)])]),
    ]);
    assert_eq!(evaluate(&target, &inputs), evaluate(&source, &inputs));
    let mut empty = inputs.clone();
    empty.insert("s".into(), Vec::new());
    assert_eq!(evaluate(&target, &empty), evaluate(&source, &empty));
    let mut estimated = source.clone();
    estimated.evidence = syn::parse_quote!(evidence { key R [b]; count S = 0; });
    let estimated = lookup_first(&estimated, advice).unwrap();
    assert_eq!(estimated.structures, target.structures);
}

#[test]
fn unchanged_unary_intersection_preserves_authored_provider_entries() {
    let structures = syn::parse_quote! {
        physical {
            relation Left(c0:i32); relation Right(c0:i32);
            raw = pipe rename Left@0 {c0 -> x};
            right = let rename Right@1 {c0 -> x};
            left = let semi eager [x] raw,right;
            answer = pipe canon probe first [x] left,right;
            return answer as intersection(x);
        }
        structures {
            raw host scan = Reader []; right host scan = Reader [];
            left rows = ::std::vec::Vec<_>;
            right = Authored [x]; answer canon = Canon;
        }
    };
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let advice = lookup_first_advice(&source, &catalog()).unwrap().unwrap();
    assert!(advice.providers.is_empty());
    let target = lookup_first(&source, advice.to_token_stream()).unwrap();
    // The existing left semijoin witnesses the sole Lookup. The final Expand
    // already has the requested order, so its authored provider stays unchanged.
    assert_eq!(target, source);
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn retained_union_prefix_keeps_constant_and_range_reads_as_row_scans() {
    let structures = syn::parse_quote! {
        physical {
            relation P(x:i32); relation R(x:i32,y:i32);
            p = let rename P@0 {x -> x};
            one = pipe point select p if (x = 1);
            two = pipe range select p if (x >= 2, x < 3);
            arms = let union one,two;
            r = pipe rename R@1 {x -> x,y -> y};
            answer = pipe canon probe first [x] r,arms;
            return answer as result(x,y);
        }
        structures {
            p host scan = Reader []; p rows = ::std::vec::Vec<_>;
            r host scan = Reader []; arms = Index [x]; answer canon = Canon;
        }
    };
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let advice = lookup_first_advice(&source, &catalog()).unwrap().unwrap();
    let target = lookup_first(&source, advice.to_token_stream()).unwrap();
    let facts = cq_lang_sequence::contract::check(&target).unwrap();
    assert!(facts.structures.retained.contains_key("p"));
    assert!(!facts.structures.built.contains_key("p"));
    let reads = &facts.structures.physical.keyed_reads["p"];
    assert!(
        reads
            .iter()
            .any(|(_, _, supply)| *supply == cq_syntax::SupplyKind::Const)
    );
    assert!(
        reads
            .iter()
            .any(|(_, _, supply)| *supply == cq_syntax::SupplyKind::Range)
    );
    assert!(
        !reads
            .iter()
            .any(|(_, _, supply)| *supply == cq_syntax::SupplyKind::Stream)
    );
    assert_eq!(
        target,
        lookup_first(&source, advice.to_token_stream()).unwrap()
    );
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn narrower_membership_does_not_witness_a_later_full_key_lookup() {
    let structures = syn::parse_quote! {
        physical {
            relation R(a:i32); relation S(a:i32,b:i32); relation T(a:i32,b:i32);
            raw = pipe rename R@0 {a -> a};
            s = let rename S@1 {a -> a,b -> b};
            t = let rename T@2 {a -> a,b -> b};
            r = pipe semi eager [a] raw,s;
            rt = pipe probe first [a] r,t;
            answer = pipe canon probe first [a,b] rt,s;
            return answer as result(a,b);
        }
        structures {
            raw host scan = Reader []; s host scan = Reader []; t host scan = Reader [];
            s = Index [a][b]; t = Index [a]; answer canon = Canon;
        }
    };
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let target = lookup_first(
        &source,
        quote!(using Index, order [r,t,s], transfer [], early []),
    )
    .unwrap();
    assert!(
        target
            .structures
            .physical
            .program
            .definitions
            .iter()
            .any(|definition| {
                definition.name.to_string().starts_with("__lookup_lookup")
                    && matches!(&definition.operator, Operator::Semi(_, join)
                if join.right == "s" && join.columns.len() == 2)
            })
    );
    let inputs = BTreeMap::from([
        ("raw".into(), vec![row(&[("a", 1)])]),
        ("t".into(), vec![row(&[("a", 1), ("b", 2)])]),
        ("s".into(), vec![row(&[("a", 1), ("b", 3)])]),
    ]);
    assert!(evaluate(&source, &inputs).is_empty());
    assert_eq!(evaluate(&target, &inputs), evaluate(&source, &inputs));
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn cycle_closing_keys_wait_for_required_expand() {
    let source = triangle();
    let target = lookup_first(
        &source,
        quote!(using Index, order [r,s,t], transfer [], early []),
    )
    .unwrap();
    let definitions = &target.structures.physical.program.definitions;
    let expand = definitions
        .iter()
        .position(|d| matches!(&d.operator, Operator::Probe(_, keyed) if keyed.right == "s"))
        .unwrap();
    let closes_cycle = definitions.iter().position(|d| matches!(&d.operator, Operator::Semi(_, keyed) if keyed.right == "t" && keyed.columns.len() == 2)).unwrap();
    assert!(expand < closes_cycle);
    assert!(
        matches!(&definitions.last().unwrap().operator, Operator::Probe(_, keyed) if keyed.right == "t" && keyed.columns.len() == 2)
    );
    check_rewrite(&source, &target).unwrap();
}

type Row = BTreeMap<String, i32>;
fn row(values: &[(&str, i32)]) -> Row {
    values
        .iter()
        .map(|(name, value)| ((*name).into(), *value))
        .collect()
}

// Independent finite relational oracle, including existential semijoin scope.
// The production implementation is exercised by the checked rewrite above;
// this oracle detects wrong plans even if a checker regression admits one.
fn evaluate(program: &Program, inputs: &BTreeMap<String, Vec<Row>>) -> BTreeSet<Row> {
    let mut results: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for definition in &program.structures.physical.program.definitions {
        let rows = match &definition.operator {
            Operator::Rename(_) => inputs[&definition.name.to_string()].clone(),
            Operator::Project(project) => results[&project.input.to_string()]
                .iter()
                .map(|input| {
                    project
                        .attributes
                        .iter()
                        .map(|column| {
                            let name = column.to_string();
                            let value = input[&name];
                            (name, value)
                        })
                        .collect()
                })
                .collect(),
            Operator::Probe(_, keyed) | Operator::Semi(_, keyed) => {
                let left = &results[&keyed.left.to_string()];
                let right = &results[&keyed.right.to_string()];
                let mut rows = Vec::new();
                for l in left {
                    for r in right {
                        if keyed
                            .columns
                            .iter()
                            .all(|column| l[&column.to_string()] == r[&column.to_string()])
                        {
                            let mut output = l.clone();
                            if matches!(definition.operator, Operator::Semi(..)) {
                                rows.push(output);
                                break;
                            }
                            output.extend(r.clone());
                            rows.push(output);
                        }
                    }
                }
                rows
            }
            other => panic!("oracle fixture has unsupported operator {other:?}"),
        };
        results.insert(definition.name.to_string(), rows);
    }
    results
        .remove(&program.structures.physical.program.returned.to_string())
        .unwrap()
        .into_iter()
        .collect()
}

#[test]
fn false_counts_selectivities_and_keys_change_plans_not_answers() {
    let mut source = triangle();
    let inputs = BTreeMap::from([
        (
            "r".into(),
            vec![
                row(&[("a", 1), ("b", 2)]),
                row(&[("a", 1), ("b", 3)]),
                row(&[("a", 9), ("b", 9)]),
            ],
        ),
        (
            "s".into(),
            vec![
                row(&[("b", 2), ("c", 4)]),
                row(&[("b", 2), ("c", 5)]),
                row(&[("b", 3), ("c", 6)]),
            ],
        ),
        (
            "t".into(),
            vec![
                row(&[("a", 1), ("c", 4)]),
                row(&[("a", 1), ("c", 6)]),
                row(&[("a", 8), ("c", 5)]),
            ],
        ),
    ]);
    let expected = evaluate(&source, &inputs);
    assert_eq!(expected.len(), 2);
    let ordinary = lookup_first(&source, quote!(using Index)).unwrap();
    source.evidence = syn::parse_quote!(evidence {
        count R = 0; count S = 1; count T = 999999;
        key S [b]; key R [a]; selectivity s = 0 / 1;
    });
    let advised = lookup_first(&source, quote!(using Index)).unwrap();
    assert_ne!(ordinary.structures.physical, advised.structures.physical);
    assert_eq!(evaluate(&ordinary, &inputs), expected);
    assert_eq!(evaluate(&advised, &inputs), expected);
    check_rewrite(&source, &advised).unwrap();
}

#[test]
fn catalog_advice_round_trips_and_replays_author_family() {
    let source = chain();
    let advice = lookup_first_advice(&source, &catalog()).unwrap().unwrap();
    let replay: LookupFirst = syn::parse2(advice.to_token_stream()).unwrap();
    assert_eq!(advice, replay);
    let target = lookup_first(&source, advice.to_token_stream()).unwrap();
    assert_eq!(
        target,
        lookup_first(&source, replay.to_token_stream()).unwrap()
    );
}

#[test]
fn heterogeneous_catalog_selects_each_new_layout_and_preserves_authored_types() {
    let structures = syn::parse_quote! {
        physical {
            relation R(a:i32); relation S(a:i32,b:i32);
            relation T(b:i32,c:String); relation U(c:String,d:String);
            r = pipe rename R@0 {a -> a};
            s = let rename S@1 {a -> a,b -> b};
            t = let rename T@2 {b -> b,c -> c};
            u = pipe rename U@3 {c -> c,d -> d};
            rs = pipe probe first [a] r,s;
            rst = pipe probe first [b] rs,t;
            answer = pipe canon probe first [c] rst,u;
            return answer as result(a,b,c,d);
        }
        structures {
            r host scan = Reader []; s host scan = Reader [];
            t host scan = Reader []; u host scan = Reader [];
            s = AuthoredInteger [a]; t = AuthoredString [b];
            u host answer = Reader [c]; answer canon = Canon;
        }
    };
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let capabilities = vec![Capability::Exact, Capability::Membership];
    let available = Catalog {
        providers: vec![
            ProviderToken {
                supports: Vec::new(),
                ty: syn::parse_quote!(Unreadable),
                capabilities: capabilities.clone(),
                levels: Vec::new(),
                elements: None,
            },
            ProviderToken {
                supports: Vec::new(),
                ty: syn::parse_quote!(IntegerIndex),
                capabilities: capabilities.clone(),
                levels: vec![vec!["host".into(), "device".into()]],
                elements: Some(vec![syn::parse_quote!(i32)]),
            },
            ProviderToken {
                supports: Vec::new(),
                ty: syn::parse_quote!(HostIndex),
                capabilities,
                levels: vec![vec!["host".into()]],
                elements: None,
            },
        ],
        codecs: Vec::new(),
        canons: Vec::new(),
    };
    let advice = lookup_first_advice(&source, &available).unwrap().unwrap();
    assert_eq!(
        advice,
        lookup_first_advice(&source, &available).unwrap().unwrap()
    );
    let replay: LookupFirst = syn::parse2(advice.to_token_stream()).unwrap();
    assert_eq!(advice, replay);
    assert!(advice.providers.iter().any(|(name, _, ty, _)| {
        name.to_string().starts_with("__lookup_forward") && *ty == syn::parse_quote!(IntegerIndex)
    }));
    assert!(
        advice
            .providers
            .iter()
            .any(|(name, _, ty, _)| name == "u" && *ty == syn::parse_quote!(HostIndex))
    );
    assert!(
        advice
            .providers
            .iter()
            .all(|(_, _, ty, _)| *ty != syn::parse_quote!(Unreadable))
    );
    let target = lookup_first(&source, replay.to_token_stream()).unwrap();
    assert_eq!(
        target,
        lookup_first(&source, advice.to_token_stream()).unwrap()
    );
    assert!(
        target
            .structures
            .structures
            .entries
            .iter()
            .any(|entry| entry.name == "t"
                && matches!(&entry.structure,
            structures::Structure::Built { keyed, .. }
                if keyed.ty == syn::parse_quote!(AuthoredString)))
    );
    let facts = cq_lang_sequence::contract::check(&target).unwrap();
    let mut integer_forwards = 0;
    for entry in &target.structures.structures.entries {
        if !entry.name.to_string().starts_with("__lookup_forward") {
            continue;
        }
        if let structures::Structure::Built { keyed, .. } = &entry.structure {
            assert_eq!(keyed.ty, syn::parse_quote!(IntegerIndex));
            assert!(
                facts.structures.layouts[&entry.name.to_string()]
                    .iter()
                    .all(
                        |column| facts.structures.physical.rows[&entry.name.to_string()].types
                            [column]
                            == syn::parse_quote!(i32)
                    )
            );
            integer_forwards += 1;
        }
    }
    assert!(integer_forwards > 0);
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn provider_overrides_reject_duplicates_unused_and_authored_layouts() {
    let source = chain();
    for arguments in [
        quote!(using Index, providers [missing [a] = Other]),
        quote!(using Index, providers [s [a] = Other]),
        quote!(using Index, providers [
            __lookup_forward [a] = Other, __lookup_forward [a] = Other
        ]),
        quote!(using Index, providers [], providers []),
    ] {
        assert!(lookup_first(&source, arguments).is_err());
    }
}

#[test]
fn catalog_advice_rejects_unavailable_generated_row_types() {
    let structures = syn::parse_quote! {
        physical {
            relation R(a:i32); relation S(a:i32);
            r = pipe rename R@0 {a -> a};
            s = pipe rename S@1 {a -> a};
            answer = pipe canon probe first [a] r,s;
            return answer as result(a);
        }
        structures {
            r host scan = Reader []; s host scan = Reader [];
            s host answer = Reader [a]; answer canon = Canon;
        }
    };
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let mut available = catalog();
    available.providers[0].elements = Some(vec![syn::parse_quote!(String)]);
    assert!(lookup_first_advice(&source, &available).is_err());
    available.providers[0].elements = None;
    available.providers[0].levels.clear();
    assert!(lookup_first_advice(&source, &available).is_err());
}

#[test]
fn generated_provider_uses_live_guard_output_not_full_physical_heading() {
    let structures = syn::parse_quote! {
        physical {
            relation P(a:i32,label:String); relation L(a:i32);
            p = pipe rename P@0 {a -> a,label -> label};
            guarded = let guard p if (label.contains("green"));
            l = let rename L@1 {a -> a};
            joined = pipe probe first [a] guarded,l;
            answer = pipe canon project joined keep {a};
            return answer as answer(a);
        }
        structures {
            p host scan = Reader []; l host scan = Reader [];
            guarded rows = ::std::vec::Vec<_>; l = Index [a];
            answer canon = Canon;
        }
    };
    let mut source = cq_sequence_lowering::compile(&structures).unwrap();
    source.evidence = syn::parse_quote!(evidence { count P = 1; count L = 100; });
    let mut available = catalog();
    available.providers[0].elements = Some(vec![syn::parse_quote!(i32)]);
    let advice = lookup_first_advice(&source, &available).unwrap().unwrap();
    assert!(
        advice
            .providers
            .iter()
            .any(|(name, _, _, _)| name == "guarded")
    );
    let target = lookup_first(&source, advice.to_token_stream()).unwrap();
    let facts = cq_lang_sequence::contract::check(&target).unwrap();
    assert_eq!(
        facts.structures.physical.headings["guarded"],
        ["a", "label"]
    );
    assert_eq!(facts.structures.layouts["guarded"], ["a"]);
    assert_eq!(
        target,
        lookup_first(
            &source,
            syn::parse2::<LookupFirst>(advice.to_token_stream(),)
                .unwrap()
                .to_token_stream()
        )
        .unwrap(),
    );
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn rejects_bad_orders_preserves_authored_schedules_and_named_evidence() {
    let mut source = chain();
    assert!(lookup_first(&source, quote!(using Index, order [r,s,s])).is_err());
    source.evidence = syn::parse_quote!(evidence { count rs = 50; });
    assert!(lookup_first_advice(&source, &catalog()).unwrap().is_none());
    assert!(lookup_first(&source, quote!(using Index)).is_err());
    source.evidence = Default::default();
    source.sequence.definitions[0].term =
        Term::Canon(Box::new(source.sequence.definitions[0].term.clone()));
    assert!(lookup_first_advice(&source, &catalog()).unwrap().is_none());
    assert!(lookup_first(&source, quote!(using Index)).is_err());
}

#[test]
fn a_false_estimated_key_never_licenses_dropping_a_join() {
    let mut source = triangle();
    source.evidence = syn::parse_quote!(evidence { key T [a]; count T = 1; });
    let mut changed = lookup_first(&source, quote!(using Index)).unwrap();
    // Return a proper prefix: both programs are individually meaningful, but
    // the Sequence construction no longer computes the same query support.
    changed.sequence.definitions.last_mut().unwrap().term = Term::Store {
        capacity: None,
        inner: Box::new(Term::Ref(syn::parse_quote!(r))),
    };
    assert!(check_rewrite(&source, &changed).is_err());
}

#[test]
fn rust_prefix_keeps_written_before_obligations() {
    let structures = syn::parse_quote! {
        physical {
            relation R(a:i32); relation S(a:i32,b:i32);
            r = pipe rename R@0 {a -> a};
            s = let rename S@1 {a -> a,b -> b};
            guarded = pipe guard r if (true);
            answer = pipe canon probe first [a] guarded,s;
            return answer as result(a,b);
        }
        structures {
            r host scan = Reader []; s host scan = Reader [];
            s = Index [a]; answer canon = Canon;
        }
    };
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let target = lookup_first(&source, quote!(using Index)).unwrap();
    let prefix = &target.structures.physical.program.definitions[..3];
    assert_eq!(
        prefix
            .iter()
            .map(|d| d.name.to_string())
            .collect::<Vec<_>>(),
        ["r", "s", "guarded"]
    );
    assert_eq!(
        prefix[2].operator,
        source.structures.physical.program.definitions[2].operator
    );
    let mut moved = target.structures.clone();
    moved.physical.program.definitions.swap(1, 2);
    let moved = cq_sequence_lowering::compile(&moved).unwrap();
    assert!(check_rewrite(&source, &moved).is_err());
}

#[test]
fn distinct_lookup_keys_name_distinct_provider_stores() {
    let source = triangle();
    let target = lookup_first(
        &source,
        quote!(using Index, order [r,s,t], transfer [], early []),
    )
    .unwrap();
    let facts = cq_lang_sequence::contract::check(&target).unwrap();
    assert!(
        facts
            .structures
            .built
            .values()
            .all(|layouts| layouts.len() == 1)
    );
    let alias = target
        .structures
        .physical
        .program
        .definitions
        .iter()
        .find(|definition| definition.name.to_string().starts_with("__lookup_provider"))
        .unwrap();
    assert!(matches!(&alias.operator, Operator::Project(project)
        if project.input == "t" && project.attributes.len() == 2));
    assert!(facts.structures.retained.contains_key("t"));
    assert!(
        !facts
            .structures
            .retained
            .contains_key(&alias.name.to_string())
    );
    assert!(
        target
            .sequence
            .definitions
            .iter()
            .any(|definition| definition.name == alias.name
                && matches!(&definition.term, Term::Store { inner, .. }
                if matches!(inner.as_ref(), Term::Map(name, _) if *name == alias.name)))
    );
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn provider_aliases_are_inserted_after_the_pinned_rust_prefix() {
    let mut structures = triangle().structures;
    structures
        .physical
        .program
        .definitions
        .insert(3, syn::parse_quote!(guarded = pipe guard r if (true);));
    let Operator::Probe(_, keyed) = &mut structures.physical.program.definitions[4].operator else {
        panic!("triangle probe");
    };
    keyed.left = syn::parse_quote!(guarded);
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let target = lookup_first(
        &source,
        quote!(using Index, order [guarded,s,t], transfer [], early []),
    )
    .unwrap();
    for (before, after) in source.structures.physical.program.definitions[..4]
        .iter()
        .zip(&target.structures.physical.program.definitions[..4])
    {
        assert_eq!(before.name, after.name);
        assert_eq!(before.operator, after.operator);
    }
    assert!(
        target.structures.physical.program.definitions[4..]
            .iter()
            .any(|definition| definition.name.to_string().starts_with("__lookup_provider"))
    );
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn live_smallest_prefix_keeps_candidate_and_membership_fallbacks() {
    let physical = syn::parse_quote! {
        physical {
            relation R(a:i32,b:i32); relation S(a:i32,c:i32); relation T(b:i32,c:i32);
            relation X(c:i32,d:i32); relation Y(d:i32,e:i32);
            r = pipe rename R@0 {a -> a,b -> b};
            s = pipe rename S@1 {a -> a,c -> c};
            t = pipe rename T@2 {b -> b,c -> c};
            rs = pipe probe smallest [a] r,s;
            rst = pipe probe first [b,c] rs,t;
            guarded = pipe guard rst if (true);
            x = let rename X@3 {c -> c,d -> d};
            y = let rename Y@4 {d -> d,e -> e};
            prefix = pipe probe first [c] guarded,x;
            answer = pipe canon probe first [d] prefix,y;
            return answer as result(a,b,c,d,e);
        }
    };
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let target = lookup_first(&source, quote!(using Index)).unwrap();
    let before = cq_lang_sequence::contract::check(&source).unwrap();
    let after = cq_lang_sequence::contract::check(&target).unwrap();
    let mut preserved = 0;
    for (key, ty) in &before.structures.fallbacks {
        if key.1 == "candidates" || key.1 == "membership" {
            assert_eq!(after.structures.fallbacks.get(key), Some(ty));
            preserved += 1;
        }
    }
    assert_eq!(preserved, 4);
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn barbell_and_h09_smallest_corpus_plans_replay_lookup_first() {
    for name in ["barbell", "retail_h09"] {
        let query = cq_workloads::case(name).unwrap().module().unwrap();
        let mut rewritten = 0;
        for plan in 0..cq_workloads::pipelines::Planner::ALL.len() {
            let pipeline = cq_workloads::pipelines::Pipeline {
                plan,
                smallest: true,
                ..Default::default()
            };
            let physical = pipeline.physical(&query).unwrap();
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
            let source = cq_sequence_lowering::compile(&structures).unwrap();
            if let Some(advice) = lookup_first_advice(&source, &catalog()).unwrap() {
                let target = lookup_first(&source, advice.to_token_stream())
                    .unwrap_or_else(|error| panic!("{name} {plan}: {error}"));
                check_rewrite(&source, &target).unwrap();
                rewritten += 1;
            }
        }
        assert!(rewritten > 0, "{name}");
    }
}

#[test]
fn removed_smallest_region_drops_only_its_obsolete_fallbacks() {
    let physical = syn::parse_quote! {
        physical {
            relation R(a:i32,b:i32); relation S(a:i32,c:i32); relation T(b:i32,c:i32);
            r = pipe rename R@0 {a -> a,b -> b};
            s = pipe rename S@1 {a -> a,c -> c};
            t = pipe rename T@2 {b -> b,c -> c};
            rs = pipe probe smallest [a] r,s;
            answer = pipe canon probe first [b,c] rs,t;
            return answer as result(a,b,c);
        }
    };
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let target = lookup_first(&source, quote!(using Index)).unwrap();
    let facts = cq_lang_sequence::contract::check(&target).unwrap();
    assert!(
        facts
            .structures
            .fallbacks
            .keys()
            .all(|(_, reader, _)| reader != "candidates" && reader != "membership")
    );
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn d72_automatic_cyclic_plan_records_one_layout_per_provider() {
    let query = syn::parse_str(include_str!(
        "../../planners/tests/fixtures/le_retail_d72_inner.cq"
    ))
    .unwrap();
    let physical = cq_workloads::pipelines::Pipeline::default()
        .physical(&query)
        .unwrap();
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let advice = lookup_first_advice(&source, &catalog()).unwrap().unwrap();
    let target = lookup_first(&source, advice.to_token_stream()).unwrap();
    let facts = cq_lang_sequence::contract::check(&target).unwrap();
    assert!(
        facts
            .structures
            .built
            .values()
            .all(|layouts| layouts.len() == 1)
    );
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn superseded_prefix_layouts_do_not_create_unread_provider_aliases() {
    let mut structures = chain().structures;
    structures.structures.entries.extend([
        syn::parse_quote!(s = Extra [a][b];),
        syn::parse_quote!(s rows = ::std::vec::Vec<_>;),
    ]);
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let target = lookup_first(&source, quote!(using Index)).unwrap();
    assert!(!target.structures.structures.entries.iter().any(
        |entry| matches!(&entry.structure, structures::Structure::Built { keyed, .. }
            if keyed.ty == syn::parse_quote!(Extra))
    ));
    check_rewrite(&source, &target).unwrap();
}

#[test]
fn native_intersection_merge_retains_displaced_left_reader() {
    let structures = syn::parse_quote! {
        physical {
            relation Left(c0:i32); relation Right(c0:i32);
            r0 = pipe rename Left@0 {c0 -> x};
            r1 = pipe rename Right@1 {c0 -> x};
            r2 = pipe merge [x] r0,r1;
            r3 = pipe canon project r2 keep {x};
            return r3 as intersection(x);
        }
        structures {
            r0 host scan = Reader []; r0 host r2 = OrderedReader [x];
            r1 host scan = Reader []; r1 host r2 = OrderedReader [x];
            r3 canon = Canon;
        }
    };
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    let advice = lookup_first_advice(&source, &catalog()).unwrap().unwrap();
    let target = lookup_first(&source, advice.to_token_stream()).unwrap();
    let facts = cq_lang_sequence::contract::check(&target).unwrap();
    assert_eq!(
        facts.structures.physical.modes["r0"],
        structures::physical::ModeKind::Let
    );
    assert!(facts.structures.retained.contains_key("r0"));
    assert!(!target.structures.structures.entries.iter().any(|entry| {
        entry.name == "r0"
            && matches!(&entry.structure, structures::Structure::Host { reader, .. } if reader == "r2")
    }));
    check_rewrite(&source, &target).unwrap();
}

mod intersection_configuration_matrix {
    use super::*;
    use syn::visit_mut::VisitMut;

    fn check_configuration(
        planner: proc_macro2::TokenStream,
        mut pipeline: cq_workloads::pipelines::Pipeline,
    ) {
        let _ = CONFIGURATIONS;
        let sorted_runs = planner.to_string() == "le_runs";
        let planner: cq_workloads::pipelines::Planner = if sorted_runs {
            cq_workloads::pipelines::Planner::Le
        } else {
            syn::parse2(planner).unwrap()
        };
        pipeline.plan = cq_workloads::pipelines::Planner::ALL
            .iter()
            .position(|candidate| candidate.name() == planner.name())
            .unwrap();
        let query = cq_workloads::case("intersection")
            .unwrap()
            .module()
            .unwrap();
        let physical = pipeline.physical(&query).unwrap();
        let selection = if pipeline.positional {
            quote!(positional(Left, Right))
        } else {
            Default::default()
        };
        let mut selected = cq_structure_selection::compile_with(&physical, selection).unwrap();
        let original = selected.clone();
        struct Sorted;
        impl VisitMut for Sorted {
            fn visit_type_mut(&mut self, ty: &mut syn::Type) {
                syn::visit_mut::visit_type_mut(self, ty);
                if let syn::Type::Path(path) = ty
                    && path
                        .path
                        .segments
                        .last()
                        .is_some_and(|segment| segment.ident == "HashIndex")
                {
                    *ty = syn::parse_quote!(::cq_workloads::SortedIndex<_, _>);
                }
            }
        }
        if pipeline.ordered_dedup {
            for entry in &mut selected.structures.entries {
                if let structures::Structure::Dedup { ty, .. }
                | structures::Structure::HeadDedup { ty, .. } = &mut entry.structure
                {
                    *ty = syn::parse_quote!(::std::collections::BTreeSet<_>);
                }
            }
        }
        if sorted_runs {
            selected = cq_rewrite_passes::sorted_runs(&selected, Default::default()).unwrap();
        }
        if pipeline.sorted_payload {
            for entry in &mut selected.structures.entries {
                if let structures::Structure::Built { keyed, .. }
                | structures::Structure::Host { keyed, .. } = &mut entry.structure
                {
                    Sorted.visit_type_mut(&mut keyed.ty);
                }
            }
        }
        structures::check_rewrite(&original, &selected).unwrap();
        let source = cq_sequence_lowering::compile(&selected).unwrap();
        let advice = lookup_first_advice(&source, &catalog())
            .unwrap_or_else(|error| panic!("{error}\nSOURCE: {}", source.to_token_stream()));
        if let Some(advice) = advice {
            let target = lookup_first(&source, advice.to_token_stream()).unwrap();
            check_rewrite(&source, &target).unwrap();
        }
    }

    macro_rules! pipeline_configuration {
        ($case:ident, $name:ident;
         planner: [$($plan:tt)*];
         merge: [$($merge:tt)*]; canon: [$($canon:tt)*];
         intersection: [$($intersection:tt)*];
         host: [$load:ident] $view:ident [$($positional:ident)?];
         dedup: [$($dedup:tt)*]; payload: [$($payload:tt)*];
         membership: [$($membership:tt)*]; printer: $printer:path;) => {
            #[test]
            fn $name() {
                let _ = CONFIG;
                super::check_configuration(
                    quote::quote!($($plan)*),
                    cq_workloads::pipelines::Pipeline {
                        merge: !stringify!($($merge)*).is_empty(),
                        canon: !stringify!($($canon)*).is_empty(),
                        smallest: !stringify!($($intersection)*).is_empty(),
                        positional: !stringify!($($positional)?).is_empty(),
                        ordered_dedup: !stringify!($($dedup)*).is_empty(),
                        sorted_payload: !stringify!($($payload)*).is_empty(),
                        lazy: !stringify!($($membership)*).is_empty(),
                        ..Default::default()
                    },
                );
            }
        };
    }
    include!("../../workloads/e2e/corpus/tests/support/le_configurations.rs");
    include!("../../workloads/e2e/corpus/tests/support/configurations.rs");
    case_configurations!(intersection, run_intersection);
}

#[test]
fn unary_intersection_corpus_orderings_preserve_checked_representations() {
    let query = cq_workloads::case("intersection")
        .unwrap()
        .module()
        .unwrap();
    for plan in 0..cq_workloads::pipelines::Planner::ALL.len() {
        let physical = cq_workloads::pipelines::Pipeline {
            plan,
            ..Default::default()
        }
        .physical(&query)
        .unwrap();
        for positional in [false, true] {
            let selection = if positional {
                quote!(positional(Left, Right))
            } else {
                Default::default()
            };
            let structures = cq_structure_selection::compile_with(&physical, selection).unwrap();
            let source = cq_sequence_lowering::compile(&structures).unwrap();
            let advice = lookup_first_advice(&source, &catalog()).unwrap_or_else(|error| {
                panic!("intersection {plan} positional={positional}: {error}")
            });
            if let Some(advice) = advice {
                let target = lookup_first(&source, advice.to_token_stream()).unwrap();
                check_rewrite(&source, &target).unwrap();
            }
        }
    }
}

#[test]
fn constraint_only_cq_union_arms_keep_the_prefix_domain() {
    let query = syn::parse_quote! {
        struct Q;
        relation P(x:i32);
        answer(x) :- P(x), { if *x == 1 } | { if *x == 2 };
    };
    let physical = cq_workloads::pipelines::Pipeline::default()
        .physical(&query)
        .unwrap();
    let structures = cq_structure_selection::compile(&physical).unwrap();
    let source = cq_sequence_lowering::compile(&structures).unwrap();
    if let Some(advice) = lookup_first_advice(&source, &catalog()).unwrap() {
        let target = lookup_first(&source, advice.to_token_stream()).unwrap();
        let facts = cq_lang_sequence::contract::check(&target).unwrap();
        for (name, layouts) in &facts.structures.built {
            for (levels, _) in layouts {
                assert!(facts.structures.physical.keyed_reads[name].iter().any(
                    |(_, keys, supply)| {
                        *supply == cq_syntax::SupplyKind::Stream
                            && structures::prefix_depth(levels, keys).is_some()
                    }
                ));
            }
        }
        check_rewrite(&source, &target).unwrap();
    }
}
