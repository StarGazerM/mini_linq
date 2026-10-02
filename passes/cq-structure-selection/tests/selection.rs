use cq_lang_physical as physical;
use cq_lang_structures as structures;
use cq_structure_selection::{check, compile};
use cq_testkit::corpus as scenarios;
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
        l = let rename L {a -> a}; r = let rename R {a -> a,b -> b}; s = pipe rename S {b -> b};
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
        let p: physical::Program = syn::parse_str(&format!("physical {{ relation L(a:i32); relation R(a:i32); l = pipe rename L {{a -> a}}; r = pipe rename R {{a -> a}}; j = pipe canon {op}; return j as answer(a); }}")).unwrap();
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
    let p: physical::Program = syn::parse_quote!(physical { relation R(a:i32); r = pipe rename R {a -> x}; w = pipe canon point select r if (x = 2); return w as answer(x); });
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
            "physical {{ relation R(a:i32); r = {mode} rename R {{a -> x}}; w = pipe canon point select r if (x = 2); return w as answer(x); }}"
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
        l = pipe rename L {a -> x}; r = let rename R {a -> x};
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
        l = pipe rename L {a -> a,b -> b}; r = let rename R {a -> a,b -> b};
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
        l = pipe rename L { a -> a, b -> b };
        a = let rename A { a -> a, c -> c };
        b = let rename B { b -> b, c -> c };
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
fn nonemptiness_and_keyed_membership_share_a_trie_with_an_empty_first_level() {
    let p: physical::Program = syn::parse_quote! { physical {
        relation L(a: i32); relation R(a: i32);
        l = pipe rename L { a -> x }; r = let rename R { a -> x };
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
