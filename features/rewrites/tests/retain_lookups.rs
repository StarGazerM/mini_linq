use cq_lang_structures::{self as ir, Structure};
use cq_rewrite_passes::{retain_lookups, sorted_runs};
use quote::{ToTokens, quote};

fn triangle_source() -> ir::Program {
    syn::parse_quote! { physical {
        relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
        l = pipe rename L@0 {a -> a,b -> b};
        r = let rename R@1 {b -> b,c -> c};
        pre_r = pipe semi eager [b] l,r;
        s = let rename S@2 {a -> a,c -> c};
        pre_s = pipe semi eager [a] pre_r,s;
        candidate = pipe probe smallest [b] pre_s,r;
        checked = pipe canon semi eager [a,c] candidate,s;
        return checked as answer(a,b,c);
    } structures {
        r = ::cq_gpu::PrefixRun [b][c];
        s = ::cq_gpu::PrefixRun [a][c];
        l host scan = ::core::iter::Map<_, _> [];
        r host scan = ::core::iter::Map<_, _> [];
        s host scan = ::core::iter::Map<_, _> [];
        r rows = ::std::vec::Vec<_>;
        s rows = ::std::vec::Vec<_>;
        checked canon = ::std::collections::HashSet<_>;
    } }
}

#[test]
fn triangle_records_both_prefix_witnesses_at_first_probe() {
    let source = triangle_source();
    let target = retain_lookups(&source, quote!()).unwrap();
    let facts = ir::contract::check(&target).unwrap();
    assert_eq!(facts.witnesses.len(), 2);
    assert_eq!(facts.shares.len(), 2);
    for (first, build, key) in [("pre_r", "r", "b"), ("pre_s", "s", "a")] {
        let share = &facts.shares[first];
        assert_eq!(share.build, build);
        assert_eq!(share.keys, [key]);
        assert_eq!(share.force, "candidate");
        assert_eq!(
            facts.witnesses[first].ty,
            syn::parse_quote!(::cq_storage::RunCursor<'_, _>)
        );
    }
    assert_eq!(source.physical, target.physical);
    assert_eq!(retain_lookups(&target, quote!()).unwrap(), target);
    assert_eq!(
        syn::parse2::<ir::Program>(target.to_token_stream()).unwrap(),
        target
    );
}

#[test]
fn leveled_host_lookup_retains_exact_snapshot_ranges() {
    let mut source: ir::Program = syn::parse_quote! { physical {
        relation L(a:i64); relation R(a:i64,b:i64);
        l = pipe rename L@0 {a -> x};
        r = pipe rename R@1 {a -> x,b -> y};
        s = pipe semi eager [x] l,r;
        p = pipe canon probe first [x] s,r;
        return p as answer(x,y);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        r host result(s) = ::cq_gpu_runtime::LeveledDeviceIndex<_,_,((::cq_gpu_runtime::FrozenKey<0>,),)> [x];
        r host result(p) = ::cq_gpu_runtime::LeveledDeviceIndex<_,_,((::cq_gpu_runtime::FrozenKey<0>,),)> [x];
        p canon = ::std::collections::HashSet<_>;
    } };
    let target = retain_lookups(&source, quote!()).unwrap();
    let facts = ir::contract::check(&target).unwrap();
    assert_eq!(facts.shares["s"].force, "p");
    assert!(facts.witnesses.contains_key("s"));
    assert_eq!(target.physical, source.physical);
    assert_eq!(retain_lookups(&target, quote!()).unwrap(), target);
    source.structures.entries[3] = syn::parse_quote! {
        r host result(p) = ::cq_gpu_runtime::LeveledDeviceIndex<_,_,((::cq_gpu_runtime::FrozenKey<1>,),)> [x];
    };
    assert!(retain_lookups(&source, quote!()).is_err());
}

#[test]
fn triangle_prefix_shares_reject_wrong_build_key_and_force() {
    let target = retain_lookups(&triangle_source(), quote!()).unwrap();
    for replacement in [
        syn::parse_quote!(pre_s share = lookup r [a] first pre_s force candidate;),
        syn::parse_quote!(pre_s share = lookup s [c] first pre_s force candidate;),
        syn::parse_quote!(pre_s share = lookup s [a] first pre_s force checked;),
    ] {
        let mut bad = target.clone();
        let slot = bad
            .structures
            .entries
            .iter_mut()
            .find(|entry| {
                entry.name == "pre_s" && matches!(entry.structure, Structure::Share { .. })
            })
            .unwrap();
        *slot = replacement;
        assert!(ir::contract::check(&bad).is_err());
    }
}

#[test]
fn triangle_prefix_witnesses_do_not_escape_or_infer_integral_keys() {
    let mut nonintegral = triangle_source();
    nonintegral.physical = syn::parse2(
        nonintegral
            .physical
            .to_token_stream()
            .to_string()
            .replace("i32", "bool")
            .parse()
            .unwrap(),
    )
    .unwrap();
    // Nonintegral directories are not retained even when the provider type is named.
    ir::contract::check(&nonintegral).unwrap();
    assert_eq!(retain_lookups(&nonintegral, quote!()).unwrap(), nonintegral);
    let mut escaping = triangle_source();
    escaping.physical.program.definitions[4].canon = Some(Default::default());
    escaping
        .structures
        .entries
        .push(syn::parse_quote!(pre_s canon = ::std::collections::HashSet<_>;));
    assert_eq!(retain_lookups(&escaping, quote!()).unwrap(), escaping);
}

fn hashed_source() -> ir::Program {
    let physical = syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32);
        l = pipe rename L@0 {a -> x};
        r = let rename R@1 {a -> x,b -> y};
        s = pipe semi eager [x] l,r;
        filtered = pipe select s if (x >= 0);
        projected = pipe project filtered keep {x};
        p = pipe probe first [x] projected,r;
        out = pipe canon project p keep {x,y};
        return out as answer(x,y);
    }};
    cq_structure_selection::compile(&physical).unwrap()
}

fn source() -> ir::Program {
    sorted_runs(&hashed_source(), quote!()).unwrap()
}

#[test]
fn records_one_checked_cursor_and_roundtrips_idempotently() {
    let source = source();
    assert!(ir::contract::check(&source).unwrap().shares.is_empty());
    let target = retain_lookups(&source, quote!()).unwrap();
    ir::check_rewrite(&source, &target).unwrap();
    let facts = ir::contract::check(&target).unwrap();
    assert_eq!(facts.shares.len(), 1);
    assert_eq!(facts.shares["s"].build, "r");
    assert_eq!(facts.shares["s"].keys, ["x"]);
    assert_eq!(facts.shares["s"].first, "s");
    assert_eq!(facts.shares["s"].force, "p");
    assert_eq!(facts.witnesses.len(), 1);
    assert_eq!(
        facts.witnesses["s"].ty,
        syn::parse_quote!(::cq_storage::RunCursor<'_, _>)
    );
    assert_eq!(source.physical, target.physical);
    assert_eq!(retain_lookups(&target, quote!()).unwrap(), target);
    assert_eq!(
        syn::parse2::<ir::Program>(target.to_token_stream()).unwrap(),
        target
    );
    assert!(retain_lookups(&source, quote!(unexpected)).is_err());
}

#[test]
fn preserves_positions_and_completes_an_existing_share() {
    let mut source = source();
    source.structures.entries.push(syn::parse_quote! {
        s repr = positions {L lineage {x -> a}} owned [];
    });
    source.structures.entries.push(syn::parse_quote! {
        s share = lookup r [x] first s force p;
    });
    let old_repr = ir::contract::check(&source).unwrap().repr["s"].clone();
    let target = retain_lookups(&source, quote!()).unwrap();
    let facts = ir::contract::check(&target).unwrap();
    assert_eq!(facts.repr["s"], old_repr);
    assert_eq!(facts.shares.len(), 1);
    assert_eq!(facts.witnesses.len(), 1);
    ir::check_rewrite(&source, &target).unwrap();
}

#[test]
fn a_named_sorted_run_type_is_also_eligible() {
    let mut source = source();
    for entry in &mut source.structures.entries {
        if let Structure::Built { keyed, .. } = &mut entry.structure {
            keyed.ty = syn::parse_quote!(SortedRun<_, _, ::cq_storage::BorrowedI64TupleCodec>);
        }
    }
    let target = retain_lookups(&source, quote!()).unwrap();
    assert_eq!(ir::contract::check(&target).unwrap().witnesses.len(), 1);
}

#[test]
fn hash_index_does_not_claim_a_copy_cursor() {
    let source = hashed_source();
    assert_eq!(retain_lookups(&source, quote!()).unwrap(), source);
}

#[test]
fn a_probe_before_the_semi_does_not_share() {
    let physical = syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32);
        l = pipe rename L@0 {a -> x};
        r = let rename R@1 {a -> x,b -> y};
        p = pipe probe first [x] l,r;
        seen = pipe select p if (y >= 0);
        keys = pipe project seen keep {x};
        s = pipe semi eager [x] keys,r;
        out = pipe canon project s keep {x};
        return out as answer(x);
    }};
    let source = cq_structure_selection::compile(&physical).unwrap();
    let source = sorted_runs(&source, quote!()).unwrap();
    assert_eq!(
        ir::contract::check(&source).unwrap().built["r"][0].1,
        syn::parse_quote!(::cq_storage::SortedRun<_, _, ::cq_storage::BorrowedI64TupleCodec>)
    );
    assert_eq!(retain_lookups(&source, quote!()).unwrap(), source);
}

#[test]
fn retained_rows_and_canon_are_not_witness_escape_paths() {
    let mut retained = source();
    retained
        .physical
        .program
        .definitions
        .iter_mut()
        .find(|definition| definition.name == "s")
        .unwrap()
        .mode = syn::parse_quote!(let);
    retained.structures.entries.push(syn::parse_quote! {
        s rows = ::std::vec::Vec<_>;
    });
    ir::contract::check(&retained).unwrap();
    assert_eq!(retain_lookups(&retained, quote!()).unwrap(), retained);

    let mut canonical = source();
    let definition = canonical
        .physical
        .program
        .definitions
        .iter_mut()
        .find(|definition| definition.name == "s")
        .unwrap();
    definition.canon = Some(Default::default());
    canonical.structures.entries.push(syn::parse_quote! {
        s canon = ::cq_storage::HashSet<_>;
    });
    ir::contract::check(&canonical).unwrap();
    assert_eq!(retain_lookups(&canonical, quote!()).unwrap(), canonical);
}

#[test]
fn structural_check_rejects_tampered_share_and_witness() {
    let source = source();
    let target = retain_lookups(&source, quote!()).unwrap();
    let mut bad_share = target.clone();
    let entry = bad_share
        .structures
        .entries
        .iter_mut()
        .find(|entry| matches!(entry.structure, Structure::Share { .. }))
        .unwrap();
    *entry = syn::parse_quote! {
        s share = lookup r [x] first s force filtered;
    };
    assert!(ir::check_rewrite(&source, &bad_share).is_err());
    let mut bad_witness = target;
    let entry = bad_witness
        .structures
        .entries
        .iter_mut()
        .find(|entry| entry.name == "s" && matches!(entry.structure, Structure::Repr { .. }))
        .unwrap();
    *entry = syn::parse_quote! {
        s repr = tuples witness {r [y] at s as ::cq_storage::RunCursor<'_, _>};
    };
    assert!(ir::check_rewrite(&source, &bad_witness).is_err());
}

#[test]
fn composes_with_the_pinned_sorted_runs_fixture() {
    let physical = include_str!("fixtures/representations.physical");
    let source: ir::Program = syn::parse_str(&format!(
        "{physical}\n{}",
        include_str!("fixtures/representations.structures")
    ))
    .unwrap();
    let expected: ir::Program = syn::parse_str(&format!(
        "{physical}\n{}",
        include_str!("fixtures/sorted_runs.structures")
    ))
    .unwrap();
    let sorted = sorted_runs(&source, quote!()).unwrap();
    assert_eq!(sorted, expected);
    let target = retain_lookups(&sorted, quote!()).unwrap();
    // This fixture's semi/probe pairs use distinct builds or key levels.
    assert_eq!(target, expected);
    ir::check_rewrite(&source, &target).unwrap();
}

#[test]
fn public_rewrite_macro_records_the_cursor() {
    let captured = mini_linq::passes::select! {
        pipeline {
            ::cq_rewrites::sorted_runs,
            ::cq_rewrites::retain_lookups,
            cq_rewrite_test_driver::capture_structures
        };
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> x};
            r = let rename R@1 {a -> x,b -> y};
            s = pipe semi eager [x] l,r;
            p = pipe probe first [x] s,r;
            out = pipe canon project p keep {x,y};
            return out as answer(x,y);
        }
    };
    let target: ir::Program = syn::parse_str(captured).unwrap();
    let facts = ir::contract::check(&target).unwrap();
    assert_eq!(facts.shares.len(), 1);
    assert_eq!(facts.witnesses.len(), 1);
}

#[test]
fn composed_named_pipeline_executes_the_query() {
    for (left, right) in [
        (vec![], vec![]),
        (
            vec![(1,), (2,), (3,), (4,), (1,)],
            vec![(3, 30), (1, 10), (1, 11), (1, 10), (4, -40)],
        ),
    ] {
        let expected: std::collections::BTreeSet<_> = left
            .iter()
            .flat_map(|&(a,)| {
                right
                    .iter()
                    .filter_map(move |&(ra, b)| (a == ra && b >= 0).then_some((a, b)))
            })
            .collect();
        let actual: std::collections::BTreeSet<_> = mini_linq::compile! {
            pipeline {
                structures = [::cq_rewrites::sorted_runs, ::cq_rewrites::retain_lookups];
            }
            struct RetainedQuery;
            relation L(a:i32) = (left);
            relation R(a:i32,b:i32) = (right);
            answer(a,b) :- L(a), R(a,b), if (*b >= 0);
        }
        .map(|(a, b)| (*a, *b))
        .collect();
        assert_eq!(actual, expected);
    }
}

#[test]
fn retained_cursor_executes_through_select_and_project() {
    for (left, right) in [
        (vec![], vec![]),
        (
            vec![(-1,), (1,), (2,), (3,), (4,), (1,)],
            vec![(-1, 0), (3, 30), (1, 10), (1, 11), (1, 10), (4, 40)],
        ),
    ] {
        let expected: std::collections::BTreeSet<_> = left
            .iter()
            .flat_map(|&(a,)| {
                right
                    .iter()
                    .filter_map(move |&(ra, b)| (a == ra && a >= 0).then_some((a, b)))
            })
            .collect();
        let actual: std::collections::BTreeSet<_> = mini_linq::passes::select! {
            pipeline {
                ::cq_rewrites::sorted_runs,
                ::cq_rewrites::retain_lookups,
                mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull
            };
            physical {
                relation L(a:i32) = (left); relation R(a:i32,b:i32) = (right);
                l = pipe rename L@0 {a -> x};
                r = let rename R@1 {a -> x,b -> y};
                s = pipe semi eager [x] l,r;
                filtered = pipe select s if (x >= 0);
                projected = pipe project filtered keep {x};
                p = pipe probe first [x] projected,r;
                out = pipe canon project p keep {x,y};
                return out as answer(x,y);
            }
        }
        .map(|(x, y)| (*x, *y))
        .collect();
        assert_eq!(actual, expected);
    }
}
