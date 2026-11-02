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
                out = let canon project observed keep {x,y};
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
        syn::parse_quote!(@radix_run as ::cq_storage::RadixRun<_, _, ::std::vec::Vec<_>>)
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
                p = let canon probe first [x] l,r;
                return dedup p as answer(*y);
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
        syn::parse_quote!(@radix_run as ::cq_storage::RadixRun<_, _, ::std::vec::Vec<_>>)
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
            answer(a,b) :- L(a), R(a,b), where b >= 0;
        }
    };
}

#[test]
fn named_pipeline_composes_and_executes() {
    let captured = named_query!(left, right;
        structures = [cq_rewrites::radix_runs, cq_rewrites::retain_lookups, cq_rewrites::expand_lookups(32)];
        print_input = structures;
        print = cq_rewrite_test_driver::capture_structures;
    );
    let program: ir::Program = syn::parse_str(captured).unwrap();
    let facts = ir::contract::check(&program).unwrap();
    assert!(
        facts
            .built
            .values()
            .flatten()
            .any(|(_, provider)| *provider
                == syn::parse_quote!(@radix_run as ::cq_storage::RadixRun<_, _, ::std::vec::Vec<_>>))
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
        p = let canon probe first [x] l,r;
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
        syn::parse_quote!(KeyAlias),
        syn::parse_quote!(keys::CustomKey),
        syn::parse_quote!(String),
        syn::parse_quote!(i128),
        syn::parse_quote!(usize),
    ] {
        let source = single_key_source(ty, quote!(let));
        let target = radix_runs(&source, quote!()).unwrap();
        assert_eq!(
            ir::contract::check(&source).unwrap().built["r"][0].1,
            syn::parse_quote!(@hash_index as ::cq_storage::HashIndex<'_, _, _>)
        );
        assert_eq!(
            ir::contract::check(&target).unwrap().built["r"][0].1,
            syn::parse_quote!(@radix_run as ::cq_storage::RadixRun<_, _, ::std::vec::Vec<_>>)
        );
        assert_eq!(source.physical, target.physical);
        assert_eq!(radix_runs(&target, quote!()).unwrap(), target);
        // Recorded singleton layout selects the provider; rustc, not this
        // rewrite, establishes OrderKey for concrete keys and aliases.
        assert_eq!(
            cq_rewrite_passes::invoke_structures(
                &source,
                &syn::parse_quote!(cq_rewrites::radix_runs),
                quote!(),
            )
            .unwrap(),
            target
        );
        assert!(radix_runs(&source, quote!(12, 32)).is_err());
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
        p = let canon probe first [x,y] l,r;
        return p as answer(x,y,z);
    }})
    .unwrap();
    assert_eq!(radix_runs(&composite, quote!()).unwrap(), composite);
    let membership = cq_structure_selection::compile(&syn::parse_quote! { physical {
        relation L(a:i64); relation R(a:i64);
        l = pipe rename L@0 {a -> x};
        r = let rename R@1 {a -> x};
        p = let canon semi eager [x] l,r;
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
        checked = let canon probe first [a,c] candidate,s;
        return checked as answer(a,b,c);
    }})
    .unwrap();
    assert!(smallest.structures.entries.iter().any(|entry| matches!(
        &entry.structure, Structure::Built { keyed, .. } if keyed.levels.len() > 1
    )));
    smallest
        .structures
        .entries
        .push(syn::parse_quote!(r = @hash_index as ::cq_storage::HashIndex<'_, _, _> [b];));
    smallest
        .structures
        .entries
        .push(syn::parse_quote!(r rows = @vec as ::std::vec::Vec<_>;));
    smallest
        .structures
        .declare(&cq_lang_structures::provider::base::declarations());
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

macro_rules! partial_reader {
    ($left:expr, $right:expr; $($pipeline:tt)*) => {
        mini_linq::passes::select! {
            pipeline { $($pipeline)* };
            physical {
                relation L(a:i64) = ($left); relation R(a:i64,b:i64,c:i64) = ($right);
                l = pipe rename L@0 {a -> x};
                r = let rename R@1 {a -> x,b -> y,c -> z};
                p = pipe probe first [x] l,r;
                g = pipe guard p if (*y > 0);
                k = pipe project g keep {x};
                q = pipe probe first [x] k,r;
                out = let canon project q keep {x,y,z};
                return out as answer(x,y,z);
            }
        }
    };
}

/// SEL-ItemLayout: the first probe carries only `y` of the `{y, z}` payload,
/// so the run stores its payload column-wise; a complete-row reader alone,
/// as in every test above, keeps `items rows`.
#[test]
fn partial_payload_reader_selects_tuple_columns() {
    let captured = partial_reader!(left, right;
        cq_rewrites::radix_runs, cq_rewrite_test_driver::capture_structures
    );
    let program: ir::Program = syn::parse_str(captured).unwrap();
    let facts = ir::contract::check(&program).unwrap();
    assert_eq!(
        facts.built["r"][0].1,
        syn::parse_quote!(@radix_run as ::cq_storage::RadixRun<_, _, ::cq_storage::TupleColumns<_>>)
    );
    assert!(program.structures.entries.iter().any(|entry| matches!(
        &entry.structure,
        Structure::Built { keyed, .. }
            if matches!(keyed.items.as_ref().map(|items| &items.layout), Some(ir::ItemLayout::Columns(_)))
    )));
    let left = [(1_i64,), (2,), (3,)];
    let right = [
        (1_i64, 5_i64, 7_i64),
        (1, -1, 8),
        (2, -2, 9),
        (3, 4, 10),
        (3, 4, 11),
    ];
    let run = |rows: Vec<(&i64, &i64, &i64)>| -> BTreeSet<(i64, i64, i64)> {
        rows.into_iter().map(|(x, y, z)| (*x, *y, *z)).collect()
    };
    let radix = run(partial_reader!(&left, &right;
        cq_rewrites::radix_runs, mini_linq::passes::sequence,
        mini_linq::passes::sequential, mini_linq::passes::translate_pull
    )
    .collect());
    let baseline = run(partial_reader!(&left, &right;
        mini_linq::passes::sequence, mini_linq::passes::sequential,
        mini_linq::passes::translate_pull
    )
    .collect());
    assert_eq!(radix, baseline);
    assert_eq!(
        radix,
        BTreeSet::from([(1, 5, 7), (1, -1, 8), (3, 4, 10), (3, 4, 11)])
    );
    // A column layout needs a split payload: recording it on a one-field
    // payload is rejected by the Structures checker.
    let source = single_key_source(syn::parse_quote!(i64), quote!(let));
    let mut target = radix_runs(&source, quote!()).unwrap();
    for entry in &mut target.structures.entries {
        if let Structure::Built { keyed, .. } = &mut entry.structure {
            keyed.items = Some(syn::parse_quote!(items columns));
        }
    }
    assert!(ir::contract::check(&target).is_err());
}

/// STR-ItemLayout: the recorded `items` fact is the only source of the
/// provider's layout argument. The instance spelling ends before the declared
/// slot; the checked analysis prints the declared spelling of the fact.
#[test]
fn recorded_item_layout_alone_prints_the_layout_argument() {
    let captured = partial_reader!(left, right;
        cq_rewrites::radix_runs, cq_rewrite_test_driver::capture_structures
    );
    let program: ir::Program = syn::parse_str(captured).unwrap();
    let run = |program: &ir::Program| {
        program
            .structures
            .entries
            .iter()
            .position(|entry| matches!(&entry.structure, Structure::Built { keyed, .. } if keyed.provider.token == "radix_run"))
            .unwrap()
    };
    let keyed = |program: &mut ir::Program| {
        let index = run(program);
        match &mut program.structures.entries[index].structure {
            Structure::Built { keyed, .. } => keyed.as_mut().clone(),
            _ => unreachable!(),
        }
    };
    let set = |program: &mut ir::Program, value: ir::Keyed| {
        let index = run(program);
        if let Structure::Built { keyed, .. } = &mut program.structures.entries[index].structure {
            **keyed = value;
        }
    };
    let recorded = keyed(&mut program.clone());
    assert_eq!(
        recorded.provider,
        syn::parse_quote!(@radix_run as ::cq_storage::RadixRun<_, _>)
    );
    // Flipping the fact flips the printed argument.
    let mut rows = program.clone();
    let mut flipped = recorded.clone();
    flipped.items = Some(syn::parse_quote!(items rows));
    set(&mut rows, flipped);
    assert_eq!(
        ir::contract::check(&rows).unwrap().built["r"][0].1,
        syn::parse_quote!(@radix_run as ::cq_storage::RadixRun<_, _, ::std::vec::Vec<_>>)
    );
    // A spelling stating the layout itself, or a missing fact, is rejected.
    let mut spelled = program.clone();
    let mut authored = recorded.clone();
    authored.provider.ty = syn::parse_quote!(::cq_storage::RadixRun<_, _, ::std::vec::Vec<_>>);
    set(&mut spelled, authored);
    assert!(ir::contract::check(&spelled).is_err());
    let mut missing = program.clone();
    let mut absent = recorded;
    absent.items = None;
    set(&mut missing, absent);
    assert!(ir::contract::check(&missing).is_err());
    // Retype advice states the layout with the replacement provider.
    let retyped = cq_rewrite_passes::retype(
        &program,
        quote!(r => @radix_run as ::cq_storage::RadixRun<_, _> supports [expand] items rows),
    )
    .unwrap();
    assert_eq!(
        ir::contract::check(&retyped).unwrap().built["r"][0].1,
        syn::parse_quote!(@radix_run as ::cq_storage::RadixRun<_, _, ::std::vec::Vec<_>>)
    );
    assert!(
        cq_rewrite_passes::retype(
            &program,
            quote!(r => @radix_run as ::cq_storage::RadixRun<_, _> supports [expand]),
        )
        .is_err()
    );
}
