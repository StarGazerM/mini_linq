use cq_lang_physical as physical;
use cq_lang_structures as structures;
use cq_structure_selection::{check, compile};
use cq_testkit::corpus as scenarios;
use quote::ToTokens;
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
