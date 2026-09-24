//! IX → Structures: demands are bound to catalogue entries, hosts serve what
//! they declare, and `merge` orders both operands.
use cq_ir::policy::Policy;
use cq_ir::relational_plan as ra;
use cq_structure_selection::{check, compile};
use quote::{ToTokens, quote};

fn chain() -> ra::Module {
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
    }
}

fn entries(policy: proc_macro2::TokenStream) -> (String, bool) {
    let source = cq_index_policy::compile(&chain()).unwrap();
    let policy: Policy = syn::parse2(policy).unwrap();
    let target = compile(&source, &policy).unwrap();
    let text = target
        .entries
        .iter()
        .map(|e| e.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join(" ");
    (text, check(&source, &target, &policy))
}

/// The expected entries, spelled as the IR spells them.
fn text(entries: proc_macro2::TokenStream) -> String {
    let module: cq_ir::structures::Module = syn::parse2({
        let source = cq_index_policy::compile(&chain()).unwrap();
        quote! { #source structures { #entries } }
    })
    .unwrap();
    module
        .entries
        .iter()
        .map(|e| e.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn defaults_preserve_constant_time_access_and_skip_host_served_demands() {
    // R keyed on `a` answers r's join demand in place; nothing else is
    // answered, so s gets a hash index and l a key set for the semijoin.
    let (text, ok) = entries(quote! { policy { L; R keyed [a]; S; } });
    assert!(ok);
    assert_eq!(
        text,
        self::text(quote! {
            L; R keyed [0]; S;
            l = ::std::collections::HashSet<_> [0];
            s = ::cq_storage::HashIndex<_, _> [0];
        })
    );
}

#[test]
fn merge_keys_both_join_operands_in_order_and_the_strongest_need_wins() {
    let (text, ok) = entries(quote! { policy { L; R; S; join = merge; } });
    assert!(ok);
    // l is both a merge operand and a semijoin reducer: one ordered
    // structure on `a` answers both.
    assert_eq!(
        text,
        self::text(quote! {
            L; R; S;
            l = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [0];
            r = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [0];
            s = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [0];
            j0 = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [1];
        })
    );
}

#[test]
fn policy_key_columns_are_names_and_must_exist() {
    let source = cq_index_policy::compile(&chain()).unwrap();
    let policy: Policy = syn::parse_quote! { policy { L; R keyed [z]; S; } };
    let err = compile(&source, &policy).unwrap_err();
    assert_eq!(err.to_string(), "`R` has no column `z`");
    let policy: Policy = syn::parse_quote! { policy { L; R; } };
    let err = compile(&source, &policy).unwrap_err();
    assert_eq!(err.to_string(), "policy must list host relation `S`");
}

#[test]
fn index_advice_names_any_rust_type_for_structures_built_over_that_relation() {
    let plan: ra::Module = syn::parse_quote! {
        struct Chain;
        relation L(a: i32);
        #[index(my_crate::Trie)]
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
    };
    let source = cq_index_policy::compile(&plan).unwrap();
    let policy: Policy = syn::parse_quote! { policy { L; R; S; } };
    let target = compile(&source, &policy).unwrap();
    assert!(check(&source, &target, &policy));
    let module: cq_ir::structures::Module = syn::parse2(quote! {
        #source structures {
            L; R; S;
            l = ::std::collections::HashSet<_> [0];
            r = my_crate::Trie [0];
            s = ::cq_storage::HashIndex<_, _> [0];
        }
    })
    .unwrap();
    assert_eq!(target.entries, module.entries);
}
