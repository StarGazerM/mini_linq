use cq_lang_structures as structures;
use quote::{ToTokens, quote};

fn chain() -> structures::Program {
    syn::parse_quote! {
        physical {
            relation L(k:i32,v:i32);
            l = pipe rename L {k -> k,v -> v};
            s = pipe select l if (k >= 2);
            p = pipe project s keep {v};
            c = let canon project p keep {v};
            return c as answer(v);
        }
        structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @host_scan as ::core::iter::Map<_, _> [];
            c rows = @sort as ::cq_storage::dedup::Sort; c canon = rows;
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
            l = pipe rename L {k -> k};
            r = let rename R {k -> k,v -> v};
            j = pipe probe first [k] l,r;
            b = pipe bind j with n: i32 = (*k + 1);
            c = let canon project b keep {k,v,n};
            return c as answer(k,v,n);
        }
        structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; user_index = UserIndex { reads [exact, range, membership], build inferred }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
            l host scan = @host_scan as ::core::iter::Map<_, _> [];
            r host scan = @host_scan as ::core::iter::Map<_, _> [];
            r = @user_index as UserIndex [k]; r rows = @vec as ::std::vec::Vec<_>;
            c rows = @sort as ::cq_storage::dedup::Sort; c canon = rows;
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
        assert!(cq_represent::compile_with(&source, arguments).is_err());
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
    target.structures.entries[0] = syn::parse_quote!(l host scan = @other as Other [] ;);
    assert!(cq_represent::check(&source, &target).is_err());
}
