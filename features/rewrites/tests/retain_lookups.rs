use cq_lang_structures::{self as ir, Structure};
use cq_rewrite_passes::{retain_lookups, sorted_runs};
use quote::{ToTokens, quote};
use syn::parse::Parser;
use cq_testkit::corpus::parse_structures_capture;

fn custom_payload() -> ir::Declaration {
    syn::parse_quote!(
        custom_payload = CustomPayload<_, _> {
            reads [exact, range, membership], supports [retain, expand, keys], build typed
        };
    )
}

fn triangle_source() -> (ir::physical::Program, ir::Program) {
    parse_structures_capture(&quote! { physical {
        relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
        l = pipe rename L@0 {a -> a,b -> b};
        r = let rename R@1 {b -> b,c -> c};
        pre_r = pipe semi eager [b] l,r;
        s = let rename S@2 {a -> a,c -> c};
        pre_s = pipe semi eager [a] pre_r,s;
        candidate = pipe probe smallest [b] pre_s,r;
        checked = let canon semi eager [a,c] candidate,s;
        return checked as answer(a,b,c);
    } structures { providers { custom_directory = CustomDirectory { reads [exact, range, membership], supports [retain, expand, keys], build inferred }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        r = @custom_directory as CustomDirectory [b][c] supports [retain, expand];
        s = @custom_directory as CustomDirectory [a][c] supports [retain, expand];
        l host scan = @host_scan as ::core::iter::Map<_, _> [];
        r host scan = @host_scan as ::core::iter::Map<_, _> [];
        s host scan = @host_scan as ::core::iter::Map<_, _> [];
        r rows = @vec as ::std::vec::Vec<_>;
        s rows = @vec as ::std::vec::Vec<_>;
        checked rows = @sort as ::cq_storage::dedup::Sort; checked canon = rows;
    } }.to_string()).unwrap()
}

#[test]
fn triangle_records_both_prefix_witnesses_at_first_probe() {
    let (physical, source) = triangle_source();
    let target = retain_lookups(&physical, &source, quote!()).unwrap();
    let facts = ir::contract::check(&physical, &target).unwrap();
    assert_eq!(facts.witnesses.len(), 2);
    assert_eq!(facts.shares.len(), 2);
    for (first, build, key) in [("pre_r", "r", "b"), ("pre_s", "s", "a")] {
        let share = &facts.shares[first];
        assert_eq!(share.build, build);
        assert_eq!(share.keys, [key]);
        assert_eq!(share.force, "candidate");
        assert!(facts.witnesses.contains_key(first));
    }
    assert_eq!(retain_lookups(&physical, &target, quote!()).unwrap(), target);
    assert_eq!(
        (|input: syn::parse::ParseStream<'_>| ir::Program::parse(input, &physical)).parse2(target.to_token_stream()).unwrap(),
        target
    );
}

#[test]
fn candidate_retention_uses_directory_support_not_complete_payload_support() {
    let (physical, mut source) = triangle_source();
    source.structures.entries.push(syn::parse_quote!(
        r = @custom_payload as CustomPayload [b];
    ));
    source.structures.declare(&[custom_payload()]).unwrap();
    let target = retain_lookups(&physical, &source, quote!()).unwrap();
    let facts = ir::contract::check(&physical, &target).unwrap();
    assert!(facts.witnesses.contains_key("pre_r"));
    assert_eq!(facts.shares["pre_r"].force, "candidate");

    source.structures.entries[0] = syn::parse_quote!(
        r = @custom_directory as CustomDirectory [b][c];
    );
    *source.structures.entries.last_mut().unwrap() = syn::parse_quote!(
        r = @custom_payload as CustomPayload [b] supports [expand];
    );
    let target = retain_lookups(&physical, &source, quote!()).unwrap();
    let facts = ir::contract::check(&physical, &target).unwrap();
    assert!(!facts.witnesses.contains_key("pre_r"));
    assert!(facts.witnesses.contains_key("pre_s"));
}

#[test]
fn declared_host_lookup_retains_exact_snapshot_ranges() {
    let (physical, mut source) = parse_structures_capture(&quote! { physical {
        relation L(a:i64); relation R(a:i64,b:i64);
        l = pipe rename L@0 {a -> x};
        r = pipe rename R@1 {a -> x,b -> y};
        s = pipe semi eager [x] l,r;
        p = let canon probe first [x] s,r;
        return p as answer(x,y);
    } structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; snapshot = Snapshot<0> { reads [exact, range, membership], supports [retain, expand, keys], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        l host scan = @host_scan as ::core::iter::Map<_,_> [];
        r host scan = @host_scan as ::core::iter::Map<_,_> [];
        r host result(s) = @snapshot as Snapshot<0> [x] supports [retain];
        r host result(p) = @snapshot as Snapshot<0> [x] supports [retain];
        p rows = @sort as ::cq_storage::dedup::Sort; p canon = rows;
    } }.to_string()).unwrap();
    let target = retain_lookups(&physical, &source, quote!()).unwrap();
    let facts = ir::contract::check(&physical, &target).unwrap();
    assert_eq!(facts.shares["s"].force, "p");
    assert!(facts.witnesses.contains_key("s"));
    assert_eq!(retain_lookups(&physical, &target, quote!()).unwrap(), target);
    let mut changed_provider = source.clone();
    changed_provider.structures.entries[3] = syn::parse_quote! {
        r host result(p) = @other_snapshot as Snapshot<0> [x] supports [retain];
    };
    changed_provider
        .structures
        .declare(&[syn::parse_quote! {
            other_snapshot = Snapshot<0> {
                reads [exact, range, membership], supports [retain, expand, keys], build typed
            };
        }])
        .unwrap();
    // One host/key has one fallback snapshot, even when two provider tokens
    // spell the same Rust type. This violates the input contract before a
    // retention decision can be made.
    assert!(ir::contract::check(&physical, &changed_provider).is_err());
    assert!(retain_lookups(&physical, &changed_provider, quote!()).is_err());
    source.structures.entries[3] = syn::parse_quote! {
        r host result(p) = @snapshot as Snapshot<1> [x] supports [retain];
    };
    assert!(retain_lookups(&physical, &source, quote!()).is_err());
}

#[test]
fn triangle_prefix_shares_reject_wrong_build_key_and_force() {
    let (physical, source) = triangle_source();
    let target = retain_lookups(&physical, &source, quote!()).unwrap();
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
        assert!(ir::contract::check(&physical, &bad).is_err());
    }
}

#[test]
fn triangle_prefix_witnesses_do_not_infer_key_types() {
    let (physical, source) = triangle_source();
    let expected = retain_lookups(&physical, &source, quote!()).unwrap();
    let expected_facts = ir::contract::check(&physical, &expected).unwrap();
    let key_types: [syn::Type; 3] = [
        syn::parse_quote!(bool),
        syn::parse_quote!(KeyAlias),
        syn::parse_quote!(keys::UserKey),
    ];
    for ty in key_types {
        let (mut physical, source) = triangle_source();
        for declaration in &mut physical.program.declarations {
            for column in &mut declaration.columns {
                column.ty = ty.clone();
            }
        }
        // Key capabilities are Rust obligations, not inferred from type spelling.
        let target = retain_lookups(&physical, &source, quote!()).unwrap();
        let facts = ir::contract::check(&physical, &target).unwrap();
        assert_eq!(facts.shares, expected_facts.shares);
        assert_eq!(facts.witnesses, expected_facts.witnesses);
    }
}

#[test]
fn triangle_prefix_witnesses_do_not_escape() {
    let (mut physical, mut escaping) = triangle_source();
    physical.program.definitions[4].canon = Some(Default::default());
    physical.program.definitions[4].mode = syn::parse_quote!(let);
    escaping.structures.entries.extend([
        syn::parse_quote!(pre_s rows = @sort as ::cq_storage::dedup::Sort;),
        syn::parse_quote!(pre_s canon = rows;),
    ]);
    assert_eq!(retain_lookups(&physical, &escaping, quote!()).unwrap(), escaping);
}

fn hashed_source() -> (ir::physical::Program, ir::Program) {
    let physical = syn::parse_quote! { physical {
        relation L(a:i32); relation R(a:i32,b:i32);
        l = pipe rename L@0 {a -> x};
        r = let rename R@1 {a -> x,b -> y};
        s = pipe semi eager [x] l,r;
        filtered = pipe select s if (x >= 0);
        projected = pipe project filtered keep {x};
        p = pipe probe first [x] projected,r;
        out = let canon project p keep {x,y};
        return out as answer(x,y);
    }};
    let source = cq_structure_selection::compile(&physical).unwrap();
    (physical, source)
}

fn source() -> (ir::physical::Program, ir::Program) {
    let (physical, source) = hashed_source();
    let source = sorted_runs(&physical, &source, quote!()).unwrap();
    (physical, source)
}

#[test]
fn records_one_checked_cursor_and_roundtrips_idempotently() {
    let (physical, source) = source();
    assert!(ir::contract::check(&physical, &source).unwrap().shares.is_empty());
    let target = retain_lookups(&physical, &source, quote!()).unwrap();
    ir::check_rewrite(&physical, &source, &physical, &target).unwrap();
    let facts = ir::contract::check(&physical, &target).unwrap();
    assert_eq!(facts.shares.len(), 1);
    assert_eq!(facts.shares["s"].build, "r");
    assert_eq!(facts.shares["s"].keys, ["x"]);
    assert_eq!(facts.shares["s"].first, "s");
    assert_eq!(facts.shares["s"].force, "p");
    assert_eq!(facts.witnesses.len(), 1);
    assert!(facts.witnesses.contains_key("s"));
    assert_eq!(retain_lookups(&physical, &target, quote!()).unwrap(), target);
    assert_eq!(
        (|input: syn::parse::ParseStream<'_>| ir::Program::parse(input, &physical)).parse2(target.to_token_stream()).unwrap(),
        target
    );
    assert!(retain_lookups(&physical, &source, quote!(unexpected)).is_err());
}

#[test]
fn preserves_positions_and_completes_an_existing_share() {
    let (physical, mut source) = source();
    source.structures.entries.push(syn::parse_quote! {
        s repr = positions {L lineage {x -> a}} owned [];
    });
    source.structures.entries.push(syn::parse_quote! {
        s share = lookup r [x] first s force p;
    });
    let old_repr = ir::contract::check(&physical, &source).unwrap().repr["s"].clone();
    let target = retain_lookups(&physical, &source, quote!()).unwrap();
    let facts = ir::contract::check(&physical, &target).unwrap();
    assert_eq!(facts.repr["s"], old_repr);
    assert_eq!(facts.shares.len(), 1);
    assert_eq!(facts.witnesses.len(), 1);
    ir::check_rewrite(&physical, &source, &physical, &target).unwrap();
}

#[test]
fn a_custom_provider_with_declared_support_is_eligible() {
    let (physical, mut source) = source();
    for entry in &mut source.structures.entries {
        if let Structure::Built { keyed, .. } = &mut entry.structure {
            keyed.provider = custom_payload().instance();
        }
    }
    source.structures.declare(&[custom_payload()]).unwrap();
    let target = retain_lookups(&physical, &source, quote!()).unwrap();
    assert_eq!(ir::contract::check(&physical, &target).unwrap().witnesses.len(), 1);
}

#[test]
fn hash_index_does_not_claim_a_copy_cursor() {
    let (physical, source) = hashed_source();
    assert_eq!(retain_lookups(&physical, &source, quote!()).unwrap(), source);
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
        out = let canon project s keep {x};
        return out as answer(x);
    }};
    let source = cq_structure_selection::compile(&physical).unwrap();
    let source = sorted_runs(&physical, &source, quote!()).unwrap();
    let built = &ir::contract::check(&physical, &source).unwrap().built["r"][0].1;
    assert_eq!(built.token, "sorted_run");
    assert_eq!(
        built.ty,
        syn::parse_quote!(::cq_storage::SortedRun<_, _, ::cq_storage::I64Codec>)
    );
    assert_eq!(retain_lookups(&physical, &source, quote!()).unwrap(), source);
}

#[test]
fn retained_rows_and_canon_are_not_witness_escape_paths() {
    let (mut physical, mut retained) = source();
    physical
        .program
        .definitions
        .iter_mut()
        .find(|definition| definition.name == "s")
        .unwrap()
        .mode = syn::parse_quote!(let);
    retained.structures.entries.push(syn::parse_quote! {
        s rows = @vec as ::std::vec::Vec<_>;
    });
    retained
        .structures
        .declare(&cq_lang_structures::provider::base::declarations())
        .unwrap();
    ir::contract::check(&physical, &retained).unwrap();
    assert_eq!(retain_lookups(&physical, &retained, quote!()).unwrap(), retained);

    let (mut physical, mut canonical) = source();
    let definition = physical
        .program
        .definitions
        .iter_mut()
        .find(|definition| definition.name == "s")
        .unwrap();
    definition.canon = Some(Default::default());
    definition.mode = syn::parse_quote!(let);
    canonical.structures.entries.extend([
        syn::parse_quote!(s rows = @sort as ::cq_storage::dedup::Sort;),
        syn::parse_quote!(s canon = rows;),
    ]);
    canonical
        .structures
        .declare(&cq_lang_structures::provider::base::declarations())
        .unwrap();
    ir::contract::check(&physical, &canonical).unwrap();
    assert_eq!(retain_lookups(&physical, &canonical, quote!()).unwrap(), canonical);
}

#[test]
fn structural_check_rejects_tampered_share_and_witness() {
    let (physical, source) = source();
    let target = retain_lookups(&physical, &source, quote!()).unwrap();
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
    assert!(ir::check_rewrite(&physical, &source, &physical, &bad_share).is_err());
    let mut bad_witness = target;
    let entry = bad_witness
        .structures
        .entries
        .iter_mut()
        .find(|entry| entry.name == "s" && matches!(entry.structure, Structure::Repr { .. }))
        .unwrap();
    *entry = syn::parse_quote! {
        s repr = tuples witness {r [y] at s as _};
    };
    assert!(ir::check_rewrite(&physical, &source, &physical, &bad_witness).is_err());
}

#[test]
fn composes_with_the_pinned_sorted_runs_fixture() {
    let physical: ir::physical::Program =
        syn::parse_str(include_str!("fixtures/representations.physical")).unwrap();
    let source = (|input: syn::parse::ParseStream<'_>| ir::Program::parse(input, &physical))
        .parse_str(include_str!("fixtures/representations.structures")).unwrap();
    let expected = (|input: syn::parse::ParseStream<'_>| ir::Program::parse(input, &physical))
        .parse_str(include_str!("fixtures/sorted_runs.structures")).unwrap();
    let sorted = sorted_runs(&physical, &source, quote!()).unwrap();
    assert_eq!(sorted, expected);
    let target = retain_lookups(&physical, &sorted, quote!()).unwrap();
    // This fixture's semi/probe pairs use distinct builds or key levels.
    assert_eq!(target, expected);
    ir::check_rewrite(&physical, &source, &physical, &target).unwrap();
}

#[test]
fn public_rewrite_macro_records_the_cursor() {
    let captured = mini_linq::passes::select! {
        pipeline {
            ::cq_rewrites::sorted_runs,
            ::cq_rewrites::retain_lookups,
            cq_rewrite_test_driver::capture_structures_stack
        };
        physical {
            relation L(a:i32); relation R(a:i32,b:i32);
            l = pipe rename L@0 {a -> x};
            r = let rename R@1 {a -> x,b -> y};
            s = pipe semi eager [x] l,r;
            p = pipe probe first [x] s,r;
            out = let canon project p keep {x,y};
            return out as answer(x,y);
        }
    };
    let (physical, target) = parse_structures_capture(captured).unwrap();
    let facts = ir::contract::check(&physical, &target).unwrap();
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
            answer(a,b) :- L(a), R(a,b), where b >= 0;
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
                out = let canon project p keep {x,y};
                return out as answer(x,y);
            }
        }
        .map(|(x, y)| (*x, *y))
        .collect();
        assert_eq!(actual, expected);
    }
}

#[test]
fn d19_le_runs_membership_bridge_preserves_the_complete_bag() {
    let lineitem = vec![(1_i64, 9_i64), (1, 9), (2, 9), (3, 8)];
    let orders = vec![(1_i64, 100_i64), (1, 101), (1, 101), (2, 200), (3, 300)];
    let supplier = vec![(9_i64,)];
    let mut expected: Vec<_> = lineitem.iter()
        .filter(|&&(_, supplier_key)| supplier.contains(&(supplier_key,)))
        .flat_map(|&(order_key, supplier_key)| orders.iter().filter_map(move |&(key, customer_key)|
            (key == order_key).then_some((order_key, customer_key, supplier_key))))
        .collect();
    let visited = &std::cell::RefCell::new(Vec::new());
    let canonical: std::collections::BTreeSet<_> = mini_linq::passes::select! {
        pipeline {
            ::cq_rewrites::sorted_runs,
            ::cq_rewrites::retain_lookups,
            mini_linq::passes::sequence, mini_linq::passes::sequential, mini_linq::passes::translate_pull
        };
        physical {
            relation Lineitem(o:i64,s:i64) = (&lineitem);
            relation Orders(o:i64,c:i64) = (&orders);
            relation Supplier(s:i64) = (&supplier);
            l = pipe rename Lineitem@0 {o -> order_key,s -> supplier_key};
            r = let rename Orders@1 {o -> order_key,c -> customer_key};
            m = let canon rename Supplier@2 {s -> supplier_key};
            eager = pipe semi eager [order_key] l,r;
            checked = pipe probe first [supplier_key] eager,m;
            expanded = pipe probe first [order_key] checked,r;
            observed = pipe guard expanded if ({
                visited.borrow_mut().push((*order_key, *customer_key, *supplier_key));
                true
            });
            out = let canon project observed keep {order_key,customer_key,supplier_key};
            return out as answer(order_key,customer_key,supplier_key);
        }
    }.map(|(order_key, customer_key, supplier_key)| (*order_key, *customer_key, *supplier_key)).collect();
    assert_eq!(canonical, expected.iter().copied().collect());
    // Observe the entire bag before the required canonical return boundary,
    // so Sort cannot hide a lost first row or changed duplicate multiplicity.
    let mut actual = visited.borrow().clone();
    expected.sort_unstable();
    actual.sort_unstable();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 7);
    assert_eq!(actual.iter().filter(|row| **row == (1, 101, 9)).count(), 4);
}

fn membership_bridge_source() -> (ir::physical::Program, ir::Program) {
    let physical = syn::parse_quote! { physical {
        relation L(a:i32,c:i32); relation R(a:i32,b:i32); relation M(c:i32);
        l = pipe rename L@0 {a -> x,c -> z};
        r = let rename R@1 {a -> x,b -> y};
        m = let rename M@2 {c -> z};
        s = pipe semi eager [x] l,r;
        checked = pipe probe first [z] s,m;
        projected = pipe project checked keep {x};
        p = pipe probe first [x] projected,r;
        out = let canon project p keep {x,y};
        return out as answer(x,y);
    }};
    let source = cq_structure_selection::compile(&physical).unwrap();
    let source = sorted_runs(&physical, &source, quote!()).unwrap();
    (physical, source)
}

#[test]
fn retains_same_lookup_across_membership_on_another_key() {
    let (physical, source) = membership_bridge_source();
    let target = retain_lookups(&physical, &source, quote!()).unwrap();
    let facts = ir::contract::check(&physical, &target).unwrap();
    assert_eq!(facts.shares.len(), 1);
    assert_eq!(facts.shares["s"].build, "r");
    assert_eq!(facts.shares["s"].keys, ["x"]);
    assert_eq!(facts.shares["s"].force, "p");
    assert_eq!(facts.witnesses.len(), 1);
    assert_eq!(facts.witnesses["s"].first, "s");
    ir::check_rewrite(&physical, &source, &physical, &target).unwrap();
    assert_eq!(retain_lookups(&physical, &target, quote!()).unwrap(), target);
    assert_eq!(
        (|input: syn::parse::ParseStream<'_>| ir::Program::parse(input, &physical)).parse2(target.to_token_stream()).unwrap(),
        target
    );
}

#[test]
fn authored_membership_bridge_preserves_positions_without_producer_replay() {
    let (physical, mut authored) = membership_bridge_source();
    authored.structures.entries.extend([
        syn::parse_quote!(s share = lookup r [x] first s force p;),
        syn::parse_quote!(
            s repr = positions {L lineage {x -> a,z -> c}} owned []
                witness {r [x] at s as _};
        ),
    ]);
    let facts = ir::contract::check(&physical, &authored).unwrap();
    assert_eq!(facts.shares["s"].force, "p");
    assert_eq!(facts.witnesses["s"].keys.columns[0], "x");
    assert!(matches!(facts.repr["s"], ir::Repr::Positions { .. }));
    assert_eq!(retain_lookups(&physical, &authored, quote!()).unwrap(), authored);
}

#[test]
fn membership_bridge_requires_the_original_native_snapshot_occurrence() {
    fn native_demands(mut source: ir::Program, force: syn::Ident) -> ir::Program {
        // Membership selection is not a native exact lookup. This fixture
        // explicitly supplies the same retainable protocol at both demands
        // so only the occurrence identity differs in the negative case.
        source.structures.entries.retain(|entry| !matches!(&entry.structure,
            Structure::Host { reader: ir::ReaderRef::Result(reader), .. }
                | Structure::Native { reader: ir::ReaderRef::Result(reader), .. }
                if reader == "s" || reader == "p"));
        source.structures.entries.extend([
            syn::parse_quote!(r native result(s) = exact [x] supports [retain, expand];),
            syn::parse_quote!(#force native result(p) = exact [x] supports [retain, expand];),
        ]);
        source.structures.declare(&[]).unwrap();
        source
    }
    let physical = syn::parse_quote! { physical {
        relation L(a:i32,c:i32);
        relation R(a:i32,b:i32) host [a] reads [exact] supports [retain, expand];
        relation M(c:i32);
        l = pipe rename L@0 {a -> x,c -> z};
        r = pipe rename R@1 {a -> x,b -> y};
        m = let rename M@2 {c -> z};
        s = pipe semi eager [x] l,r;
        checked = pipe probe first [z] s,m;
        projected = pipe project checked keep {x};
        p = pipe probe first [x] projected,r;
        out = let canon project p keep {x,y};
        return out as answer(x,y);
    }};
    let source = native_demands(cq_structure_selection::compile(&physical).unwrap(), syn::parse_quote!(r));
    let retained = retain_lookups(&physical, &source, quote!()).unwrap();
    assert_eq!(
        ir::contract::check(&physical, &retained).unwrap().shares["s"].build,
        "r"
    );
    let mut physical = physical;
    physical.program.definitions.insert(
        3,
        syn::parse_quote!(other = pipe rename R@3 {a -> x,b -> y};),
    );
    physical.program.definitions[7] =
        syn::parse_quote!(p = pipe probe first [x] projected,other;);
    rejects_bridge(&physical, native_demands(cq_structure_selection::compile(&physical).unwrap(), syn::parse_quote!(other)));
}

fn rejects_bridge(physical: &ir::physical::Program, source: ir::Program) {
    ir::contract::check(&physical, &source).unwrap();
    assert_eq!(retain_lookups(physical, &source, quote!()).unwrap(), source);
    // The independent contract must reject the same unsupported route even
    // when an authored decision bypasses the producer.
    let mut authored = source;
    authored.structures.entries.extend([
        syn::parse_quote!(s share = lookup r [x] first s force p;),
        syn::parse_quote!(s repr = tuples witness {r [x] at s as _};),
    ]);
    assert!(ir::contract::check(physical, &authored).is_err());
}

#[test]
fn membership_bridge_cannot_drop_the_retained_key() {
    let mut physical = membership_bridge_source().0;
    physical.program.definitions[5] =
        syn::parse_quote!(projected = pipe project checked keep {z};);
    physical.program.definitions[6] =
        syn::parse_quote!(p = pipe probe first [] projected,r;);
    let source = cq_structure_selection::compile(&physical).unwrap();
    rejects_bridge(&physical, sorted_runs(&physical, &source, quote!()).unwrap());
}

#[test]
fn an_introducing_probe_is_not_a_bridge_even_if_its_payload_is_dropped() {
    let physical = syn::parse_quote! { physical {
        relation L(a:i32,c:i32); relation R(a:i32,b:i32); relation M(c:i32,d:i32);
        l = pipe rename L@0 {a -> x,c -> z};
        r = let rename R@1 {a -> x,b -> y};
        m = let rename M@2 {c -> z,d -> w};
        s = pipe semi eager [x] l,r;
        checked = pipe probe first [z] s,m;
        projected = pipe project checked keep {x};
        p = pipe probe first [x] projected,r;
        out = let canon project p keep {x,y};
        return out as answer(x,y);
    }};
    let source = cq_structure_selection::compile(&physical).unwrap();
    rejects_bridge(&physical, sorted_runs(&physical, &source, quote!()).unwrap());
}

#[test]
fn membership_bridge_does_not_change_the_lookup_provider() {
    let mut physical = membership_bridge_source().0;
    physical.program.definitions.insert(
        3,
        syn::parse_quote!(other = let rename R@3 {a -> x,b -> y};),
    );
    physical.program.definitions[7] =
        syn::parse_quote!(p = pipe probe first [x] projected,other;);
    let source = cq_structure_selection::compile(&physical).unwrap();
    rejects_bridge(&physical, sorted_runs(&physical, &source, quote!()).unwrap());
}

#[test]
fn opaque_bindings_and_guards_are_not_retention_bridges() {
    for bridge in [
        syn::parse_quote!(projected = pipe bind checked with w:i32 = (*x);),
        syn::parse_quote!(projected = pipe guard checked if (*x >= 0);),
    ] {
        let mut physical = membership_bridge_source().0;
        physical.program.definitions[5] = bridge;
        let source = cq_structure_selection::compile(&physical).unwrap();
        rejects_bridge(&physical, sorted_runs(&physical, &source, quote!()).unwrap());
    }
}

#[test]
fn membership_bridge_cannot_publish_or_canonicalize_the_cursor() {
    for retained in [
        syn::parse_quote!(checked = let probe first [z] s,m;),
        syn::parse_quote!(checked = let canon probe first [z] s,m;),
    ] {
        let mut physical = membership_bridge_source().0;
        physical.program.definitions[4] = retained;
        let source = cq_structure_selection::compile(&physical).unwrap();
        rejects_bridge(&physical, sorted_runs(&physical, &source, quote!()).unwrap());
    }
}
