use cq_lang_physical as physical;
use cq_lang_structures as structures;
use cq_structure_selection::{check, compile};
#[path = "../../../test-support/relational/scenarios.rs"]
mod scenarios;
#[test]
fn scenarios_discharge_every_physical_read_and_reject_changed_targets() {
    for case in scenarios::SCENARIOS {
        let source = syn::parse_str(case.physical).unwrap();
        let policy = syn::parse_str(case.policy).unwrap();
        let target = compile(&source, &policy).unwrap();
        check(&source, &target, &policy).unwrap();
        assert_eq!(
            target.structures,
            syn::parse_str::<structures::Structures>(case.structures).unwrap(),
            "{}",
            case.name
        );
        let mut changed = target;
        changed.physical.program.head.relation = syn::parse_quote!(mutated);
        assert!(check(&source, &changed, &policy).is_err());
    }
}

fn chain() -> physical::Program {
    syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32); relation S(b:i32);
        l = let rename L {a -> a}; r = let rename R {a -> a,b -> b}; s = pipe rename S {b -> b};
        j = pipe probe[a] l,r; x = pipe semi[a] j,l; p = pipe canon semi[b] x,s; return p as answer(a,b);
    } }
}

#[test]
fn provider_slot_types_and_preservation_are_checked_independently() {
    let source = chain();
    let policy = syn::parse_quote!(policy {});
    let target = compile(&source, &policy).unwrap();
    assert_eq!(
        target.structures,
        syn::parse_quote!(structures { l = index_provider!(semi) [a]; r = index_provider!(probe) [a]; })
    );
    check(&source, &target, &policy).unwrap();
    let mut reordered = target.clone();
    reordered.structures.entries.reverse();
    check(&source, &reordered, &policy).unwrap();
    for replacement in [
        syn::parse_quote!(l = index_provider!(probe) [a];),
        syn::parse_quote!(l = Custom [a];),
    ] {
        let mut q = target.clone();
        q.structures.entries[0] = replacement;
        assert!(check(&source, &q, &policy).is_err());
    }
    let mut q = target.clone();
    q.structures.entries.remove(1);
    assert!(check(&source, &q, &policy).is_err());
    let mut q = target.clone();
    q.structures
        .entries
        .push(syn::parse_quote!(s = index_provider!(semi) [b];));
    assert!(check(&source, &q, &policy).is_err());
    let mut q = target;
    q.physical.program.declarations[0].value = Some(syn::parse_quote!(= (rows())));
    assert!(check(&source, &q, &policy).is_err());
}

#[test]
fn host_renames_have_no_structure_entries_for_any_read_kind() {
    for op in ["probe[a] l,r", "semi[a] l,r", "merge[a] l,r"] {
        let p: physical::Program = syn::parse_str(&format!("physical {{ relation L(a:i32); relation R(a:i32); l = pipe rename L {{a -> a}}; r = pipe rename R {{a -> a}}; j = pipe canon {op}; return j as answer(a); }}")).unwrap();
        let policy = syn::parse_quote!(policy {});
        let target = compile(&p, &policy).unwrap();
        assert!(target.structures.entries.is_empty());
        check(&p, &target, &policy).unwrap();
    }
    let p: physical::Program = syn::parse_quote!(physical { relation R(a:i32); r = pipe rename R {a -> x}; w = pipe canon point select r if (x = 2); return w as answer(x); });
    let target = compile(&p, &syn::parse_quote!(policy {})).unwrap();
    assert!(target.structures.entries.is_empty());
}

#[test]
fn const_supplies_on_host_and_materialized_inputs_never_build() {
    for mode in ["pipe", "let"] {
        let p: physical::Program = syn::parse_str(&format!(
            "physical {{ relation R(a:i32); r = {mode} rename R {{a -> x}}; w = pipe canon point select r if (x = 2); return w as answer(x); }}"
        )).unwrap();
        let policy = syn::parse_quote!(policy {});
        let target = compile(&p, &policy).unwrap();
        assert!(target.structures.entries.is_empty());
        check(&p, &target, &policy).unwrap();
        let mut unnecessary = target;
        unnecessary
            .structures
            .entries
            .push(syn::parse_quote!(r = index_provider!(probe) [x];));
        assert!(check(&p, &unnecessary, &policy).is_err());
    }
}

#[test]
fn ordered_read_types_and_changed_key_streams_have_mutation_coverage() {
    let p: physical::Program = syn::parse_quote!(physical {
        relation L(a:i32); relation R(a:i32);
        l = pipe rename L {a -> x}; r = let rename R {a -> x};
        w = pipe canon merge[x] l,r; return w as answer(x);
    });
    let policy = syn::parse_quote!(policy {});
    let target = compile(&p, &policy).unwrap();
    assert_eq!(
        target.structures,
        syn::parse_quote!(structures { r = index_provider!(ordered) [x]; })
    );
    let mut q = target;
    q.structures.entries[0] = syn::parse_quote!(r = index_provider!(probe) [x];);
    assert!(check(&p, &q, &policy).is_err());
}

#[test]
fn compatible_membership_chains_use_provider_selected_trie_levels() {
    let p: physical::Program = syn::parse_quote!(physical {
        relation L(a:i32,b:i32); relation R(a:i32,b:i32);
        l = pipe rename L {a -> a,b -> b}; r = let rename R {a -> a,b -> b};
        p = pipe project l keep {a}; x = pipe semi[a] p,r;
        j = pipe canon probe[a,b] r,r;
        k = pipe canon semi[a] j,x; return k as answer(a,b);
    });
    // x is a keyed intermediate and must be retained.
    let mut p = p;
    p.program.definitions[3].mode = syn::parse_quote!(let);
    let policy = syn::parse_quote!(policy {});
    let target = compile(&p, &policy).unwrap();
    check(&p, &target, &policy).unwrap();
    let entry = target
        .structures
        .entries
        .iter()
        .position(|e| e.name == "r")
        .unwrap();
    assert_eq!(
        target.structures.entries[entry],
        syn::parse_quote!(r = index_provider!(trie index_provider!(probe)) [a][b];)
    );
    let mut q = target;
    q.structures.entries[entry] = syn::parse_quote!(r = index_provider!(probe) [a,b];);
    assert!(check(&p, &q, &policy).is_err());
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
        p = pipe probe[a] l,a;
        result = pipe canon semi[b,c] p,b;
        return result as answer(a,b,c);
    }};
    let policy = syn::parse_quote!(policy { intersect = smallest; });
    let target = compile(&source, &policy).unwrap();
    check(&source, &target, &policy).unwrap();
    let entry = target
        .structures
        .entries
        .iter()
        .position(|e| {
            let structures::Structure::Built { keyed, .. } = &e.structure;
            keyed.levels.len() == 2
        })
        .unwrap();
    let mut changed = target.clone();
    let structures::Structure::Built { keyed, .. } =
        &mut changed.structures.entries[entry].structure;
    keyed.levels.reverse();
    assert!(check(&source, &changed, &policy).is_err());
    let mut changed = target;
    let structures::Structure::Built { keyed, .. } =
        &mut changed.structures.entries[entry].structure;
    keyed.ty = syn::parse_quote!(index_provider!(semi));
    assert!(check(&source, &changed, &policy).is_err());
}

#[test]
fn nonemptiness_and_keyed_membership_share_a_trie_with_an_empty_first_level() {
    let p: physical::Program = syn::parse_quote! { physical {
        relation L(a: i32); relation R(a: i32);
        l = pipe rename L { a -> x }; r = let rename R { a -> x };
        unit = pipe unit;
        nonempty = pipe semi[] unit,r;
        members = pipe probe[] nonempty,l;
        result = pipe canon semi[x] members,r;
        return result as answer(x);
    }};
    let policy = syn::parse_quote!(policy {});
    let target = compile(&p, &policy).unwrap();
    assert_eq!(
        target.structures,
        syn::parse_quote!(structures {
            r = index_provider!(trie index_provider!(semi)) [][x];
        })
    );
    check(&p, &target, &policy).unwrap();
}
