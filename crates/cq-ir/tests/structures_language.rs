//! Authored Structures modules check the contract independently of selection:
//! every IX demand is served, hosts are declared, entries are well formed.
use cq_ir::capabilities::Operation;
use cq_ir::structures;
use quote::quote;

fn module(entries: proc_macro2::TokenStream) -> structures::Module {
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
        uses { j0.right = i0; j1.right = i1; x.right = i2; }
        structures { #entries }
    }
}

#[test]
fn demands_are_served_by_chosen_structures_or_translated_host_capabilities() {
    // R is declared (b, a): sorted [a, b], so its key `a` is host column 0,
    // which serves r's demand on r-ordinal 0 (`a`) through the rename.
    let ok = module(quote! {
        L keyed [0];
        R keyed [0];
        S;
        s = ::cq_storage::HashIndex<_, _> [0];
    });
    let analysis = structures::contract::check(&ok).unwrap();
    // A host key answers seek and membership in place, translated.
    assert_eq!(
        analysis.provided["r"],
        [
            Operation::Iter,
            Operation::Seek(vec![0]),
            Operation::Contains(vec![0])
        ]
    );
    // A built structure answers every operation on its key, structurally;
    // whether its type implements each is rustc's check.
    assert_eq!(
        analysis.provided["s"],
        [
            Operation::Seek(vec![0]),
            Operation::Contains(vec![0]),
            Operation::Ordered(vec![0])
        ]
    );
    assert_eq!(analysis.aliases["r"], "R");
    assert!(!analysis.aliases.contains_key("s"));

    let cases: [(&str, proc_macro2::TokenStream, &str); 6] = [
        (
            "duplicate structure",
            quote! {
                L keyed [0];
                R keyed [0];
                S;
                s = my::Index<_, _> [0];
                s = ::cq_storage::HashIndex<_, _> [0];
            },
            "duplicate structure on these columns",
        ),
        (
            "unserved join demand",
            quote! {
                L keyed [0];
                R;
                S;
                s = ::cq_storage::HashIndex<_, _> [0];
            },
            "no structure on `r` serves Seek([0])",
        ),
        (
            "a structure on other columns does not serve the demand",
            quote! {
                L keyed [0];
                R keyed [0];
                S;
                s = ::cq_storage::HashIndex<_, _> [1];
            },
            "no structure on `s` serves Seek([0])",
        ),
        (
            "missing host",
            quote! {
                L keyed [0];
                R keyed [0];
                s = ::cq_storage::HashIndex<_, _> [0];
            },
            "`S` has no entry",
        ),
        (
            "keyed structure on a host",
            quote! {
                L keyed [0];
                R keyed [0];
                S = ::cq_storage::HashIndex<_, _> [0];
                s = ::cq_storage::HashIndex<_, _> [0];
            },
            "represented by its host value",
        ),
        (
            "column out of range",
            quote! {
                L keyed [0];
                R keyed [0];
                S;
                s = ::cq_storage::HashIndex<_, _> [2];
            },
            "exceeds heading arity",
        ),
    ];
    for (name, entries, expected) in cases {
        let error = structures::contract::check(&module(entries))
            .expect_err(name)
            .to_string();
        assert!(error.contains(expected), "{name}: {error}");
    }
}
