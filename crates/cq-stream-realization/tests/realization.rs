//! Stream → Access: authored expectations for the realized region and its
//! structure statements, and Access-contract acceptance on every shared case.
#[allow(dead_code)]
#[path = "../../../test-support/relational/cases.rs"]
mod cases;

use cq_ir::{relational_plan as ra, stream};
use cq_stream_realization::{Endpoints, compile};
use quote::{ToTokens, quote};

fn endpoints() -> Endpoints {
    Endpoints {
        support_iter: syn::parse_quote!(mini_linq::access_to_support_iter),
    }
}

fn stream_of(plan: &ra::Module, policy: proc_macro2::TokenStream) -> stream::Module {
    let source = cq_index_policy::compile(plan).unwrap();
    let policy: cq_ir::policy::Policy = syn::parse2(policy).unwrap();
    let structures = cq_structure_selection::compile(&source, &policy).unwrap();
    cq_stream_lowering::compile(&structures, &policy).unwrap()
}

fn chain() -> ra::Module {
    syn::parse_quote! {
        struct Chain;
        relation L(a: i32);
        relation R(a: i32, b: i32);
        relation S(b: i32, c: i32);
        answer(a, c) :- L(a), R(a, b), S(b, c);
        relational {
            l = rename L {a -> a};
            r = rename R {a -> a, b -> b};
            s = rename S {b -> b, c -> c};
            j0 = natural_join l with r;
            j1 = natural_join j0 with s;
            p = project j1 keep {a, c};
            output p as answer(a, c);
        }
    }
}

fn text(tokens: impl ToTokens) -> String {
    tokens.to_token_stream().to_string()
}

#[test]
fn hash_chain_builds_two_indexes_and_one_flat_region() {
    let module = stream_of(&chain(), quote! { policy { L; R; S; } });
    let realized = compile(&module, &endpoints()).unwrap();
    // Each built structure is followed by the guard for the operation the
    // plan performs on it, naming the relation in any trait error.
    let statements: Vec<String> = realized
        .statements
        .iter()
        .map(text)
        .filter(|s| !s.starts_with(":: cq_storage :: require"))
        .collect();
    assert_eq!(statements.len(), 2);
    assert!(
        realized
            .statements
            .iter()
            .map(text)
            .any(|s| s == ":: cq_storage :: require :: seek (__cq_r_on_0) ;")
    );
    let expected_r: syn::Stmt = syn::parse_quote! {
        let __cq_r_on_0 = &<::cq_storage::HashIndex<_, _> as ::cq_storage::Build<_>>::build(
            ::core::iter::Iterator::map(
                mini_linq::access_to_support_iter! { r(a, b) => ((a, b,)) :- R(a, b) => bind (a, b,) <- (R.iter()); },
                |(a, b,)| ((a,), (b,))
            )
        );
    };
    assert_eq!(statements[0], text(expected_r));
    let expected: cq_ir::rust_access_plan::Plan = syn::parse_quote! {
        answer(a, c) => ((a, c,)) :-
            L(a) => bind (a,) <- (L.iter()),
            r(a, b) => bind (b,) <- (::cq_storage::IndexRead::lookup(&__cq_r_on_0, &(a,))),
            s(b, c) => bind (c,) <- (::cq_storage::IndexRead::lookup(&__cq_s_on_0, &(b,)));
    };
    assert_eq!(text(&realized.access), text(&expected));
}

#[test]
fn host_maps_are_read_in_place_with_cloned_keys_in_declared_order() {
    // R is declared (b, a) but the host map is keyed by `a`, which is R's
    // sorted column 0; S is keyed by b, declared first. No index is built.
    let plan: ra::Module = syn::parse_quote! {
        struct Hosted;
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
            p = project j1 keep {a, c};
            output p as answer(a, c);
        }
    };
    let module = stream_of(&plan, quote! { policy { L; R keyed [a]; S keyed [b]; } });
    let realized = compile(&module, &endpoints()).unwrap();
    assert!(realized.statements.is_empty());
    let expected: cq_ir::rust_access_plan::Plan = syn::parse_quote! {
        answer(a, c) => ((a, c,)) :-
            L(a) => bind (a,) <- (L.iter()),
            R(b, a) => bind (b,) <- (::cq_storage::IndexRead::lookup(&&R, &(::core::clone::Clone::clone(a),))),
            S(b, c) => bind (c,) <- (::cq_storage::IndexRead::lookup(&&S, &(::core::clone::Clone::clone(b),)));
    };
    assert_eq!(text(&realized.access), text(&expected));
}

#[test]
fn merge_policy_realizes_co_iteration_over_built_ordered_maps() {
    let module = stream_of(&chain(), quote! { policy { L; R; S; join = merge; } });
    let realized = compile(&module, &endpoints()).unwrap();
    let names: Vec<String> = realized
        .statements
        .iter()
        .map(|s| text(s).split_whitespace().nth(1).unwrap().to_owned())
        .filter(|s| s != "cq_storage")
        .collect();
    assert_eq!(
        names,
        ["__cq_l_on_0", "__cq_r_on_0", "__cq_s_on_0", "__cq_j0_on_1"]
    );
    let access = text(&realized.access);
    assert!(access.contains(":: cq_storage :: merge ("), "{access}");
    assert!(
        access.contains("| (__cq_k , __cq_l , __cq_r) | (__cq_k . 0 , __cq_l . 0 , __cq_r . 0 ,)"),
        "{access}"
    );
}

#[test]
fn every_shared_case_realizes_to_a_checked_access_region() {
    for case in cases::CASES {
        let plan: ra::Module = syn::parse_str(case.plan).unwrap();
        let hosts: proc_macro2::TokenStream = plan
            .program
            .inputs()
            .map(|d| {
                let name = &d.name;
                quote! { #name; }
            })
            .collect();
        for join in [quote!(), quote!(join = merge;)] {
            let module = stream_of(&plan, quote! { policy { #hosts #join } });
            // `compile` checks the Access contract of the region it returns.
            let realized = compile(&module, &endpoints())
                .unwrap_or_else(|e| panic!("{} under {join}: {e}", case.name));
            assert!(
                !realized.access.body.is_empty() || case.name == "scan",
                "{}",
                case.name
            );
        }
    }
}
