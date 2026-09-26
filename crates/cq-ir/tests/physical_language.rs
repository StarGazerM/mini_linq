//! Authored Physical modules check the contract: one physical form per RA
//! result, join columns by name, pipes read exactly once, keyed reads on lets.
use cq_ir::physical::{self, ModeKind};
use quote::quote;

fn module(definitions: proc_macro2::TokenStream) -> syn::Result<physical::Module> {
    syn::parse2(quote! {
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
        physical { #definitions }
    })
}

#[test]
fn hash_chain_is_well_formed_and_reads_are_recorded() {
    let module = module(quote! {
        l = alias rename;
        r = let rename;
        s = let rename;
        j0 = pipe probe[a] l, r;
        j1 = pipe probe[b] j0, s;
        x = pipe semi[a] j1, l;
        p = pipe canon project;
        return p;
    })
    .unwrap();
    let analysis = physical::contract::check(&module).unwrap();
    assert_eq!(analysis.modes["j0"], ModeKind::Pipe);
    assert_eq!(analysis.key_reads["r"], vec![("j0".to_string(), vec!["a".to_string()])]);
    assert_eq!(analysis.key_reads["l"], vec![("x".to_string(), vec!["a".to_string()])]);
    assert_eq!(analysis.hosts["r"], "R");
    assert_eq!(analysis.renames["r"]["a"], "a");
    assert!(analysis.canon.contains("p"));
    assert!(analysis.scanned.contains("l") && analysis.scanned.contains("p"));
    assert!(!analysis.scanned.contains("r"));
    assert_eq!(physical::ordinals(&analysis, "r", &["a".into()]), vec![0]);
    assert_eq!(physical::erase(&module), module.index_requirements);
}

#[test]
fn merge_records_both_sides_and_allows_a_typed_pipe_side() {
    let module = module(quote! {
        l = let rename;
        r = let rename;
        s = let rename;
        j0 = pipe merge[a] l, r;
        j1 = let merge[b] j0, s;
        x = pipe semi[a] j1, l;
        p = pipe canon project;
        return p;
    })
    .unwrap();
    let analysis = physical::contract::check(&module).unwrap();
    assert_eq!(analysis.merge_reads["j0"], vec![("j1".to_string(), vec!["b".to_string()])]);
    assert_eq!(analysis.merge_reads["r"], vec![("j0".to_string(), vec!["a".to_string()])]);
    // j1 is read through a key by x, so it is a let even though merged once.
    assert_eq!(analysis.modes["j1"], ModeKind::Let);
}

#[test]
fn contract_rejects_wrong_forms() {
    let rejects = |definitions, needle: &str| {
        let error = module(definitions)
            .and_then(|m| physical::contract::check(&m))
            .err()
            .expect("rejected")
            .to_string();
        assert!(error.contains(needle), "{error}");
    };
    rejects(
        quote! {
            l = alias rename; r = pipe rename; s = let rename;
            j0 = pipe probe[a] l, r; j1 = pipe probe[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe canon project; return p;
        },
        "read through a key, so it must be `let`",
    );
    rejects(
        quote! {
            l = alias rename; r = let rename; s = let rename;
            j0 = pipe probe[b] l, r; j1 = pipe probe[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe canon project; return p;
        },
        "join columns are",
    );
    rejects(
        quote! {
            l = alias rename; r = let rename; s = let rename;
            j0 = pipe select; j1 = pipe probe[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe canon project; return p;
        },
        "is a natural join in RA",
    );
    rejects(
        quote! {
            l = alias rename; r = let rename; s = let rename;
            j0 = pipe probe[a] l, r; j1 = pipe probe[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe canon project; return x;
        },
        "RA outputs `p`",
    );
    rejects(
        quote! {
            l = alias rename; r = let rename; s = let rename;
            j0 = alias probe[a] l, r; j1 = pipe probe[b] j0, s;
            x = pipe semi[a] j1, l; p = pipe canon project; return p;
        },
        "only a rename of a host",
    );
}
