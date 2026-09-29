//! Authored Structures modules check the contract independently of selection:
//! every Physical premise is discharged, hosts are declared, entries are well
//! formed, the return is canonical.
use cq_lang_structures as structures;
use quote::quote;

fn module(
    physical: proc_macro2::TokenStream,
    entries: proc_macro2::TokenStream,
) -> structures::Module {
    syn::parse_quote! {
        struct Chain;
        relation L(a: i32);
        relation R(b: i32, a: i32);
        relation S(b: i32, c: i32);
        answer(a, c) :- L(a), R(b, a), S(b, c);
        relational {
            l = rename L {a -> a};
            r = rename R {b -> b, a -> a};
            s = rename S {b -> b, c -> c};
            j0 = natural_join l with r;
            j1 = natural_join j0 with s;
            x = semijoin j1 with l;
            p = project x keep {a, c};
            output p as answer(a, c);
        }
        indexes { i0 = @r[0]; i1 = @s[0]; i2 = @l[0]; }
        physical { #physical }
        structures { #entries }
    }
}

fn hash_chain() -> proc_macro2::TokenStream {
    quote! {
        l = alias rename; r = alias rename; s = let rename;
        j0 = pipe probe[a] l, r; j1 = pipe probe[b] j0, s;
        x = pipe semi[a] j1, l; p = pipe canon project; return p;
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
    let analysis = structures::contract::check_module(&ok).unwrap();
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
            l = alias rename; r = alias rename; s = let rename;
            j0 = pipe probe[a] l, r; j1 = pipe probe[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe project; return p;
        },
        quote! { L keyed [a] canon; R keyed [a] canon; S canon; s = ::cq_storage::HashIndex<_, _> [b]; },
    );
    let error = structures::contract::check_module(&needs_flag)
        .unwrap_err()
        .to_string();
    assert!(error.contains("not canonical"), "{error}");
    // Without canonical hosts, even x is not unique; with the flag it is fine.
    let flagged = module(
        hash_chain(),
        quote! { L keyed [a]; R keyed [a]; S; s = ::cq_storage::HashIndex<_, _> [b]; },
    );
    let analysis = structures::contract::check_module(&flagged).unwrap();
    assert!(!analysis.unique.contains_key("x"));
    assert!(analysis.unique.contains_key("p"));
}

#[test]
fn merge_sides_are_ordered_by_structure_or_sorted_pipes() {
    let hosts = quote! { L keyed [a] ordered; R keyed [a] ordered; S keyed [b] ordered; };
    // j0 is sorted on a by its own merge, but j1 merges on b: rejected.
    let error = structures::contract::check_module(&module(
        quote! {
            l = alias rename; r = alias rename; s = alias rename;
            j0 = pipe merge[a] l, r; j1 = pipe merge[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe canon project; return p;
        },
        hosts.clone(),
    ))
    .unwrap_err()
    .to_string();
    assert!(error.contains("merges `j0` on [\"b\"]"), "{error}");
    // Materialized on b, it is ordered on b.
    let ok = module(
        quote! {
            l = alias rename; r = alias rename; s = alias rename;
            j0 = let merge[a] l, r; j1 = pipe merge[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe canon project; return p;
        },
        quote! { #hosts j0 = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [b]; },
    );
    structures::contract::check_module(&ok).unwrap();
    // A structure over a pipe is rejected.
    let error = structures::contract::check_module(&module(
        quote! {
            l = alias rename; r = alias rename; s = alias rename;
            j0 = pipe merge[a] l, r; j1 = pipe merge[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe canon project; return p;
        },
        quote! { #hosts j0 = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [b]; },
    ))
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("is a pipe; a structure is built only over a `let`"),
        "{error}"
    );
}

#[test]
fn contract_rejects_malformed_entries() {
    let cases: [(proc_macro2::TokenStream, &str); 5] = [
        (
            quote! { L keyed [a]; R keyed [a]; S; s = my::Index<_, _> [b]; s = ::cq_storage::HashIndex<_, _> [b]; },
            "duplicate structure on these columns",
        ),
        (
            quote! { L keyed [a]; R; S; s = ::cq_storage::HashIndex<_, _> [b]; },
            "`j0` reads `r` on [\"a\"]",
        ),
        (
            quote! { L keyed [a]; R keyed [a]; S; s = ::cq_storage::HashIndex<_, _> [c]; },
            "`j1` reads `s` on [\"b\"]",
        ),
        (
            quote! { L keyed [a]; R keyed [a]; s = ::cq_storage::HashIndex<_, _> [b]; },
            "`S` has no entry",
        ),
        (
            quote! { L keyed [a]; R keyed [a]; S; s = ::cq_storage::HashIndex<_, _> [z]; },
            "`s` has no column `z`",
        ),
    ];
    for (entries, needle) in cases {
        let error = structures::contract::check_module(&module(hash_chain(), entries))
            .unwrap_err()
            .to_string();
        assert!(error.contains(needle), "{error}");
    }
}
