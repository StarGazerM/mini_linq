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
        "L = host provides { iter } ; R = host provides { seek @ R [0] } ; S = host provides { iter } ; l = key_set [0] ; s = hash [0] ;"
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
        "L = host provides { iter } ; R = host provides { iter } ; S = host provides { iter } ; l = ordered [0] ; r = ordered [0] ; s = ordered [0] ; j0 = ordered [1] ;"
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
fn index_advice_on_a_relation_fixes_the_kind_of_structures_built_over_its_rename() {
    let hosts =
        quote! { policy { L provides { iter }; R provides { iter }; S provides { iter }; } };
    assert_eq!(
        advised(quote! { #[index(ordered)] }, hosts.clone()).unwrap(),
        "l = key_set [0] ; r = ordered [0] ; s = hash [0] ;"
    );
    // Advice that cannot serve the demand is an error, not a replacement.
    let err = advised(quote! { #[index(key_set)] }, hosts).unwrap_err();
    assert_eq!(
        err.to_string(),
        "relation `R` advises `key_set` indexes, but `r` needs `hash` on [0]"
    );
    // Nor does advice override a merge strategy's need for order.
    let err = advised(
        quote! { #[index(hash)] },
        quote! { policy { L provides { iter }; R provides { iter }; S provides { iter }; join = merge; } },
    )
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "relation `R` advises `hash` indexes, but `r` needs `ordered` on [0]"
    );
}
