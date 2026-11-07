use cq_lang_structures::{self as ir, Structure};
use cq_structure_selection::retype;
use cq_testkit::PassCase;
use quote::{ToTokens, quote};

/// Declares a custom test token whose spelling no pass inspects.
fn custom(token: &str, ty: proc_macro2::TokenStream, build: &str) -> ir::Declaration {
    let token = syn::Ident::new(token, proc_macro2::Span::call_site());
    let build = syn::Ident::new(build, proc_macro2::Span::call_site());
    syn::parse_quote!(#token = #ty { reads [exact, membership], build #build };)
}

/// Advice declaring the `replacement` token spelled `ty`.
fn replacement(ty: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    quote!(providers { replacement = #ty { reads [exact, range, membership], build typed }; })
}

#[test]
fn membership_advice_preserves_a_fully_bound_physical_probe() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation L(a:u32,b:u32); relation R(a:u32,b:u32);
        l = pipe rename L {a -> x,b -> y};
        r = let rename R {a -> x,b -> y};
        joined = let canon probe first [x,y] l,r;
        return joined as answer(x,y);
    } structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; hash_index = ::cq_storage::HashIndex<'_, _, _> { reads [exact, membership], supports [keys], build typed, join_filter, statistics_build [rows, max_group], statistics [ rows: rows occurrences exact hash_finalize stat_rows host optional build single, keys: keys distinct_keys exact hash_finalize stat_keys host optional build single, max_group: max_group max_bucket_occurrences exact hash_finalize stat_max_group host optional build single ] }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        l host scan = @host_scan as ::core::iter::Map<_,_> [];
        r host scan = @host_scan as ::core::iter::Map<_,_> [];
        r = @hash_index as ::cq_storage::HashIndex<'_,_,_> [x,y] supports [keys];
        joined rows = @sort as ::cq_storage::dedup::Sort; joined canon = rows;
    }};
    let target = cq_rewrite_passes::membership(&source, quote!(r)).unwrap();
    assert_eq!(target.physical, source.physical);
    ir::check_rewrite(&source, &target).unwrap();
    assert!(target.structures.entries.iter().all(|entry| !matches!(
        &entry.structure, Structure::Built { keyed, .. } if keyed.supports.is_some()
    )));
    assert!(
        ir::contract::check(&target)
            .unwrap()
            .membership
            .contains(&("r".into(), vec!["x".into(), "y".into()]))
    );
    assert_eq!(
        cq_rewrite_passes::membership(&target, quote!(r)).unwrap(),
        target
    );
    assert!(cq_rewrite_passes::membership(&source, quote!(missing)).is_err());
}

#[test]
fn membership_advice_cannot_discard_a_demanded_payload() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation L(a:u32); relation R(a:u32,b:u32);
        l = pipe rename L {a -> x};
        r = let rename R {a -> x,b -> y};
        joined = let canon probe first [x] l,r;
        return joined as answer(x,y);
    } structures { providers { host_scan = ::core::iter::Map<_, _> { reads [], build typed }; hash_index = ::cq_storage::HashIndex<'_, _, _> { reads [exact, membership], supports [keys], build typed, join_filter, statistics_build [rows, max_group], statistics [ rows: rows occurrences exact hash_finalize stat_rows host optional build single, keys: keys distinct_keys exact hash_finalize stat_keys host optional build single, max_group: max_group max_bucket_occurrences exact hash_finalize stat_max_group host optional build single ] }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        l host scan = @host_scan as ::core::iter::Map<_,_> [];
        r host scan = @host_scan as ::core::iter::Map<_,_> [];
        r = @hash_index as ::cq_storage::HashIndex<'_,_,_> [x];
        joined rows = @sort as ::cq_storage::dedup::Sort; joined canon = rows;
    }};
    assert!(cq_rewrite_passes::membership(&source, quote!(r)).is_err());
}

fn smallest_expand_source() -> ir::Program {
    syn::parse_quote! { physical {
        relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
        l = pipe rename L {a -> a,b -> b};
        r = let rename R {b -> b,c -> c};
        s = let rename S {a -> a,c -> c};
        candidate = pipe probe smallest [b] l,r;
        checked = let canon probe first [a,c] candidate,s;
        return checked as answer(a,b,c);
    } structures { providers { custom_directory = CustomDirectory { reads [exact, range, membership], supports [retain, expand, keys], build inferred }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; vec = ::std::vec::Vec<_> { reads [], build typed, accounting rows }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        r = @custom_directory as CustomDirectory [b][c] supports [expand];
        s = @custom_directory as CustomDirectory [a][c] supports [expand];
        l host scan = @host_scan as ::core::iter::Map<_, _> [];
        r host scan = @host_scan as ::core::iter::Map<_, _> [];
        s host scan = @host_scan as ::core::iter::Map<_, _> [];
        r rows = @vec as ::std::vec::Vec<_>;
        s rows = @vec as ::std::vec::Vec<_>;
        checked rows = @sort as ::cq_storage::dedup::Sort; checked canon = rows;
    } }
}

#[test]
fn entry_retype_changes_only_one_provider_and_checks_the_result() {
    let source = smallest_expand_source();
    let device_columns = custom("device_columns", quote!(DeviceColumns), "inferred");
    let target = retype(
        &source,
        quote!(providers { #device_columns } entry(4) => @device_columns as DeviceColumns),
    )
    .unwrap();
    let mut expected = source.clone();
    expected.structures.entries[4] =
        syn::parse_quote!(s host scan = @device_columns as DeviceColumns [];);
    expected
        .structures
        .declare(&[device_columns.clone()])
        .unwrap();
    assert_eq!(target, expected);
    ir::check_rewrite(&source, &target).unwrap();
    for selector in [quote!(entry(100)), quote!(entry(4, 5))] {
        assert!(
            retype(
                &source,
                quote!(providers { #device_columns } #selector => @device_columns as DeviceColumns)
            )
            .is_err()
        );
    }
}

#[test]
fn expand_lookups_records_only_the_first_supported_smallest_probe() {
    let source = smallest_expand_source();
    let target = cq_rewrite_passes::expand_lookups(&source, quote!(256)).unwrap();
    let mut expected = source.clone();
    expected
        .structures
        .entries
        .push(syn::parse_quote!(candidate stage = batch 256 expand;));
    assert_eq!(target, expected);
    ir::check_rewrite(&source, &target).unwrap();
    assert_eq!(target.physical, source.physical);
    assert_eq!(
        syn::parse2::<ir::Program>(target.to_token_stream()).unwrap(),
        target
    );
    assert_eq!(
        cq_rewrite_passes::expand_lookups(&target, quote!(256)).unwrap(),
        target
    );
    let facts = ir::contract::check(&target).unwrap();
    assert!(facts.expands.contains("candidate"));
    assert!(!facts.expands.contains("checked"));
}

#[test]
fn expand_lookups_skips_unrecorded_range_capabilities_without_retyping() {
    let declared = [
        custom("other_trie", quote!(OtherTrie), "inferred"),
        cq_lang_structures::provider::base::declaration("sorted_run"),
    ];
    for (index, entry) in [
        (0, syn::parse_quote!(r = @other_trie as OtherTrie [b][c];)),
        (1, syn::parse_quote!(s = @other_trie as OtherTrie [a][c];)),
        (
            0,
            syn::parse_quote!(r = @sorted_run as ::cq_storage::SortedRun<_, _> [b][c];),
        ),
    ] {
        let mut source = smallest_expand_source();
        source.structures.entries[index] = entry;
        source.structures.declare(&declared).unwrap();
        ir::contract::check(&source).unwrap();
        assert_eq!(
            cq_rewrite_passes::expand_lookups(&source, quote!(256)).unwrap(),
            source
        );
    }
    let mut mixed = smallest_expand_source();
    mixed.structures.entries[1] = syn::parse_quote!(s = @other_trie as OtherTrie [a][c];);
    mixed
        .structures
        .entries
        .push(syn::parse_quote!(r = @sorted_run as ::cq_storage::SortedRun<_, _> [b];));
    mixed.structures.declare(&declared).unwrap();
    ir::contract::check(&mixed).unwrap();
    assert_eq!(
        cq_rewrite_passes::expand_lookups(&mixed, quote!(256)).unwrap(),
        mixed
    );
    let mut missing = smallest_expand_source();
    missing.structures.entries.remove(1);
    assert!(cq_rewrite_passes::expand_lookups(&missing, quote!(256)).is_err());
    assert!(cq_rewrite_passes::expand_lookups(&smallest_expand_source(), quote!(0)).is_err());
}

fn membership_bridge_source() -> ir::Program {
    syn::parse_quote! { physical {
        relation L(a:i64); relation R(a:i64,b:i64);
        relation M(b:i64,tag:i64); relation S(b:i64,c:i64); relation T(c:i64,d:i64);
        l = pipe rename L {a -> a};
        r = let rename R {a -> a,b -> b};
        m = pipe rename M {b -> b,tag -> tag};
        selected = let point select m if (tag = 7);
        s = let rename S {b -> b,c -> c};
        t = let rename T {c -> c,d -> d};
        first = pipe probe first [a] l,r;
        member = pipe probe first [b] first,selected;
        after = pipe probe first [b] member,s;
        last = pipe probe first [c] after,t;
        out = let canon project last keep {a,b,c,d,tag};
        return out as answer(a,b,c,d,tag);
    } structures { providers { adaptive_keyset = ::cq_storage::AdaptiveKeySet<_, _, _> { reads [membership], build typed, shared }; sorted_run = ::cq_storage::SortedRun<_, _> { reads [exact, range, membership], supports [retain, expand, keys], build typed, join_filter, run_filter, shared, statistics_build [rows, max_group], statistics [ rows: rows occurrences exact run_finalize stat_rows host required build single, keys: keys distinct_keys exact run_finalize stat_keys host required build single, max_group: max_group max_bucket_occurrences exact run_finalize stat_max_group host required build single, runs: runs runs exact run_finalize stat_runs host required build single ] }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; }
        selected = @adaptive_keyset as ::cq_storage::AdaptiveKeySet<_,::cq_storage::I64Codec,64> [b] membership payload [b] constants {tag = 7};
        r = @sorted_run as ::cq_storage::SortedRun<_,_> [a] supports [expand];
        s = @sorted_run as ::cq_storage::SortedRun<_,_> [b] supports [expand];
        t = @sorted_run as ::cq_storage::SortedRun<_,_> [c] supports [expand];
        l host scan = @host_scan as ::core::iter::Map<_,_> [];
        r host scan = @host_scan as ::core::iter::Map<_,_> [];
        m host scan = @host_scan as ::core::iter::Map<_,_> [];
        m host result(selected) = @host_scan as ::core::iter::Map<_,_> [tag];
        s host scan = @host_scan as ::core::iter::Map<_,_> [];
        t host scan = @host_scan as ::core::iter::Map<_,_> [];
        out rows = @sort as ::cq_storage::dedup::Sort; out canon = rows;
    } }
}

#[test]
fn expand_lookups_crosses_certified_bitmap_membership_bridges() {
    let source = membership_bridge_source();
    let target = cq_rewrite_passes::expand_lookups(&source, quote!(256, depth 3)).unwrap();
    let mut expected = source.clone();
    expected.structures.entries.extend([
        syn::parse_quote!(first stage = batch 256 expand;),
        syn::parse_quote!(after stage = batch 256 expand;),
        syn::parse_quote!(last stage = batch 256 expand;),
    ]);
    assert_eq!(target, expected);
    ir::check_rewrite(&source, &target).unwrap();
    assert_eq!(target.physical, source.physical);
    assert_eq!(
        cq_rewrite_passes::expand_lookups(&target, quote!(256, depth 3)).unwrap(),
        target
    );
    let facts = ir::contract::check(&target).unwrap();
    assert!(!facts.expands.contains("member"));
    assert_eq!(facts.filtered["selected"].constants.len(), 1);
}

#[test]
fn expand_lookups_limits_nested_frames_without_retyping_deeper_probes() {
    let source = membership_bridge_source();
    for (arguments, expected) in [
        (quote!(256), vec!["after", "first"]),
        (quote!(256, depth 1), vec!["first"]),
        (quote!(256, depth 3), vec!["after", "first", "last"]),
    ] {
        let target = cq_rewrite_passes::expand_lookups(&source, arguments.clone()).unwrap();
        let facts = ir::contract::check(&target).unwrap();
        assert_eq!(
            facts.expands.iter().map(String::as_str).collect::<Vec<_>>(),
            expected
        );
        assert_eq!(target.physical, source.physical);
        assert_eq!(facts.built, ir::contract::check(&source).unwrap().built);
        assert_eq!(
            cq_rewrite_passes::expand_lookups(&target, arguments).unwrap(),
            target
        );
    }
    for arguments in [
        quote!(256, depth 0),
        quote!(256, frames 2),
        quote!(256, depth 2,),
    ] {
        assert!(cq_rewrite_passes::expand_lookups(&source, arguments).is_err());
    }
}

#[test]
fn expand_lookups_stops_at_uncertified_payload_probe_bridges() {
    let mut source = membership_bridge_source();
    source.structures.entries[0] = syn::parse_quote!(
        selected = @hash_index as ::cq_storage::HashIndex<'_,_,_> [b] payload [b] constants {tag = 7};
    );
    source
        .structures
        .declare(&cq_lang_structures::provider::base::declarations())
        .unwrap();
    ir::contract::check(&source).unwrap();
    let target = cq_rewrite_passes::expand_lookups(&source, quote!(256)).unwrap();
    let mut expected = source.clone();
    expected
        .structures
        .entries
        .push(syn::parse_quote!(first stage = batch 256 expand;));
    assert_eq!(target, expected);
    assert_eq!(target.physical, source.physical);
}

fn fixture(suffix: &str) -> ir::Program {
    syn::parse_str(&format!(
        "{}\n{suffix}",
        include_str!("fixtures/representations.physical")
    ))
    .unwrap()
}

fn source() -> ir::Program {
    let source = fixture(include_str!("fixtures/representations.structures"));
    // This authored fixture explicitly retains buffers so the rows selector
    // remains covered independently of the default retention policy.
    ir::contract::check(&source).unwrap();
    source
}

fn missing_storage(target: &ir::Program) -> ir::Program {
    let mut target = target.clone();
    target.structures.entries.remove(0);
    target
}

fn changed_physical(target: &ir::Program) -> ir::Program {
    let mut target = target.clone();
    target.physical.program.head.args = target
        .physical
        .program
        .head
        .args
        .into_iter()
        .rev()
        .collect();
    target
}

#[test]
fn every_selector_has_a_pinned_ir_and_independent_mutation_checks() {
    for (arguments, expected) in [
        (
            {
                let declared = replacement(quote!(Replacement<'_, _, _>));
                quote!(#declared r => @replacement as Replacement<'_, _, _>)
            },
            include_str!("fixtures/retype_result.structures"),
        ),
        (
            {
                let declared = replacement(quote!(Replacement<'_, _, _>));
                quote!(#declared payload => @replacement as Replacement<'_, _, _>)
            },
            include_str!("fixtures/retype_payload.structures"),
        ),
        (
            {
                let declared = replacement(quote!(Replacement<_>));
                quote!(#declared membership => @replacement as Replacement<_>)
            },
            include_str!("fixtures/retype_membership.structures"),
        ),
        (
            {
                let declared = replacement(quote!(Replacement<_, Vec<_>>));
                quote!(#declared ordered => @replacement as Replacement<_, Vec<_>>)
            },
            include_str!("fixtures/retype_ordered.structures"),
        ),
        (
            {
                let declared = replacement(quote!(Replacement<'_, _, Inner<_>>));
                quote!(#declared trie => @replacement as Replacement<'_, _, Inner<_>>)
            },
            include_str!("fixtures/retype_trie.structures"),
        ),
        (
            quote!(
                providers { vec_deque = ::std::collections::VecDeque<_> { reads [], build typed }; }
                rows => @vec_deque as ::std::collections::VecDeque<_>
            ),
            include_str!("fixtures/retype_rows.structures"),
        ),
    ] {
        let checked = PassCase {
            source: source(),
            expected_next: Some(fixture(expected)),
            compile: Box::new(|p| retype(p, arguments.clone())),
            check: Box::new(ir::check_rewrite),
            mutations: vec![Box::new(missing_storage), Box::new(changed_physical)],
            answers: None,
        }
        .run(None)
        .unwrap();
        assert_eq!(checked.physical, source().physical);
        assert_eq!(
            syn::parse2::<ir::Program>(checked.to_token_stream()).unwrap(),
            checked
        );
        assert_eq!(retype(&checked, arguments).unwrap(), checked);
    }
}

#[test]
fn sorted_runs_preserves_the_observation_contract_and_records_only_eligible_types() {
    let checked = PassCase {
        source: source(),
        expected_next: Some(fixture(include_str!("fixtures/sorted_runs.structures"))),
        compile: Box::new(|p| cq_rewrite_passes::sorted_runs(p, quote!())),
        check: Box::new(ir::check_rewrite),
        mutations: vec![Box::new(missing_storage), Box::new(changed_physical)],
        answers: None,
    }
    .run(None)
    .unwrap();
    assert_eq!(
        cq_rewrite_passes::sorted_runs(&checked, quote!()).unwrap(),
        checked
    );
    assert!(cq_rewrite_passes::sorted_runs(&source(), quote!(unexpected)).is_err());
}

#[test]
fn selectors_are_by_recorded_reads_and_levels_not_old_type_spelling() {
    let original = source();
    let result = replacement(quote!(Replacement<'_, _, _>));
    assert_eq!(
        retype(
            &original,
            quote!(#result r#r => @replacement as Replacement<'_, _, _>)
        )
        .unwrap(),
        fixture(include_str!("fixtures/retype_result.structures"))
    );
    let alias = custom("alias", quote!(Alias<'_, _, _>), "typed");
    let renamed = retype(
        &original,
        quote!(providers { #alias } trie => @alias as Alias<'_, _, _>),
    )
    .unwrap();
    let trie = replacement(quote!(Replacement<'_, _, Inner<_>>));
    let target = retype(
        &renamed,
        quote!(#trie trie => @replacement as Replacement<'_, _, Inner<_>>),
    )
    .unwrap();
    assert_eq!(
        target,
        fixture(include_str!("fixtures/retype_trie.structures"))
    );
    assert_eq!(
        retype(
            &original,
            quote!(#result result(r) => @replacement as Replacement<'_, _, _>)
        )
        .unwrap(),
        fixture(include_str!("fixtures/retype_result.structures"))
    );
    assert_eq!(
        retype(
            &original,
            quote!(
                providers { vec_deque = ::std::collections::VecDeque<_> { reads [], build typed }; }
                kind(rows) => @vec_deque as ::std::collections::VecDeque<_>
            )
        )
        .unwrap(),
        fixture(include_str!("fixtures/retype_rows.structures"))
    );
}

#[test]
fn unmatched_or_malformed_selectors_are_errors() {
    let source = source();
    for arguments in [
        quote!(dedup => @hash_index as _),
        quote!(dedup => @hash_index as some_macro!()),
        quote!(trie => @hash_index as Generic<some_macro!()>),
        quote!(missing => @hash_index as Valid<_>),
        quote!(kind(scan) => @hash_index as Valid<_>),
        quote!(result(out) => @hash_index as Valid<_>),
        quote!(result(r, s) => @hash_index as Valid<_>),
        // STR-Provider: a replacement is a declared token, never a bare type.
        quote!(payload => Valid<_>),
        quote!(payload => @undeclared as Valid<_>),
        quote!(payload),
        quote!(payload => @hash_index as Valid<_>;),
        quote!(),
    ] {
        assert!(retype(&source, arguments.clone()).is_err(), "{arguments}");
    }
}

#[test]
fn provider_trait_compatibility_is_not_inferred_from_type_spelling() {
    let source = source();
    for arguments in [
        quote!(payload => @std_hash_set as ::std::collections::HashSet<_>),
        quote!(ordered => @hash_index as ::cq_storage::HashIndex<'_, _, _>),
    ] {
        // Retyping preserves the recorded layout. Rust trait checking, not
        // matching a provider's name, decides whether it can serve the reads.
        let target = retype(&source, arguments).unwrap();
        ir::check_rewrite(&source, &target).unwrap();
        assert_eq!(source.physical, target.physical);
    }
}

#[test]
fn computed_head_uses_sort_without_touching_physical() {
    let source: cq_testkit::physical::Program = syn::parse_quote!(physical {
        relation R(a:i32); r = let canon rename R {a -> x};
        return dedup r as answer(*x % 2);
    });
    let source = cq_structure_selection::compile(&source).unwrap();
    let expected: ir::Program = syn::parse_quote! {
        physical {
            relation R(a:i32); r = let canon rename R {a -> x};
            return dedup r as answer(*x % 2);
        }
        structures { providers { sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared }; host_scan = ::core::iter::Map<_, _> { reads [], build typed }; }
            r canon = rows; r rows = @sort as ::cq_storage::dedup::Sort;
            r host scan = @host_scan as ::core::iter::Map<_, _> [];
            r head = @sort as ::cq_storage::dedup::Sort;
        }
    };
    assert_eq!(source, expected);
    ir::contract::check(&source).unwrap();
}

#[test]
fn shared_host_fallbacks_cannot_disagree_after_retyping_one_kind() {
    let source: cq_testkit::physical::Program = syn::parse_quote!(physical {
        relation L(a:i32); relation R(a:i32);
        l = pipe rename L {a -> x}; r = pipe rename R {a -> x};
        s = pipe semi eager [x] l,r;
        p = let canon probe first [x] s,r; return p as answer(x);
    });
    let source = cq_structure_selection::compile(&source).unwrap();
    let shared_index = custom("shared_index", quote!(Shared<'_, _, _>), "typed");
    assert!(
        retype(
            &source,
            quote!(providers { #shared_index } payload => @shared_index as Shared<'_, _, _>)
        )
        .is_err()
    );
    let target = retype(
        &source,
        quote!(providers { #shared_index } r => @shared_index as Shared<'_, _, _>),
    )
    .unwrap();
    let expected: ir::Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32);
            l = pipe rename L {a -> x}; r = pipe rename R {a -> x};
            s = pipe semi eager [x] l,r;
            p = let canon probe first [x] s,r; return p as answer(x);
        }
        structures {
            providers {
                sort = ::cq_storage::dedup::Sort { reads [distinct], build inferred, shared };
                host_scan = ::core::iter::Map<_, _> { reads [], build typed };
                shared_index = Shared<'_, _, _> { reads [exact, membership], build typed };
            }
            l host scan = @host_scan as ::core::iter::Map<_, _> [];
            r host scan = @host_scan as ::core::iter::Map<_, _> [];
            r host result(s) = @shared_index as Shared<'_, _, _> [x] membership;
            r host result(p) = @shared_index as Shared<'_, _, _> [x] membership;
            p canon = rows; p rows = @sort as ::cq_storage::dedup::Sort;
        }
    };
    assert_eq!(target, expected);
    // Retyping the shared provider preserves STR-FullKey existence reads.
    ir::check_rewrite(&source, &target).unwrap();
    assert!(matches!(
        target.structures.entries[1].structure,
        Structure::Host { .. }
    ));
}

#[test]
fn selection_and_keyset_retype_preserve_lineage_policy_and_physical() {
    let physical = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(b:i64,c:i64);
        l = pipe rename L {a -> x,b -> y}; r = pipe rename R {b -> x,c -> z};
        s = let point select r if (z = 1);
        t = let semi eager [x] l,s;
        u = pipe semi eager [x] s,t; j = pipe probe first [x] u,t;
        out = let canon project j keep {x,y}; return out as answer(x,y);
    }};
    let source = cq_structure_selection::compile_with(&physical, quote!(positional(L, R))).unwrap();
    let membership = retype(
        &source,
        quote!(kind(membership) => @adaptive_keyset as ::cq_storage::AdaptiveKeySet<_,::cq_storage::I64Codec,64>),
    )
    .unwrap();
    assert_eq!(
        ir::contract::check(&source).unwrap().built,
        ir::contract::check(&membership).unwrap().built,
        "supplementary keysets serve membership without retyping payload indices"
    );
    assert!(
        retype(
            &source,
            quote!(providers { mask = Mask<_> { reads [], build typed }; } kind(selection) => @mask as Mask<_>),
        )
        .is_err()
    );
    let target = retype(
        &source,
        quote!(providers { mask = Mask<_> { reads [positions], build typed }; } kind(selection) => @mask as Mask<_>),
    )
    .unwrap();
    assert_eq!(source.physical, target.physical);
    let target = retype(
        &source,
        quote!(kind(keyset) => @adaptive_keyset as ::cq_storage::AdaptiveKeySet<_,::cq_storage::I64Codec,64>),
    )
    .unwrap();
    assert_eq!(source.physical, target.physical);
    let target = retype(
        &source,
        quote!(kind(membership) => @std_hash_set as ::std::collections::HashSet<_>),
    )
    .unwrap();
    assert_eq!(source.physical, target.physical);
    // Positional selection needs no ordinary rows buffers; only the output's
    // canonical rows provider is eligible for the rows selector.
    let rows: Vec<_> = source
        .structures
        .entries
        .iter()
        .filter(|entry| matches!(entry.structure, Structure::Rows { .. }))
        .map(|entry| entry.name.to_string())
        .collect();
    assert_eq!(rows, ["out"]);
    let canonical = retype(
        &source,
        quote!(
            providers { canonical_rows = CanonicalRows { reads [distinct], build inferred }; }
            kind(rows) => @canonical_rows as CanonicalRows
        ),
    )
    .unwrap();
    assert_eq!(canonical.physical, source.physical);
    assert_eq!(
        ir::contract::check(&canonical).unwrap().canonical["out"],
        ir::CanonSource::Rows(syn::parse_quote!(@canonical_rows as CanonicalRows)),
    );
    let mut row_only = source.clone();
    for e in &mut row_only.structures.entries {
        if let Structure::Read {
            predicate: ir::ReadPredicate::Membership { summary, .. },
            ..
        } = &mut e.structure
        {
            *summary = syn::parse_quote!(row);
        }
    }
    let row_only = retype(
        &row_only,
        quote!(kind(keyset) => @std_hash_set as ::std::collections::HashSet<_>),
    )
    .unwrap();
    ir::check_rewrite(&source, &row_only).unwrap();
}

#[test]
fn sorted_runs_keeps_two_column_payload_keys_hashed() {
    let physical = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(a:i64,b:i64,c:i64);
        l = pipe rename L {a -> x,b -> y};
        r = let rename R {a -> x,b -> y,c -> z};
        p = let canon probe first [x,y] l,r;
        return p as answer(x,y,z);
    }};
    let source = cq_structure_selection::compile(&physical).unwrap();
    assert!(source.structures.entries.iter().any(|entry| {
        entry.name == "r"
            && matches!(&entry.structure, Structure::Built { keyed, .. }
                if keyed.level_names() == [vec!["x".to_owned(), "y".to_owned()]]
                    && keyed.provider.token == "hash_index"
                    && keyed.provider.ty == syn::parse_quote!(::cq_storage::HashIndex<'_, _, _>))
    }));
    let target = cq_rewrite_passes::sorted_runs(&source, quote!()).unwrap();
    assert_eq!(target, source);
    ir::check_rewrite(&source, &target).unwrap();
}

#[test]
fn hashed_runs_records_composite_keys_and_preserves_single_key_selection() {
    let physical = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(a:i64,b:i64,c:i64);
        l = pipe rename L {a -> x,b -> y};
        r = let rename R {a -> x,b -> y,c -> z};
        p = let canon probe first [x,y] l,r;
        return p as answer(x,y,z);
    }};
    let source = cq_structure_selection::compile(&physical).unwrap();
    let target = cq_rewrite_passes::hashed_runs(&source, quote!()).unwrap();
    ir::check_rewrite(&source, &target).unwrap();
    let text = target.to_token_stream().to_string();
    assert!(
        text.contains("cq_gpu_runtime :: HashedRun < _ , _ > [x , y]"),
        "{text}"
    );
    assert_eq!(
        cq_rewrite_passes::hashed_runs(&target, quote!()).unwrap(),
        target
    );
    assert!(cq_rewrite_passes::hashed_runs(&source, quote!(unexpected)).is_err());
}

#[test]
fn sorted_runs_records_owned_singleton_probe_as_automatic_codec_request() {
    let physical = syn::parse_quote! { physical {
        relation R(a:i64,b:i64);
        u = pipe unit;
        l = pipe bind u with x:i64 = (meta.x);
        r = let rename R {a -> x,b -> y};
        p = let canon probe first[x] l,r;
        return p as answer(x,y);
    }};
    let source = cq_structure_selection::compile(&physical).unwrap();
    let target = cq_rewrite_passes::sorted_runs(&source, quote!()).unwrap();
    let text = target.to_token_stream().to_string();
    assert!(
        text.contains("SortedRun < _ , _ , :: cq_storage :: I64Codec > [x]"),
        "{text}"
    );
    assert_eq!(
        cq_rewrite_passes::hashed_runs(&target, quote!()).unwrap(),
        target
    );
    ir::check_rewrite(&source, &target).unwrap();
}

#[test]
fn hashed_runs_leaves_scalar_encoding_to_rust_traits() {
    for ty in [
        quote!(String),
        quote!(&str),
        quote!(bool),
        quote!(CustomKey),
        quote!(keys::KeyAlias),
    ] {
        let physical = syn::parse_quote! { physical {
            relation L(a:#ty,b:i64); relation R(a:#ty,b:i64,c:i64);
            l = pipe rename L {a -> x,b -> y};
            r = let rename R {a -> x,b -> y,c -> z};
            p = let canon probe first[x,y] l,r;
            return p as answer(x,y,z);
        }};
        let source = cq_structure_selection::compile(&physical).unwrap();
        let target = cq_rewrite_passes::hashed_runs(&source, quote!()).unwrap();
        assert!(target.to_token_stream().to_string().contains("HashedRun"));
        ir::check_rewrite(&source, &target).unwrap();
    }
}
