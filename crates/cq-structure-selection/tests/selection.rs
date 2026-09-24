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
    // R keyed on `a` (host sorted column 0) serves r's join demand; nothing
    // else is served, so s gets a hash and l a key set for the semijoin.
    let (text, ok) = entries(quote! {
        policy { L provides { iter }; R provides { seek @R[0] }; S provides { iter }; }
    });
    assert!(ok);
    assert_eq!(
        text,
        self::text(quote! {
            L = host provides { iter }; R = host provides { seek @R[0] }; S = host provides { iter };
            l = ::std::collections::HashSet<_> [0] provides { contains };
            s = ::cq_storage::HashIndex<_, _> [0] provides { seek, contains };
        })
    );
}

#[test]
fn merge_orders_both_join_operands_and_the_strongest_kind_wins() {
    let (text, ok) = entries(quote! {
        policy { L provides { iter }; R provides { iter }; S provides { iter }; join = merge; }
    });
    assert!(ok);
    // l is both a merge operand (ordered) and a semijoin reducer (key_set):
    // one ordered structure serves both.
    assert_eq!(
        text,
        self::text(quote! {
            L = host provides { iter }; R = host provides { iter }; S = host provides { iter };
            l = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [0] provides { ordered, seek, contains };
            r = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [0] provides { ordered, seek, contains };
            s = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [0] provides { ordered, seek, contains };
            j0 = ::std::collections::BTreeMap<_, ::std::vec::Vec<_>> [1] provides { ordered, seek, contains };
        })
    );
}

fn advised(
    attr: proc_macro2::TokenStream,
    policy: proc_macro2::TokenStream,
) -> syn::Result<String> {
    let plan: ra::Module = syn::parse_quote! {
        struct Chain;
        relation L(a: i32);
        #attr
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
    let policy: Policy = syn::parse2(policy).unwrap();
    let target = compile(&source, &policy)?;
    assert!(check(&source, &target, &policy));
    Ok(target
        .entries
        .iter()
        .filter(|e| matches!(e.structure, cq_ir::structures::Structure::Keyed(_)))
        .map(|e| e.to_token_stream().to_string())
        .collect::<Vec<_>>()
        .join(" "))
}

#[test]
fn index_advice_names_any_rust_type_and_must_claim_what_the_plan_needs() {
    let hosts =
        quote! { policy { L provides { iter }; R provides { iter }; S provides { iter }; } };
    // Any type: the compiler emits `Build` and the claimed traits, nothing else.
    assert_eq!(
        advised(
            quote! { #[index(my_crate::Trie<_, _> provides { seek })] },
            hosts.clone()
        )
        .unwrap(),
        text(quote! {
            l = ::std::collections::HashSet<_> [0] provides { contains };
            r = my_crate::Trie<_, _> [0] provides { seek };
            s = ::cq_storage::HashIndex<_, _> [0] provides { seek, contains };
        })
    );
    // A type that does not claim what the plan needs is an error, never replaced.
    let err = advised(
        quote! { #[index(my_crate::KeySet<_> provides { contains })] },
        hosts,
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "relation `R` advises a structure that does not claim seek needed by `r` on [0]"
    );
    // Nor does advice override a merge strategy's need for order.
    let err = advised(
        quote! { #[index(my_crate::Trie<_, _> provides { seek })] },
        quote! { policy { L provides { iter }; R provides { iter }; S provides { iter }; join = merge; } },
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "relation `R` advises a structure that does not claim ordered needed by `r` on [0]"
    );
}
