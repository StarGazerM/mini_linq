//! Authored modules check plan contracts independently of selection.
use cq_lang_structures as structures;
use quote::ToTokens;

fn program() -> structures::Program {
    syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32);
        l = pipe rename L@0 {a -> x}; r = let rename R@1 {a -> x,b -> y};
        j = pipe canon probe first [x] l,r; return j as answer(x,y);
    } structures { r = UserIndex [x]; r rows = ::std::vec::Vec<_>; r host scan = ::core::iter::Map<_, _> []; l host scan = ::core::iter::Map<_, _> []; j canon = ::std::collections::HashSet<_>; } }
}

#[test]
fn only_built_lets_have_entries_and_host_reads_make_no_claims() {
    let p = program();
    let a = structures::contract::check(&p).unwrap();
    assert!(a.host_read("l"));
    assert!(!a.host_read("r"));
    assert!(a.built_on("r", &["x".into()]).is_some());
    assert_eq!(a.host_layouts["R"], ["a", "b"]);
    assert!(!a.unique.contains_key("l"));
    assert!(!a.unique.contains_key("r"));
    assert_eq!(
        syn::parse2::<structures::Program>(p.to_token_stream()).unwrap(),
        p
    );
    for entries in [
        "r = I [y];",
        "r = I [x]; r = J [x];",
        "l = I [x];",
        "R = I [a];",
        "r = I [x][x];",
        "r = I [absent];",
        "",
    ] {
        let mut q = p.clone();
        q.structures = syn::parse_str(&format!("structures {{ {entries} }}")).unwrap();
        assert!(structures::contract::check(&q).is_err(), "{entries}");
    }
    let mut q = p;
    q.physical.program.definitions[1].mode = syn::parse_quote!(pipe);
    assert!(structures::contract::check(&q).is_err());
    q.structures = syn::parse_quote!(structures {
        l host scan = ::core::iter::Map<_, _> [];
        r host scan = ::core::iter::Map<_, _> [];
        r host j = UserIndex [x]; j canon = ::std::collections::HashSet<_>;
    });
    structures::contract::check(&q).unwrap();
}

#[test]
fn canonicality_comes_only_from_plan_boundaries_and_key_preservation() {
    let p: structures::Program = syn::parse_quote! { physical {
        relation R(a:i32,b:i32);
        r = let canon rename R@0 {a -> a,b -> b};
        e = pipe bind r with n: i32 = (1);
        p = pipe project e keep {a,b}; return p as answer(a,b);
    } structures { r canon = ::std::collections::HashSet<_>; r rows = ::std::vec::Vec<_>; r host scan = ::core::iter::Map<_, _> []; } };
    let a = structures::contract::check(&p).unwrap();
    assert_eq!(a.unique["p"], ["a", "b"]);
    let mut q = p.clone();
    q.physical.program.definitions[0].canon = None;
    assert!(structures::contract::check(&q).is_err());
    let mut q = p;
    q.physical.program.definitions[2] = syn::parse_quote!(p = pipe project e keep {a};);
    q.physical.program.head = syn::parse_quote!(answer(a));
    assert!(structures::contract::check(&q).is_err());
}

#[test]
fn ordered_premises_require_storage_or_a_same_key_merge_stream() {
    let p: structures::Program = syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32); relation S(b:i32);
        l = pipe rename L@0 {a -> a}; r = pipe rename R@1 {a -> a,b -> b}; s = pipe rename S@2 {b -> b};
        j = let merge[a] l,r; k = pipe canon merge[b] j,s; return k as answer(a,b);
    } structures {
        j = Ordered [b]; j rows = ::std::vec::Vec<_>; k canon = ::std::collections::HashSet<_>;
        l host scan = ::core::iter::Map<_, _> []; r host scan = ::core::iter::Map<_, _> []; s host scan = ::core::iter::Map<_, _> [];
        l host j = Ordered [a]; r host j = Ordered [a]; s host k = Ordered [b];
    } };
    structures::contract::check(&p).unwrap();
    let mut q = p.clone();
    q.structures.entries.clear();
    assert!(structures::contract::check(&q).is_err());
    q.physical.program.definitions[3].mode = syn::parse_quote!(pipe);
    assert!(structures::contract::check(&q).is_err());
    let mut q = p;
    q.physical.program.definitions[3].mode = syn::parse_quote!(pipe);
    assert!(structures::contract::check(&q).is_err());
}

#[test]
fn leveled_entries_cover_prefixes_but_do_not_prove_uniqueness() {
    let mut p = program();
    p.structures.entries[0] = syn::parse_quote!(r = MyTrie [][x];);
    let a = structures::contract::check(&p).unwrap();
    assert_eq!(a.built_on("r", &["x".into()]).unwrap().1, 2);
    assert_eq!(a.built_on("r", &[]).unwrap().1, 1);
    p.physical.program.definitions[2].canon = None;
    assert!(structures::contract::check(&p).is_err());
}

#[test]
fn complete_read_structures_take_precedence_over_candidate_prefixes() {
    let source: structures::Program = syn::parse_quote!(physical {
        relation L(x:i32); relation R(x:i32,y:i32);
        l = pipe rename L@0 {x -> x}; r = let rename R@1 {x -> x,y -> y};
        j = pipe probe first [x] l,r;
        checked = pipe canon probe first [x,y] j,r;
        return checked as answer(x,y);
    } structures {
        r = Curried [x][y]; r = Full [x,y]; r = Payload [x];
        r rows = ::std::vec::Vec<_>; r host scan = ::core::iter::Map<_, _> [];
        l host scan = ::core::iter::Map<_, _> [];
        checked canon = ::std::collections::HashSet<_>;
    });
    let mut reordered = source.clone();
    reordered.structures.entries.reverse();
    for program in [&source, &reordered] {
        let analysis = structures::contract::check(program).unwrap();
        assert_eq!(
            analysis.built_on("r", &["x".into()]).unwrap(),
            (&syn::parse_quote!(Payload), 1)
        );
        assert_eq!(
            analysis.built_on("r", &["x".into(), "y".into()]).unwrap(),
            (&syn::parse_quote!(Full), 1)
        );
    }
    let mut missing = source.clone();
    missing.structures.entries.remove(2);
    assert!(
        structures::contract::check(&missing)
            .unwrap_err()
            .to_string()
            .contains("payload probe must read all built levels")
    );
    let mut duplicate = source.clone();
    duplicate
        .structures
        .entries
        .push(source.structures.entries[0].clone());
    assert!(structures::contract::check(&duplicate).is_err());
}

#[test]
fn a_const_only_read_cannot_justify_a_built_entry() {
    let p: structures::Program = syn::parse_quote! { physical {
        relation R(a: i32);
        r = let rename R@0 { a -> x };
        p = pipe canon point select r if (x = 5);
        return p as answer(x);
    } structures { r rows = ::std::vec::Vec<_>; r host scan = ::core::iter::Map<_, _> []; p canon = ::std::collections::HashSet<_>; } };
    structures::contract::check(&p).unwrap();
    let mut invalid = p;
    invalid
        .structures
        .entries
        .push(syn::parse_quote!(r = UserIndex [x];));
    let error = structures::contract::check(&invalid)
        .unwrap_err()
        .to_string();
    assert!(error.contains("const-only"), "{error}");
}

#[test]
fn const_reads_can_coexist_with_storage_justified_by_a_stream() {
    let p: structures::Program = syn::parse_quote! { physical {
        relation L(a: i32); relation R(a: i32);
        l = pipe rename L@0 { a -> x }; r = let rename R@1 { a -> x };
        p = let point select r if (x = 5);
        j = pipe probe first [x] l,r;
        out = pipe canon semi eager [x] j,p;
        return out as answer(x);
    } structures { r = UserIndex [x]; p = UserIndex [x]; r rows = ::std::vec::Vec<_>; r host scan = ::core::iter::Map<_, _> []; p rows = ::std::vec::Vec<_>; out canon = ::std::collections::HashSet<_>; l host scan = ::core::iter::Map<_, _> []; } };
    structures::contract::check(&p).unwrap();
}

#[test]
fn every_built_key_level_requires_the_leibniz_premise() {
    let p: structures::Program = syn::parse_quote! { physical {
        relation L(a: i32); relation R(a: i32, payload: f64);
        l = pipe rename L@0 { a -> x };
        r = let rename R@1 { a -> x, payload -> y };
        j = pipe probe first [x] l,r;
        out = pipe canon project j keep {x};
        return out as answer(x);
    } structures { r = UserTrie [x][y]; } };
    let error = structures::contract::check(&p).unwrap_err().to_string();
    assert!(
        error.contains("y") && error.contains("Eq") && error.contains("f64"),
        "{error}"
    );
}

#[test]
fn representation_and_stage_entries_round_trip() {
    for text in [
        "r repr = tuples;",
        "r repr = positions { H lineage { a -> c }, J lineage { b -> d } } owned [x, y];",
        "r stage = batch 128;",
        "s share = lookup b [x] first s force p;",
        "s repr = tuples witness { b [x] at s as ::cq_storage::RunCursor<'_, _> };",
        "s repr = positions { L lineage { x -> a } } owned [] witness { b [x] at s as Cursor };",
        "p stage = batch 32 expand;",
    ] {
        let entry: structures::Entry = syn::parse_str(text).unwrap();
        assert_eq!(
            syn::parse2::<structures::Entry>(entry.to_token_stream()).unwrap(),
            entry
        );
    }
}

fn represented_program() -> structures::Program {
    syn::parse_quote! { physical {
        relation R(a:i32,b:i32);
        r = pipe rename R@0 {a -> x,b -> y};
        s = pipe select r if (x >= 0);
        b = pipe bind s with z: i32 = (x + y);
        out = pipe canon project b keep {x,y,z};
        return out as answer(x,y,z);
    } structures {
        r host scan = ::core::iter::Map<_, _> [];
        out canon = ::std::collections::HashSet<_>;
    } }
}

#[test]
fn representation_and_staging_are_checked_additive_rewrites() {
    let source = represented_program();
    let defaults = structures::contract::check(&source).unwrap();
    assert!(defaults.repr.is_empty());
    assert!(defaults.stages.is_empty());
    let mut target = source.clone();
    target.structures.entries.extend([
        syn::parse_quote!(r repr = tuples;),
        syn::parse_quote!(out repr = positions { R lineage { x -> a, y -> b } } owned [z];),
        syn::parse_quote!(s stage = batch 128;),
    ]);
    structures::check_rewrite(&source, &target).unwrap();
    let facts = structures::contract::check(&target).unwrap();
    assert!(matches!(
        facts.repr["out"],
        structures::Repr::Positions { .. }
    ));
    assert_eq!(facts.stages["s"], 128);
}

#[test]
fn representation_rejects_uncovered_noncomputed_and_forged_columns() {
    for (entry, expected) in [
        (
            "out repr = positions { R lineage { x -> a } } owned [z];",
            "uncovered",
        ),
        (
            "out repr = positions { R lineage { x -> a, y -> b } } owned [x,z];",
            "computed",
        ),
        (
            "out repr = positions { R lineage { x -> a, y -> a } } owned [z];",
            "injective",
        ),
        (
            "out repr = positions { R lineage { x -> b, y -> a } } owned [z];",
            "declared host",
        ),
        (
            "out repr = positions { Missing lineage { x -> a, y -> b } } owned [z];",
            "declared host",
        ),
    ] {
        let mut program = represented_program();
        program
            .structures
            .entries
            .push(syn::parse_str(entry).unwrap());
        let error = structures::contract::check(&program)
            .unwrap_err()
            .to_string();
        assert!(error.contains(expected), "{entry}: {error}");
    }
}

#[test]
fn stages_reject_partial_steps_cursor_producers_and_zero_batches() {
    for entry in [
        syn::parse_quote!(b stage = batch 16;),
        syn::parse_quote!(out stage = batch 16;),
        syn::parse_quote!(r stage = batch 0;),
    ] {
        let mut program = represented_program();
        program.structures.entries.push(entry);
        let error = structures::contract::check(&program)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("partial") || error.contains("positive"),
            "{error}"
        );
    }
    let mut cursor = program();
    cursor
        .structures
        .entries
        .push(syn::parse_quote!(j stage = batch 16;));
    let error = structures::contract::check(&cursor)
        .unwrap_err()
        .to_string();
    assert!(error.contains("cursor"), "{error}");

    let mut guard = represented_program();
    guard.physical.program.definitions[1] = syn::parse_quote!(s = pipe guard r if (*x >= 0););
    guard
        .structures
        .entries
        .push(syn::parse_quote!(s stage = batch 16;));
    let error = structures::contract::check(&guard).unwrap_err().to_string();
    assert!(error.contains("partial"), "{error}");
}

#[test]
fn representation_and_stage_decisions_are_unique_per_result() {
    for entries in [
        vec![
            syn::parse_quote!(r repr = tuples;),
            syn::parse_quote!(r repr = tuples;),
        ],
        vec![
            syn::parse_quote!(s stage = batch 16;),
            syn::parse_quote!(s stage = batch 32;),
        ],
    ] {
        let mut program = represented_program();
        program.structures.entries.extend(entries);
        let error = structures::contract::check(&program)
            .unwrap_err()
            .to_string();
        assert!(error.contains("duplicate"), "{error}");
    }
}

#[test]
fn joined_representations_carry_both_bases_without_relaxing_read_lineage() {
    let mut program = program();
    program.structures.entries.push(syn::parse_quote!(
        j repr = positions { L lineage {x -> a}, R lineage {y -> b} } owned [];
    ));
    let facts = structures::contract::check(&program).unwrap();
    assert!(facts.repr.contains_key("j"));
    assert!(!facts.lineage.contains_key("j"));
}

#[test]
fn stages_accept_total_constraints_multiple_boundaries_and_retained_producers() {
    let mut program = represented_program();
    program.physical.program.definitions[1] = syn::parse_quote!(
        s = pipe select r if (#[total] {x = x} (*x >= 0));
    );
    program.structures.entries.extend([
        syn::parse_quote!(r stage = batch 8;),
        syn::parse_quote!(s stage = batch 16;),
    ]);
    structures::contract::check(&program).unwrap();

    program
        .structures
        .entries
        .retain(|entry| !matches!(entry.structure, structures::Structure::Stage { .. }));
    program.physical.program.definitions[2].mode = syn::parse_quote!(let);
    program.structures.entries.extend([
        syn::parse_quote!(b rows = ::std::vec::Vec<_>;),
        syn::parse_quote!(out stage = batch 16;),
    ]);
    structures::contract::check(&program).unwrap();
}

#[test]
fn generator_bind_round_trip_drops_inherited_uniqueness() {
    let source: structures::Program = syn::parse_quote! { physical {
        u = pipe unit;
        g = pipe bind u with for x = ([1, 1, 2]);
        return canon g as answer(x);
    } structures {
        g head = ::std::collections::HashSet<_>;
    } };
    let facts = structures::contract::check(&source).unwrap();
    assert!(!facts.unique.contains_key("g"));
    assert!(facts.physical.rows["g"].computed.contains("x"));
    assert_eq!(
        syn::parse2::<structures::Program>(source.to_token_stream()).unwrap(),
        source
    );
    let mut invalid = source.clone();
    invalid.physical.program.return_canon = None;
    assert!(structures::contract::check(&invalid).is_err());
    let mut invalid = source;
    invalid.physical.program.definitions[1].operator =
        syn::parse_quote!(bind u with for x = ([*x]));
    assert!(structures::contract::check(&invalid).is_err());
}

fn shared_program() -> structures::Program {
    syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32);
        l = pipe rename L@0 {a -> x};
        b = let rename R@1 {a -> x,b -> y};
        s = pipe semi eager [x] l,b;
        p = pipe canon probe first [x] s,b;
        return p as answer(x,y);
    } structures {
        l host scan = ::core::iter::Map<_, _> [];
        b host scan = ::core::iter::Map<_, _> [];
        b rows = ::std::vec::Vec<_>;
        b = ::cq_storage::SortedRun<_,_> [x];
        p canon = ::std::collections::HashSet<_>;
        s share = lookup b [x] first s force p;
        s repr = tuples witness { b [x] at s as ::cq_storage::RunCursor<'_, _> };
    } }
}

#[test]
fn lookup_sharing_witness_and_expansion_are_additive() {
    let mut target = shared_program();
    target
        .structures
        .entries
        .push(syn::parse_quote!(p stage = batch 32 expand;));
    let facts = structures::contract::check(&target).unwrap();
    assert_eq!(facts.shares["s"].force, "p");
    assert!(facts.witnesses.contains_key("s"));
    assert!(facts.expands.contains("p"));
    assert_eq!(
        syn::parse2::<structures::Program>(target.to_token_stream()).unwrap(),
        target
    );
    let mut source = target.clone();
    source.structures.entries.truncate(5);
    structures::check_rewrite(&source, &target).unwrap();
}

#[test]
fn sharing_rejects_probe_before_semi() {
    let mut p = shared_program();
    p.physical.program.definitions.swap(2, 3);
    assert!(structures::contract::check(&p).is_err());
}

#[test]
fn witness_requires_matching_share() {
    let mut p = shared_program();
    p.structures.entries.remove(5);
    assert!(
        structures::contract::check(&p)
            .unwrap_err()
            .to_string()
            .contains("requires share")
    );
}

#[test]
fn witness_cannot_escape_into_canon() {
    let mut p = shared_program();
    p.physical.program.definitions[2].canon = Some(Default::default());
    p.structures
        .entries
        .push(syn::parse_quote!(s canon = ::std::collections::HashSet<_>;));
    let error = structures::contract::check(&p).unwrap_err().to_string();
    assert!(error.contains("witness s for b escapes at s"), "{error}");
}

#[test]
fn witness_cannot_escape_into_head() {
    let mut p = shared_program();
    p.physical.program.returned = syn::parse_quote!(s);
    p.physical.program.head = syn::parse_quote!(answer(x));
    p.physical.program.definitions[2].canon = Some(Default::default());
    p.structures
        .entries
        .push(syn::parse_quote!(s canon = ::std::collections::HashSet<_>;));
    assert!(structures::contract::check(&p).is_err());
}

#[test]
fn witness_cannot_escape_into_retained_let() {
    let mut p = shared_program();
    p.physical.program.definitions[2].mode = syn::parse_quote!(let);
    p.structures
        .entries
        .push(syn::parse_quote!(s rows = ::std::vec::Vec<_>;));
    let error = structures::contract::check(&p).unwrap_err().to_string();
    assert!(error.contains("witness s for b escapes at s"), "{error}");
}

#[test]
fn expand_rejects_hash_index() {
    let mut p = shared_program();
    p.structures.entries[3] = syn::parse_quote!(b = ::cq_storage::HashIndex<'_,_,_> [x];);
    p.structures
        .entries
        .push(syn::parse_quote!(p stage = batch 16 expand;));
    assert!(
        structures::contract::check(&p)
            .unwrap_err()
            .to_string()
            .contains("range-cursor")
    );
}

#[test]
fn expand_cannot_stage_across_bind() {
    let mut p = shared_program();
    p.physical
        .program
        .definitions
        .push(syn::parse_quote!(q = pipe bind p with z:i32 = (1);));
    p.physical.program.returned = syn::parse_quote!(q);
    p.physical.program.head = syn::parse_quote!(answer(x, y, z));
    p.structures.entries.extend([
        syn::parse_quote!(p stage = batch 16 expand;),
        syn::parse_quote!(q stage = batch 16;),
    ]);
    assert!(
        structures::contract::check(&p)
            .unwrap_err()
            .to_string()
            .contains("partial")
    );
}

#[test]
fn witness_cannot_escape_into_union_arm() {
    let mut p = shared_program();
    p.physical.program.definitions[3] = syn::parse_quote!(p = pipe probe first [x] s,b;);
    p.physical.program.definitions.extend([
        syn::parse_quote!(q = pipe project p keep {x};),
        syn::parse_quote!(u = pipe canon union s,q;),
    ]);
    p.physical.program.returned = syn::parse_quote!(u);
    p.physical.program.head = syn::parse_quote!(answer(x));
    p.structures.entries[4] = syn::parse_quote!(u canon = ::std::collections::HashSet<_>;);
    assert!(structures::contract::check(&p).is_err());
}

#[test]
fn witness_cannot_escape_into_fused_prefix() {
    let mut p = shared_program();
    p.physical.program.definitions[3] = syn::parse_quote!(p = pipe probe first [x] s,b;);
    p.physical.program.definitions.extend([
        syn::parse_quote!(q = pipe probe first [x] s,b;),
        syn::parse_quote!(u = pipe canon union fused p,q block @2 free [x] interface [y] over s;),
    ]);
    p.physical.program.returned = syn::parse_quote!(u);
    p.physical.program.head = syn::parse_quote!(answer(x, y));
    p.structures.entries[4] = syn::parse_quote!(u canon = ::std::collections::HashSet<_>;);
    let error = structures::contract::check(&p).unwrap_err().to_string();
    assert!(error.contains("witness s for b escapes at s"), "{error}");
}
