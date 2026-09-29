//! Authored Structures modules check the contract independently of selection:
//! every Physical premise is discharged, hosts are declared, entries are well
//! formed, the return is canonical.
use cq_lang_structures as structures;
use quote::quote;

fn module(
    physical: proc_macro2::TokenStream,
    entries: proc_macro2::TokenStream,
) -> structures::Program {
    syn::parse_quote! {
            physical {
    relation L(a: i32);
    relation R(b: i32, a: i32);
    relation S(b: i32, c: i32);
     #physical }
            structures { #entries }
        }
}

fn hash_chain() -> proc_macro2::TokenStream {
    quote! {
        l = alias rename L {a -> a}; r = alias rename R {b -> b,a -> a}; s = let rename S {b -> b,c -> c};
        j0 = pipe probe[a] l, r; j1 = pipe probe[b] j0, s;
        x = pipe semi[a] j1, l; p = pipe canon project x keep {a,c}; return p as answer(a,c);
    }
}

#[test]
fn keyed_reads_are_answered_by_host_keys_through_renames_or_built_types() {
    let ok = module(
        hash_chain(),
        quote! {
            L keyed [a];
            R keyed [a];
            S;
            s = ::cq_storage::HashIndex<_, _> [b];
        },
    );
    let analysis = structures::contract::check(&ok).unwrap();
    assert_eq!(
        analysis.host_key_for("r", &["a".into()]).unwrap().columns,
        ["a"]
    );
    assert!(analysis.host_key_for("r", &["b".into()]).is_none());
    assert!(analysis.built_on("s", &["b".into()]).is_some());
    assert_eq!(analysis.host_layouts["R"], ["b", "a"]);
    assert!(analysis.unique.contains_key("p"));
}

#[test]
fn canonical_return_is_inferred_from_canonical_hosts_through_key_keeping_operators() {
    // L, R, S canonical: joins keep uniqueness; the projection keeps a and c,
    // which is not a key of x (x's key is {a, b, c}), so p needs `canon`.
    let needs_flag = module(
        quote! {
            l = alias rename L {a -> a}; r = alias rename R {b -> b,a -> a}; s = let rename S {b -> b,c -> c};
            j0 = pipe probe[a] l, r; j1 = pipe probe[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe project x keep {a,c}; return p as answer(a,c);
        },
        quote! { L keyed [a] canon; R keyed [a] canon; S canon; s = ::cq_storage::HashIndex<_, _> [b]; },
    );
    let error = structures::contract::check(&needs_flag)
        .unwrap_err()
        .to_string();
    assert!(error.contains("p"), "{error}");
    // Without canonical hosts, even x is not unique; with the flag it is fine.
    let flagged = module(
        hash_chain(),
        quote! { L keyed [a]; R keyed [a]; S; s = ::cq_storage::HashIndex<_, _> [b]; },
    );
    let analysis = structures::contract::check(&flagged).unwrap();
    assert!(!analysis.unique.contains_key("x"));
    assert!(analysis.unique.contains_key("p"));
}

#[test]
fn merge_sides_are_ordered_by_structure_or_sorted_pipes() {
    let hosts = quote! { L keyed [a] ordered; R keyed [a] ordered; S keyed [b] ordered; };
    // j0 is sorted on a by its own merge, but j1 merges on b: rejected.
    let error = structures::contract::check(&module(
        quote! {
            l = alias rename L {a -> a}; r = alias rename R {b -> b,a -> a}; s = alias rename S {b -> b,c -> c};
            j0 = pipe merge[a] l, r; j1 = pipe merge[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe canon project x keep {a,c}; return p as answer(a,c);
        },
        hosts.clone(),
    ))
    .unwrap_err()
    .to_string();
    assert!(error.contains("j0"), "{error}");
    // Materialized on b, it is ordered on b.
    let ok = module(
        quote! {
            l = alias rename L {a -> a}; r = alias rename R {b -> b,a -> a}; s = alias rename S {b -> b,c -> c};
            j0 = let merge[a] l, r; j1 = pipe merge[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe canon project x keep {a,c}; return p as answer(a,c);
        },
        quote! { #hosts j0 = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [b]; },
    );
    structures::contract::check(&ok).unwrap();
    // A structure over a pipe is rejected.
    let error = structures::contract::check(&module(
        quote! {
            l = alias rename L {a -> a}; r = alias rename R {b -> b,a -> a}; s = alias rename S {b -> b,c -> c};
            j0 = pipe merge[a] l, r; j1 = pipe merge[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe canon project x keep {a,c}; return p as answer(a,c);
        },
        quote! { #hosts j0 = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [b]; },
    ))
    .unwrap_err()
    .to_string();
    assert!(error.contains("j0"), "{error}");
}

#[test]
fn contract_rejects_malformed_entries() {
    let cases: [(proc_macro2::TokenStream, &str); 5] = [
        (
            quote! { L keyed [a]; R keyed [a]; S; s = my::Index<_, _> [b]; s = ::cq_storage::HashIndex<_, _> [b]; },
            "s",
        ),
        (
            quote! { L keyed [a]; R; S; s = ::cq_storage::HashIndex<_, _> [b]; },
            "j0",
        ),
        (
            quote! { L keyed [a]; R keyed [a]; S; s = ::cq_storage::HashIndex<_, _> [c]; },
            "j1",
        ),
        (
            quote! { L keyed [a]; R keyed [a]; s = ::cq_storage::HashIndex<_, _> [b]; },
            "S",
        ),
        (
            quote! { L keyed [a]; R keyed [a]; S; s = ::cq_storage::HashIndex<_, _> [z]; },
            "s",
        ),
    ];
    for (entries, needle) in cases {
        let error = structures::contract::check(&module(hash_chain(), entries))
            .unwrap_err()
            .to_string();
        assert!(error.contains(needle), "{error}");
    }
}

#[test]
fn complete_program_round_trip_preserves_leveled_keys() {
    use quote::ToTokens;
    let program: structures::Program = syn::parse_quote! {
        physical {
            relation R(a:i32,b:i32); relation S(a:i32,b:i32);
            r = alias rename R {a -> x,b -> y}; s = let rename S {a -> x,b -> y};
            j = pipe canon semi[x,y] r,s; return j as answer(x,y);
        }
        structures { R; S; s = MyTrie [x][y]; }
    };
    let analysis = structures::contract::check(&program).unwrap();
    assert_eq!(analysis.built_on("s", &["x".into()]).unwrap().1, 1);
    assert_eq!(
        analysis.built_on("s", &["x".into(), "y".into()]).unwrap().1,
        2
    );
    assert_eq!(
        syn::parse2::<structures::Program>(program.to_token_stream()).unwrap(),
        program
    );
    let mut changed = program.clone();
    changed.structures.entries[2] = syn::parse_quote!(s = MyTrie [x][x];);
    assert!(
        structures::contract::check(&changed)
            .unwrap_err()
            .to_string()
            .contains("x")
    );
    let mut changed = program;
    changed.structures.entries.push(syn::parse_quote!(R;));
    assert!(
        structures::contract::check(&changed)
            .unwrap_err()
            .to_string()
            .contains("R")
    );
}
