use cq_lang_structures as structures;
use quote::{ToTokens, quote};

fn chain() -> structures::Program {
    syn::parse_quote! {
        physical {
            relation L(k:i32,v:i32);
            l = pipe rename L@0 {k -> k,v -> v};
            s = pipe select l if (k >= 2);
            p = pipe project s keep {v};
            c = pipe canon project p keep {v};
            return c as answer(v);
        }
        structures {
            l host scan = ::core::iter::Map<_, _> [];
            c canon = sort;
        }
    }
}

#[test]
fn default_is_identity_and_named_selection_records_its_prefix() {
    let source = chain();
    assert_eq!(cq_represent::compile(&source).unwrap(), source);
    let target = cq_represent::compile_with(&source, quote!(positions(p))).unwrap();
    cq_represent::check(&source, &target).unwrap();
    let facts = structures::contract::check(&target).unwrap();
    assert_eq!(
        facts.repr.keys().cloned().collect::<Vec<_>>(),
        ["l", "p", "s"]
    );
    assert!(target.to_token_stream().to_string().contains("owned []"));
    assert_eq!(
        cq_represent::compile_with(&source, quote!(positions)).unwrap(),
        target
    );
}

#[test]
fn joins_and_opaque_binds_end_the_positional_chain() {
    let source: structures::Program = syn::parse_quote! {
        physical {
            relation L(k:i32); relation R(k:i32,v:i32);
            l = pipe rename L@0 {k -> k};
            r = let rename R@1 {k -> k,v -> v};
            j = pipe probe first [k] l,r;
            b = pipe bind j with n: i32 = (*k + 1);
            c = pipe canon project b keep {k,v,n};
            return c as answer(k,v,n);
        }
        structures {
            l host scan = ::core::iter::Map<_, _> [];
            r host scan = ::core::iter::Map<_, _> [];
            r = UserIndex [k]; r rows = ::std::vec::Vec<_>;
            c canon = sort;
        }
    };
    let target = cq_represent::compile_with(&source, quote!(positions)).unwrap();
    let facts = structures::contract::check(&target).unwrap();
    assert_eq!(facts.repr.keys().cloned().collect::<Vec<_>>(), ["l"]);
    for arguments in [
        quote!(positions(j)),
        quote!(positions(b)),
        quote!(positions(missing)),
    ] {
        assert!(
            cq_represent::compile_with(&source, arguments)
                .unwrap_err()
                .to_string()
                .contains("cannot be carried as positions")
        );
    }
}

#[test]
fn checker_rejects_tampered_entries_and_unrelated_changes() {
    let source = chain();
    let mut target = cq_represent::compile_with(&source, quote!(positions)).unwrap();
    target.structures.entries.last_mut().unwrap().structure = syn::parse_quote!(repr = tuples);
    assert!(cq_represent::check(&source, &target).is_err());
    let mut target = cq_represent::compile_with(&source, quote!(positions)).unwrap();
    target.structures.entries.remove(1);
    assert!(cq_represent::check(&source, &target).is_err());
    let mut target = source.clone();
    target.structures.entries[0] = syn::parse_quote!(l host scan = Other [] ;);
    assert!(cq_represent::check(&source, &target).is_err());
}
