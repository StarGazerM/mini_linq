use std::collections::{BTreeMap, BTreeSet};

use cq_lang_sequence::{
    Capability, Catalog, Program, ProviderToken, Term, check_rewrite, structures,
};
use cq_sequence_rewrite_passes::{LookupFirst, lookup_first, lookup_first_advice, lookup_plan};
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

#[test]
fn bridge_forward_store_and_provider_precede_expansion() {
    let source = chain();
    let target = lookup_first(&source, quote!(using Index)).unwrap();
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
