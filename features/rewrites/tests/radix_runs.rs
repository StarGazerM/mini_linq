use cq_lang_structures::{self as ir, Structure};
use cq_rewrite_passes::{expand_lookups, radix_runs, retain_lookups};
use quote::quote;
use std::{cell::RefCell, collections::BTreeSet};

macro_rules! payload {
    ($ty:ty, $left:expr, $right:expr, $visits:ident; $($pipeline:tt)*) => {
        mini_linq::passes::select! {
            pipeline { $($pipeline)* };
            physical {
                relation L(a:$ty) = ($left); relation R(a:$ty,b:i64) = ($right);
                l = pipe rename L@0 {a -> x};
                r = let rename R@1 {a -> x,b -> y};
                s = pipe semi eager [x] l,r;
                p = pipe probe first [x] s,r;
                observed = pipe guard p if ({ $visits.borrow_mut().push((*x, *y)); true });
                out = pipe canon project observed keep {x,y};
                return out as answer(x,y);
            }
        }
    };
}

fn assert_plan(captured: &str, retained: bool, staged: bool) -> ir::Program {
    let program: ir::Program = syn::parse_str(captured).unwrap();
    let facts = ir::contract::check(&program).unwrap();
    assert_eq!(facts.built["r"].len(), 1);
    assert_eq!(
        facts.built["r"][0].1,
        syn::parse_quote!(::cq_storage::RadixRun<_, _>)
    );
    assert!(!facts.membership.contains(&("r".into(), vec!["x".into()])));
    assert_eq!(facts.shares.contains_key("s"), retained);
    assert_eq!(facts.witnesses.contains_key("s"), retained);
    assert_eq!(facts.expands.contains("p"), staged);
    assert_eq!(facts.stages.get("p").copied(), staged.then_some(32));
    if retained {
        assert_eq!(facts.shares["s"].force, "p");
    }
    program
}

macro_rules! numeric_execution {
    ($name:ident, $ty:ty) => {
        #[test]
        fn $name() {
            let baseline: ir::Program = syn::parse_str(payload!($ty, left, right, visits;
                cq_rewrite_test_driver::capture_structures
            )).unwrap();
            for (captured, retained, staged) in [
                (payload!($ty, left, right, visits;
                    cq_rewrites::radix_runs, cq_rewrite_test_driver::capture_structures
                ), false, false),
                (payload!($ty, left, right, visits;
                    cq_rewrites::radix_runs, cq_rewrites::retain_lookups,
                    cq_rewrite_test_driver::capture_structures
                ), true, false),
                (payload!($ty, left, right, visits;
                    cq_rewrites::radix_runs, cq_rewrites::expand_lookups(32),
                    cq_rewrite_test_driver::capture_structures
                ), false, true),
                (payload!($ty, left, right, visits;
                    cq_rewrites::radix_runs, cq_rewrites::retain_lookups,
                    cq_rewrites::expand_lookups(32), cq_rewrite_test_driver::capture_structures
                ), true, true),
            ] {
                let rewritten = assert_plan(captured, retained, staged);
                ir::check_rewrite(&baseline, &rewritten).unwrap();
                assert_eq!(rewritten.physical, baseline.physical);
            }
            for (left, right) in [
                (vec![], vec![]),
                (vec![(0 as $ty,), (<$ty>::MIN,), (<$ty>::MAX,)], vec![]),
                (vec![(1 as $ty,), (2,), (3,), (<$ty>::MIN,), (<$ty>::MAX,), (1,)],
                 vec![(3 as $ty, 30i64), (1, 10), (1, 11), (1, 10),
                      (<$ty>::MIN, -40), (<$ty>::MAX, 50)]),
                (
                    (0..291).map(|i| ((i * i * 17) as $ty,)).collect(),
                    (0..257).flat_map(|i| {
                        let key = (i * i * 17) as $ty;
                        [(key, i as i64), (key, -(i as i64))]
                    }).collect(),
                ),
            ] {
                let expected: Vec<_> = left.iter().flat_map(|&(key,)| {
                    right.iter().filter_map(move |&(other, value)| {
                        (key == other).then_some((key, value))
                    })
                }).collect();
                let answers: BTreeSet<_> = expected.iter().copied().collect();
                let visits = &RefCell::new(Vec::new());
                let default: BTreeSet<_> = payload!($ty, &left, &right, visits;
                    mini_linq::passes::sequence, mini_linq::passes::sequential,
                    mini_linq::passes::translate_pull
                ).map(|(x, y)| (*x, *y)).collect();
                assert_eq!(default, answers);
                assert_eq!(*visits.borrow(), expected);
                visits.borrow_mut().clear();
                let immediate: BTreeSet<_> = payload!($ty, &left, &right, visits;
                    cq_rewrites::radix_runs, mini_linq::passes::sequence,
                    mini_linq::passes::sequential, mini_linq::passes::translate_pull
                ).map(|(x, y)| (*x, *y)).collect();
                assert_eq!(immediate, answers);
                assert_eq!(*visits.borrow(), expected);
                visits.borrow_mut().clear();
                let retained: BTreeSet<_> = payload!($ty, &left, &right, visits;
                    cq_rewrites::radix_runs, cq_rewrites::retain_lookups,
                    mini_linq::passes::sequence, mini_linq::passes::sequential,
                    mini_linq::passes::translate_pull
                ).map(|(x, y)| (*x, *y)).collect();
                assert_eq!(retained, answers);
                assert_eq!(*visits.borrow(), expected);
                visits.borrow_mut().clear();
                let staged: BTreeSet<_> = payload!($ty, &left, &right, visits;
                    cq_rewrites::radix_runs, cq_rewrites::expand_lookups(32),
                    mini_linq::passes::sequence, mini_linq::passes::sequential,
                    mini_linq::passes::translate_pull
                ).map(|(x, y)| (*x, *y)).collect();
                assert_eq!(staged, answers);
                assert_eq!(*visits.borrow(), expected);
                visits.borrow_mut().clear();
                let combined: BTreeSet<_> = payload!($ty, &left, &right, visits;
                    cq_rewrites::radix_runs, cq_rewrites::retain_lookups,
                    cq_rewrites::expand_lookups(32), mini_linq::passes::sequence,
                    mini_linq::passes::sequential, mini_linq::passes::translate_pull
                ).map(|(x, y)| (*x, *y)).collect();
                assert_eq!(combined, answers);
                assert_eq!(*visits.borrow(), expected);
            }
        }
    };
}

numeric_execution!(signed_32_immediate_retained_and_staged, i32);
numeric_execution!(signed_64_immediate_retained_and_staged, i64);
numeric_execution!(unsigned_32_immediate_retained_and_staged, u32);
numeric_execution!(unsigned_64_immediate_retained_and_staged, u64);

macro_rules! owned_probe {
    ($key:expr, $right:expr; $($pipeline:tt)*) => {
        mini_linq::passes::select! {
            pipeline { $($pipeline)* };
            physical {
                relation R(a:u64,b:i64) = ($right);
                u = pipe unit;
                l = pipe bind u with x:u64 = ($key);
                r = let rename R@0 {a -> x,b -> y};
                p = pipe canon probe first [x] l,r;
                return p as answer(*y);
            }
        }
    };
}

#[test]
fn owned_computed_wide_probes_execute_without_a_borrowed_codec() {
    let captured = owned_probe!(key, right;
        cq_rewrites::radix_runs, cq_rewrite_test_driver::capture_structures
    );
    let program: ir::Program = syn::parse_str(captured).unwrap();
    let facts = ir::contract::check(&program).unwrap();
    assert_eq!(
        facts.built["r"][0].1,
        syn::parse_quote!(::cq_storage::RadixRun<_, _>)
    );
    let right = [(0u64, 0i64), (u64::MAX, 10), (u64::MAX, 11), (u64::MAX, 10)];
    for key in [0, 1, i64::MAX as u64 + 1, u64::MAX] {
        let actual: BTreeSet<_> = owned_probe!(key, &right;
            cq_rewrites::radix_runs, mini_linq::passes::sequence,
            mini_linq::passes::sequential, mini_linq::passes::translate_pull
        )
        .collect();
        let expected = right
            .iter()
            .filter_map(|&(k, v)| (k == key).then_some((v,)))
            .collect();
        assert_eq!(actual, expected);
    }
}

macro_rules! named_query {
    ($left:expr, $right:expr; $($pipeline:tt)*) => {
        mini_linq::compile! {
            pipeline { $($pipeline)* }
            struct RadixQuery;
            relation L(a:i64) = ($left);
            relation R(a:i64,b:i64) = ($right);
            answer(a,b) :- L(a), R(a,b), if (*b >= 0);
        }
    };
}

#[test]
fn named_pipeline_composes_and_executes() {
    let captured = named_query!(left, right;
        structures = [cq_rewrites::radix_runs, cq_rewrites::retain_lookups, cq_rewrites::expand_lookups(32)];
        print = cq_rewrite_test_driver::capture_structures;
    );
    let program: ir::Program = syn::parse_str(captured).unwrap();
    let facts = ir::contract::check(&program).unwrap();
    assert!(
        facts
            .built
            .values()
            .flatten()
            .any(|(_, ty)| *ty == syn::parse_quote!(::cq_storage::RadixRun<_, _>))
    );
    assert!(!facts.expands.is_empty());
    let left = [(i64::MIN,), (-1,), (2,), (i64::MAX,)];
    let right = [(i64::MIN, 1i64), (-1, 2), (-1, -3), (i64::MAX, 4), (-1, 2)];
    let actual: BTreeSet<_> = named_query!(&left, &right;
        structures = [cq_rewrites::radix_runs, cq_rewrites::retain_lookups, cq_rewrites::expand_lookups(32)];
    ).map(|(a, b)| (*a, *b)).collect();
    let baseline: BTreeSet<_> = named_query!(&left, &right;)
        .map(|(a, b)| (*a, *b))
        .collect();
    assert_eq!(actual, baseline);
    assert_eq!(
        actual,
        BTreeSet::from([(i64::MIN, 1), (-1, 2), (i64::MAX, 4)])
    );
}

fn single_key_source(ty: syn::Type, mode: proc_macro2::TokenStream) -> ir::Program {
    let physical = syn::parse2(quote! { physical {
        relation L(a:#ty); relation R(a:#ty,b:i64);
        l = pipe rename L@0 {a -> x};
        r = #mode rename R@1 {a -> x,b -> y};
        p = pipe canon probe first [x] l,r;
        return p as answer(x,y);
    }})
    .unwrap();
    cq_structure_selection::compile(&physical).unwrap()
}

#[test]
fn selection_is_checked_idempotent_and_preserves_defaults() {
    for ty in [
        syn::parse_quote!(i32),
        syn::parse_quote!(i64),
        syn::parse_quote!(u32),
        syn::parse_quote!(u64),
    ] {
        let source = single_key_source(ty, quote!(let));
        let target = radix_runs(&source, quote!()).unwrap();
        assert_eq!(
            ir::contract::check(&source).unwrap().built["r"][0].1,
            syn::parse_quote!(::cq_storage::HashIndex<'_, _, _>)
        );
        assert_eq!(
            ir::contract::check(&target).unwrap().built["r"][0].1,
            syn::parse_quote!(::cq_storage::RadixRun<_, _>)
        );
        assert_eq!(source.physical, target.physical);
        assert_eq!(radix_runs(&target, quote!()).unwrap(), target);
        assert!(radix_runs(&source, quote!(12, 32)).is_err());
    }
    for ty in [
        syn::parse_quote!(String),
        syn::parse_quote!(i128),
        syn::parse_quote!(usize),
    ] {
        let source = single_key_source(ty, quote!(let));
        assert_eq!(radix_runs(&source, quote!()).unwrap(), source);
    }
    let host = single_key_source(syn::parse_quote!(i64), quote!(pipe));
    let facts = ir::contract::check(&host).unwrap();
    assert!(!facts.fallbacks.is_empty());
    assert_eq!(radix_runs(&host, quote!()).unwrap(), host);
}

#[test]
fn composite_membership_trie_and_smallest_providers_are_not_retyped() {
    let composite = cq_structure_selection::compile(&syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(a:i64,b:i64,c:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = let rename R@1 {a -> x,b -> y,c -> z};
        p = pipe canon probe first [x,y] l,r;
        return p as answer(x,y,z);
    }})
    .unwrap();
    assert_eq!(radix_runs(&composite, quote!()).unwrap(), composite);
    let membership = cq_structure_selection::compile(&syn::parse_quote! { physical {
        relation L(a:i64); relation R(a:i64);
        l = pipe rename L@0 {a -> x};
        r = let rename R@1 {a -> x};
        p = pipe canon semi eager [x] l,r;
        return p as answer(x);
    }})
    .unwrap();
    assert_eq!(radix_runs(&membership, quote!()).unwrap(), membership);
    let membership = cq_rewrite_passes::membership(&membership, quote!(r)).unwrap();
    assert_eq!(radix_runs(&membership, quote!()).unwrap(), membership);
    let mut smallest = cq_structure_selection::compile(&syn::parse_quote! { physical {
        relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
        l = pipe rename L@0 {a -> a,b -> b};
        r = let rename R@1 {b -> b,c -> c};
        s = let rename S@2 {a -> a,c -> c};
        candidate = pipe probe smallest [b] l,r;
        checked = pipe canon probe first [a,c] candidate,s;
        return checked as answer(a,b,c);
    }})
    .unwrap();
    assert!(smallest.structures.entries.iter().any(|entry| matches!(
        &entry.structure, Structure::Built { keyed, .. } if keyed.levels.len() > 1
    )));
    smallest
        .structures
        .entries
        .push(syn::parse_quote!(r = ::cq_storage::HashIndex<'_, _, _> [b];));
    smallest
        .structures
        .entries
        .push(syn::parse_quote!(r rows = ::std::vec::Vec<_>;));
    ir::contract::check(&smallest).unwrap();
    assert_eq!(radix_runs(&smallest, quote!()).unwrap(), smallest);
    assert_eq!(retain_lookups(&smallest, quote!()).unwrap(), smallest);
    assert_eq!(expand_lookups(&smallest, quote!(32)).unwrap(), smallest);
}

#[test]
fn radix_payload_remains_an_exact_join_filter_source() {
    let source = single_key_source(syn::parse_quote!(i64), quote!(let));
    let target = radix_runs(&source, quote!()).unwrap();
    let facts = ir::contract::check(&target).unwrap();
    let candidates = ir::join_filter_candidates(&target, &facts, "p");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].build, "r");
    let filtered = cq_rewrite_passes::join_filter(&target, quote!(l)).unwrap();
    assert_eq!(
        ir::contract::check(&filtered).unwrap().join_filters["l"].len(),
        1
    );
}
