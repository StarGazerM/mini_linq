use cq_lang_physical as physical;
use cq_lang_structures as structures;
use cq_structure_selection::{check, compile};
use cq_testkit::corpus as scenarios;
use quote::ToTokens;

#[test]
fn overlapping_union_cannot_drop_its_final_dedup_provider() {
    let source: physical::Program = syn::parse_quote! {physical {
        relation R(a:i32); relation S(a:i32);
        r = pipe rename R@0.0.0 {a -> x}; s = pipe rename S@0.1.0 {a -> x};
        u = pipe union r,s block @0 free[] interface[x]; return canon u as answer(x);
    }};
    let target = compile(&source).unwrap();
    assert!(
        target
            .structures
            .entries
            .iter()
            .any(|entry| matches!(entry.structure, structures::Structure::HeadDedup { .. }))
    );
    let mut dropped = target.clone();
    dropped
        .structures
        .entries
        .retain(|entry| !matches!(entry.structure, structures::Structure::HeadDedup { .. }));
    assert!(check(&source, &dropped).is_err());
}

#[test]
fn independent_union_indices_stream_the_chain_without_retained_rows() {
    let source: physical::Program = syn::parse_quote! {physical {
        relation L(a:i32); relation A(a:i32,b:i32); relation B(a:i32,b:i32);
        l = pipe rename L@0 {a -> x}; a = pipe rename A@1.0.0 {a -> x,b -> y}; b = pipe rename B@1.1.0 {a -> x,b -> y};
        d = let canon union a,b block @1 free[x] interface[y];
        j = pipe probe first [x] l,d; q = pipe project j keep {y}; out = pipe canon semi eager [y] q,d; return out as answer(y);
    }};
    let target = compile(&source).unwrap();
    let facts = structures::contract::check(&target).unwrap();
    assert_eq!(facts.built["d"].len(), 2);
    assert!(!facts.retained.contains_key("d"));
    assert!(facts.filtered.contains_key("d"));
    let mut missing = target.clone();
    let position = missing.structures.entries.iter().position(|entry|entry.name=="d" && matches!(&entry.structure,structures::Structure::Built {keyed,..} if keyed.level_names()==vec![vec!["y".to_owned()]])).unwrap();
    missing.structures.entries.remove(position);
    assert!(check(&source, &missing).is_err());
}

#[test]
fn union_curried_indices_and_an_explicit_scan_keep_both_decisions() {
    let source: physical::Program = syn::parse_quote! {physical {
        relation L(a:i32,b:i32); relation A(a:i32,b:i32,c:i32); relation B(a:i32,b:i32,c:i32);
        l = pipe rename L@0 {a -> x,b -> y}; a = pipe rename A@1.0.0 {a -> x,b -> y,c -> z}; b = pipe rename B@1.1.0 {a -> x,b -> y,c -> z};
        d = let canon union a,b block @1 free[x,y] interface[z];
        p = pipe project l keep {x}; checked = pipe semi eager [x] p,d;
        c = pipe probe first [x] checked,l; j = pipe probe first [x,y] c,d;
        aux = let project d keep {x,y}; out = pipe canon semi eager [x,y] j,aux; result = pipe canon project out keep {x,z}; return result as answer(x,z);
    }};
    let target = compile(&source).unwrap();
    let facts = structures::contract::check(&target).unwrap();
    assert!(facts.retained.contains_key("d"));
    assert!(facts.filtered.contains_key("d"));
    assert_eq!(facts.built["d"][0].0.len(), 2);
}

#[test]
fn proven_disjoint_union_rejects_cross_arm_canon_but_retains_final_canon() {
    let source: physical::Program = syn::parse_quote! {physical {
        relation R(a:i32); relation S(a:i32);
        rr = pipe rename R@0.0.0 {a -> x}; ss = pipe rename S@0.1.0 {a -> x};
        r = pipe point select rr if (x = 1); s = pipe point select ss if (x = 2);
        u = pipe union r,s block @0 free[] interface[x]; return canon u as answer(x);
    }};
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    let mut kept = source.clone();
    kept.program
        .definitions
        .iter_mut()
        .find(|d| d.name == "u")
        .unwrap()
        .canon = Some(Default::default());
    assert!(physical::contract::check(&kept).is_err());
}
#[test]
fn scenarios_discharge_every_physical_read_and_reject_changed_targets() {
    for case in scenarios::SCENARIOS {
        let source = syn::parse_str(case.physical).unwrap();
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
        assert_eq!(
            target.structures,
            syn::parse_str::<structures::Structures>(case.structures).unwrap(),
            "{}",
            case.name
        );
        let mut changed = target;
        changed.physical.program.head.relation = syn::parse_quote!(mutated);
        assert!(check(&source, &changed).is_err());
    }
}

fn chain() -> physical::Program {
    syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32); relation S(b:i32);
        l = let rename L@0 {a -> a}; r = let rename R@1 {a -> a,b -> b}; s = pipe rename S@2 {b -> b};
        j = pipe probe first [a] l,r; x = pipe semi eager [a] j,l; p = pipe canon semi eager [b] x,s; return p as answer(a,b);
    } }
}

#[test]
fn concrete_types_and_preservation_are_checked_independently() {
    let source = chain();
    let target = compile(&source).unwrap();
    assert_eq!(
        built_entries(&target),
        syn::parse_quote!(structures { l = ::std::collections::HashSet<_> [a]; r = ::cq_storage::HashIndex<'_, _, _> [a]; })
    );
    check(&source, &target).unwrap();
    let mut reordered = target.clone();
    reordered.structures.entries.reverse();
    check(&source, &reordered).unwrap();
    for replacement in [
        syn::parse_quote!(l = ::cq_storage::HashIndex<'_, _, _> [a];),
        syn::parse_quote!(l = Custom [a];),
    ] {
        let mut q = target.clone();
        q.structures.entries[0] = replacement;
        check(&source, &q).unwrap();
    }
    let mut q = target.clone();
    q.structures.entries.remove(1);
    assert!(check(&source, &q).is_err());
    let mut q = target.clone();
    q.structures
        .entries
        .push(syn::parse_quote!(s = ::std::collections::HashSet<_> [b];));
    assert!(check(&source, &q).is_err());
    let mut q = target;
    q.physical.program.declarations[0].value = Some(syn::parse_quote!(= (rows())));
    assert!(check(&source, &q).is_err());
}

#[test]
fn host_reads_record_fallback_types_for_every_read_kind() {
    for op in ["probe first [a] l,r", "semi eager [a] l,r", "merge[a] l,r"] {
        let p: physical::Program = syn::parse_str(&format!("physical {{ relation L(a:i32); relation R(a:i32); l = pipe rename L@0 {{a -> a}}; r = pipe rename R@1 {{a -> a}}; j = pipe canon {op}; return j as answer(a); }}")).unwrap();
        let target = compile(&p).unwrap();
        assert!(
            target
                .structures
                .entries
                .iter()
                .all(|e| !matches!(e.structure, structures::Structure::Built { .. }))
        );
        check(&p, &target).unwrap();
    }
    let p: physical::Program = syn::parse_quote!(physical { relation R(a:i32); r = pipe rename R@0 {a -> x}; w = pipe canon point select r if (x = 2); return w as answer(x); });
    let target = compile(&p).unwrap();
    assert!(
        target
            .structures
            .entries
            .iter()
            .all(|e| !matches!(e.structure, structures::Structure::Built { .. }))
    );
}

#[test]
fn const_supplies_on_host_and_materialized_inputs_never_build() {
    for mode in ["pipe", "let"] {
        let p: physical::Program = syn::parse_str(&format!(
            "physical {{ relation R(a:i32); r = {mode} rename R@0 {{a -> x}}; w = pipe canon point select r if (x = 2); return w as answer(x); }}"
        )).unwrap();
        let target = compile(&p).unwrap();
        assert!(
            target
                .structures
                .entries
                .iter()
                .all(|e| !matches!(e.structure, structures::Structure::Built { .. }))
        );
        check(&p, &target).unwrap();
        let mut unnecessary = target;
        unnecessary
            .structures
            .entries
            .push(syn::parse_quote!(r = ::cq_storage::HashIndex<'_, _, _> [x];));
        assert!(check(&p, &unnecessary).is_err());
    }
}

#[test]
fn ordered_read_types_and_changed_key_streams_have_mutation_coverage() {
    let p: physical::Program = syn::parse_quote!(physical {
        relation L(a:i32); relation R(a:i32);
        l = pipe rename L@0 {a -> x}; r = let rename R@1 {a -> x};
        w = pipe canon merge[x] l,r; return w as answer(x);
    });
    let target = compile(&p).unwrap();
    assert_eq!(
        built_entries(&target),
        syn::parse_quote!(structures { r = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [x]; })
    );
    let mut q = target;
    q.structures.entries[0] = syn::parse_quote!(r = ::cq_storage::HashIndex<'_, _, _> [x];);
    assert!(check(&p, &q).is_err());
}

#[test]
fn compatible_membership_chains_use_recorded_trie_levels() {
    let p: physical::Program = syn::parse_quote!(physical {
        relation L(a:i32,b:i32); relation R(a:i32,b:i32);
        l = pipe rename L@0 {a -> a,b -> b}; r = let rename R@1 {a -> a,b -> b};
        p = pipe project l keep {a}; x = pipe semi eager [a] p,r;
        j = pipe canon probe first [a,b] r,r;
        k = pipe canon semi eager [a] j,x; return k as answer(a,b);
    });
    // x is a keyed intermediate and must be retained.
    let mut p = p;
    p.program.definitions[3].mode = syn::parse_quote!(let);
    let target = compile(&p).unwrap();
    check(&p, &target).unwrap();
    let entry = target
        .structures
        .entries
        .iter()
        .position(|e| e.name == "r")
        .unwrap();
    assert_eq!(
        target.structures.entries[entry],
        syn::parse_quote!(r = ::cq_storage::Trie<'_, _, ::cq_storage::HashIndex<'_, _, _>> [a][b];)
    );
    let mut q = target;
    q.structures.entries[entry] = syn::parse_quote!(r = ::cq_storage::HashIndex<'_, _, _> [a,b];);
    assert!(check(&p, &q).is_err());
}

#[test]
fn smallest_candidate_let_levels_keep_independent_mutation_checks() {
    // Author the physical premise directly: adjacent reads introduce then
    // test one column. The fine-split source planner may choose other steps.
    let source: physical::Program = syn::parse_quote! { physical {
        relation L(a: i32, b: i32);
        relation A(a: i32, c: i32); relation B(b: i32, c: i32);
        l = pipe rename L@0 { a -> a, b -> b };
        a = let rename A@1 { a -> a, c -> c };
        b = let rename B@2 { b -> b, c -> c };
        p = pipe probe smallest [a] l,a;
        result = pipe canon semi eager [b,c] p,b;
        return result as answer(a,b,c);
    }};
    let target = compile(&source).unwrap();
    check(&source, &target).unwrap();
    let entry = target
        .structures
        .entries
        .iter()
        .position(|e| {
            matches!(&e.structure, structures::Structure::Built { keyed, .. } if keyed.levels.len() == 2)
        })
        .unwrap();
    let mut changed = target.clone();
    let structures::Structure::Built { keyed, .. } =
        &mut changed.structures.entries[entry].structure
    else {
        panic!()
    };
    keyed.levels.reverse();
    assert!(check(&source, &changed).is_err());
    let mut changed = target;
    let structures::Structure::Built { keyed, .. } =
        &mut changed.structures.entries[entry].structure
    else {
        panic!()
    };
    keyed.ty = syn::parse_quote!(::std::collections::HashSet<_>);
    assert!(check(&source, &changed).is_err());
}

#[test]
fn mixed_smallest_selects_each_retained_projection_and_checks_mutations() {
    for (retained, host, prefix) in [("r", "s", "b"), ("s", "r", "a")] {
        let r = if retained == "r" {
            "r0 = pipe rename R@0 {b -> b,c -> c}; r = let project r0 keep {b,c};"
        } else {
            "r = pipe rename R@0 {b -> b,c -> c};"
        };
        let s = if retained == "s" {
            "s0 = pipe rename S@0 {a -> a,c -> c}; s = let project s0 keep {a,c};"
        } else {
            "s = pipe rename S@0 {a -> a,c -> c};"
        };
        let source: physical::Program = syn::parse_str(&format!(
            "physical {{
                relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
                l = pipe rename L@0 {{a -> a,b -> b}}; {r} {s}
                candidate = pipe probe smallest [b] l,r;
                checked = pipe canon semi eager [a,c] candidate,s;
                return checked as answer(a,b,c);
            }}"
        ))
        .unwrap();
        physical::contract::check(&source).unwrap();
        let target = compile(&source).unwrap();
        check(&source, &target).unwrap();
        assert_eq!(target.physical, source);
        assert_eq!(
            built_entries(&target),
            syn::parse_str::<structures::Structures>(&format!(
                "structures {{ {retained} = ::cq_storage::Trie<'_, _, ::std::collections::HashSet<_>> [{prefix}][c]; }}"
            ))
            .unwrap()
        );
        assert_eq!(
            syn::parse2::<structures::Program>(target.to_token_stream()).unwrap(),
            target
        );

        let entry = target
            .structures
            .entries
            .iter()
            .position(|e| matches!(e.structure, structures::Structure::Built { .. }))
            .unwrap();
        let mut missing = target.clone();
        missing.structures.entries.remove(entry);
        let diagnostic = structures::contract::check(&missing)
            .unwrap_err()
            .to_string();
        assert!(diagnostic.contains("two-level candidate"), "{diagnostic}");
        assert!(check(&source, &missing).is_err());
        for replacement in [
            format!("{retained} = ::cq_storage::HashIndex<'_, _, _> [{prefix},c];"),
            format!(
                "{retained} = ::cq_storage::Trie<'_, _, ::std::collections::HashSet<_>> [c][{prefix}];"
            ),
            format!("{retained} = ::std::collections::HashSet<_> [{prefix}][c];"),
        ] {
            let mut changed = target.clone();
            changed.structures.entries[entry] = syn::parse_str(&replacement).unwrap();
            assert!(
                structures::contract::check(&changed).is_err(),
                "{replacement}"
            );
            assert!(check(&source, &changed).is_err(), "{replacement}");
        }
        for reader in ["candidates", "membership"] {
            let mut missing = target.clone();
            missing.structures.entries.retain(|e| {
                !(e.name == host
                    && matches!(&e.structure, structures::Structure::Host { reader: r, .. } if r == reader))
            });
            assert!(
                structures::contract::check(&missing).is_err(),
                "{host}/{reader}"
            );
            assert!(check(&source, &missing).is_err(), "{host}/{reader}");
        }
    }
}

#[test]
fn candidate_levels_coexist_with_payload_membership_and_ordered_reads() {
    let source: physical::Program = syn::parse_quote!(physical {
        relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
        l0 = pipe rename L@0 {a -> a,b -> b};
        r0 = pipe rename R@1 {b -> b,c -> c}; r = let project r0 keep {b,c};
        s = pipe rename S@2 {a -> a,c -> c};
        payload = pipe probe first [b] l0,r;
        l1 = pipe project payload keep {a,b};
        members = let semi eager [c] s,r;
        l2 = pipe semi eager [a] l1,members;
        ordered = let merge[b,c] r,r;
        l = pipe semi eager [b] l2,ordered;
        candidate = pipe probe smallest [b] l,r;
        checked = pipe canon semi eager [a,c] candidate,s;
        return checked as answer(a,b,c);
    });
    let target = compile(&source).unwrap();
    assert_eq!(
        built_entries(&target),
        syn::parse_quote!(structures {
            r = ::cq_storage::Trie<'_, _, ::std::collections::HashSet<_>> [b][c];
            r = ::cq_storage::HashIndex<'_, _, _> [b];
            r = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [b,c];
            r = ::std::collections::HashSet<_> [c];
            members = ::std::collections::HashSet<_> [a];
            ordered = ::std::collections::HashSet<_> [b];
        })
    );
    let mut reordered = target.clone();
    reordered.structures.entries.reverse();
    for target in [&target, &reordered] {
        check(&source, target).unwrap();
        let analysis = structures::contract::check(target).unwrap();
        assert_eq!(
            analysis.built_on("r", &["b".into()]).unwrap(),
            (&syn::parse_quote!(::cq_storage::HashIndex<'_, _, _>), 1)
        );
        assert_eq!(
            analysis.built_on("r", &["b".into(), "c".into()]).unwrap(),
            (
                &syn::parse_quote!(::std::collections::BTreeMap<_, ::std::vec::Vec<_>>),
                1
            )
        );
    }
    for missing in 0..4 {
        let mut changed = target.clone();
        changed.structures.entries.remove(missing);
        assert!(
            check(&source, &changed).is_err(),
            "missing built entry {missing}"
        );
    }
}

#[test]
fn nonemptiness_and_keyed_membership_share_a_trie_with_an_empty_first_level() {
    let p: physical::Program = syn::parse_quote! { physical {
        relation L(a: i32); relation R(a: i32);
        l = pipe rename L@0 { a -> x }; r = let rename R@1 { a -> x };
        unit = pipe unit;
        nonempty = pipe semi eager [] unit,r;
        members = pipe probe first [] nonempty,l;
        result = pipe canon semi eager [x] members,r;
        return result as answer(x);
    }};
    let target = compile(&p).unwrap();
    assert_eq!(
        built_entries(&target),
        syn::parse_quote!(structures {
            r = ::cq_storage::Trie<'_, _, ::std::collections::HashSet<_>> [][x];
        })
    );
    check(&p, &target).unwrap();
}

fn built_entries(target: &structures::Program) -> structures::Structures {
    let mut entries = target.structures.clone();
    entries
        .entries
        .retain(|e| matches!(e.structure, structures::Structure::Built { .. }));
    entries
}

fn filtered_source() -> physical::Program {
    syn::parse_quote!(physical {
        relation L(k:i32); relation R(tag:String,k:i32,value:i32,unused:i32);
        l = pipe rename L@0 {k -> k};
        r = pipe rename R@1 {tag -> tag,k -> k,value -> value,unused -> unused};
        selected = let point select r if (tag = "a");
        joined = pipe probe first [k] l,selected;
        keys = pipe project joined keep {k};
        checked = pipe semi eager [k] keys,selected;
        again = pipe probe first [k] checked,selected;
        out = pipe canon project again keep {k,value};
        return out as answer(k,value);
    })
}

#[test]
fn filtered_readers_share_one_projected_index_and_check_column_coverage() {
    let source = filtered_source();
    let target = compile(&source).unwrap();
    let analysis = structures::contract::check(&target).unwrap();
    assert!(!analysis.retained.contains_key("selected"));
    assert_eq!(analysis.built["selected"].len(), 1);
    let entry = target
        .structures
        .entries
        .iter()
        .position(|e| e.name == "selected")
        .unwrap();
    assert_eq!(
        target.structures.entries[entry],
        syn::parse_quote!(selected = ::cq_storage::HashIndex<'_, _, _> [k] payload [k,value] constants {tag = "a"};)
    );
    assert_eq!(
        syn::parse2::<structures::Program>(target.to_token_stream()).unwrap(),
        target
    );
    for replacement in [
        syn::parse_quote!(selected = ::cq_storage::HashIndex<'_, _, _> [k] payload [k] constants {tag = "a"};),
        syn::parse_quote!(selected = ::cq_storage::HashIndex<'_, _, _> [value] payload [k,value] constants {tag = "a"};),
        syn::parse_quote!(selected = ::cq_storage::HashIndex<'_, _, _> [k] payload [k,value] constants {tag = "b"};),
        syn::parse_quote!(selected = ::cq_storage::HashIndex<'_, _, _> [k] payload [k,value,tag] constants {tag = "a"};),
    ] {
        let mut bad = target.clone();
        bad.structures.entries[entry] = replacement;
        assert!(check(&source, &bad).is_err());
        assert!(structures::check_rewrite(&target, &bad).is_err());
    }
    let mut bad = target.clone();
    bad.structures
        .entries
        .push(syn::parse_quote!(selected rows = ::std::vec::Vec<_>;));
    assert!(check(&source, &bad).is_err());
    let mut changed_occurrence = target.clone();
    let physical::Operator::Rename(rename) =
        &mut changed_occurrence.physical.program.definitions[1].operator
    else {
        unreachable!()
    };
    rename.occurrence = syn::parse_quote!(2);
    // The filtered layout still covers every read; correspondence must also
    // preserve the identity of the source occurrence supplying those rows.
    structures::contract::check(&changed_occurrence).unwrap();
    assert!(check(&source, &changed_occurrence).is_err());
    assert!(structures::check_rewrite(&target, &changed_occurrence).is_err());
    let mut reordered = target.clone();
    reordered.physical.program.definitions.swap(0, 1);
    check(&source, &reordered).unwrap();
    structures::check_rewrite(&target, &reordered).unwrap();
    let mut wrong_graph = target;
    wrong_graph.physical.program.definitions[2].operator =
        syn::parse_quote!(point select r if (tag = "b"));
    assert!(structures::check_rewrite(&reordered, &wrong_graph).is_err());
}

#[test]
fn scanned_selections_keep_rows_and_reject_a_missing_rows_decision() {
    let mut source = filtered_source();
    // The selection supplies outer rows too, so it needs a reusable row scan.
    source.program.definitions[3].operator = syn::parse_quote!(probe first [k] selected,l);
    let target = compile(&source).unwrap();
    let analysis = structures::contract::check(&target).unwrap();
    assert!(analysis.retained.contains_key("selected"));
    assert!(!analysis.filtered.contains_key("selected"));
    let mut bad = target;
    bad.structures.entries.retain(|e| {
        e.name != "selected" || !matches!(e.structure, structures::Structure::Rows { .. })
    });
    assert!(check(&source, &bad).is_err());
}

#[test]
fn range_filters_project_only_columns_demanded_downstream() {
    let mut source = filtered_source();
    source.program.definitions[2].operator = syn::parse_quote!(range select r if (value > 0));
    source.program.definitions[7].operator = syn::parse_quote!(project again keep {k});
    source.program.head = syn::parse_quote!(answer(k));
    let target = compile(&source).unwrap();
    let analysis = structures::contract::check(&target).unwrap();
    assert!(!analysis.retained.contains_key("selected"));
    assert_eq!(
        analysis.filtered["selected"],
        syn::parse_quote!(payload [k] constants {})
    );
    assert_eq!(analysis.layouts["joined"], ["k"]);
}

#[test]
fn filtered_payload_covers_rust_uses_and_keeps_opaque_environments() {
    for (guard, expected) in [
        ("(*unused > 0)", vec!["k", "unused", "value"]),
        ("(accept!(value))", vec!["k", "unused", "value"]),
    ] {
        let mut source = filtered_source();
        source.program.definitions.insert(
            4,
            syn::parse_str(&format!("guarded = pipe guard joined if {guard};")).unwrap(),
        );
        source.program.definitions[5].operator = syn::parse_quote!(project guarded keep {k});
        let target = compile(&source).unwrap();
        let analysis = structures::contract::check(&target).unwrap();
        let payload: Vec<_> = analysis.filtered["selected"]
            .payload
            .columns
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(payload, expected);
    }
}

#[test]
fn a_later_merge_does_not_remove_an_earlier_scan_requirement() {
    let source: physical::Program = syn::parse_quote!(physical {
        relation R(a:i32);
        r0 = pipe rename R@0 {a -> a}; r = let point select r0 if (a = 1);
        rows = let canon project r keep {a};
        joined = pipe merge[a] r,rows;
        out = pipe canon project joined keep {a}; return out as answer(a);
    });
    let target = compile(&source).unwrap();
    let analysis = structures::contract::check(&target).unwrap();
    assert!(analysis.physical.scanned.contains("r"));
    assert!(analysis.retained.contains_key("r"));
    let mut bad = target;
    bad.structures
        .entries
        .retain(|e| e.name != "r" || !matches!(e.structure, structures::Structure::Rows { .. }));
    assert!(check(&source, &bad).is_err());
}
