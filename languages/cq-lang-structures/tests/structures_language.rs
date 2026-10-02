//! Authored modules check plan contracts independently of selection.
use cq_lang_structures as structures;
use quote::ToTokens;

fn program() -> structures::Program {
    syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32);
        l = pipe rename L {a -> x}; r = let rename R {a -> x,b -> y};
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
        r = let canon rename R {a -> a,b -> b};
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
        l = pipe rename L {a -> a}; r = pipe rename R {a -> a,b -> b}; s = pipe rename S {b -> b};
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
fn a_const_only_read_cannot_justify_a_built_entry() {
    let p: structures::Program = syn::parse_quote! { physical {
        relation R(a: i32);
        r = let rename R { a -> x };
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
        l = pipe rename L { a -> x }; r = let rename R { a -> x };
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
        l = pipe rename L { a -> x };
        r = let rename R { a -> x, payload -> y };
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
