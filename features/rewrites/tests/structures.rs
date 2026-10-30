use cq_lang_structures::{self as ir, Structure};
use cq_structure_selection::retype;
use cq_testkit::PassCase;
use quote::{ToTokens, quote};

#[test]
fn membership_advice_preserves_a_fully_bound_physical_probe() {
    let source: ir::Program = syn::parse_quote! { physical {
        relation L(a:u32,b:u32); relation R(a:u32,b:u32);
        l = pipe rename L@0 {a -> x,b -> y};
        r = let rename R@1 {a -> x,b -> y};
        joined = pipe canon probe first [x,y] l,r;
        return joined as answer(x,y);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        r = ::cq_storage::HashIndex<'_,_,_> [x,y] supports [keys];
        joined canon = sort;
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
        l = pipe rename L@0 {a -> x};
        r = let rename R@1 {a -> x,b -> y};
        joined = pipe canon probe first [x] l,r;
        return joined as answer(x,y);
    } structures {
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        r = ::cq_storage::HashIndex<'_,_,_> [x];
        joined canon = sort;
    }};
    assert!(cq_rewrite_passes::membership(&source, quote!(r)).is_err());
}

fn smallest_expand_source() -> ir::Program {
    syn::parse_quote! { physical {
        relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
        l = pipe rename L@0 {a -> a,b -> b};
        r = let rename R@1 {b -> b,c -> c};
        s = let rename S@2 {a -> a,c -> c};
        candidate = pipe probe smallest [b] l,r;
        checked = pipe canon probe first [a,c] candidate,s;
        return checked as answer(a,b,c);
    } structures {
        r = CustomDirectory [b][c] supports [expand];
        s = CustomDirectory [a][c] supports [expand];
        l host scan = ::core::iter::Map<_, _> [];
        r host scan = ::core::iter::Map<_, _> [];
        s host scan = ::core::iter::Map<_, _> [];
        r rows = ::std::vec::Vec<_>;
        s rows = ::std::vec::Vec<_>;
        checked canon = sort;
    } }
}

#[test]
fn entry_retype_changes_only_one_provider_and_checks_the_result() {
    let source = smallest_expand_source();
    let target = retype(&source, quote!(entry(4) => DeviceColumns)).unwrap();
    let mut expected = source.clone();
    expected.structures.entries[4] = syn::parse_quote!(s host scan = DeviceColumns [];);
    assert_eq!(target, expected);
    ir::check_rewrite(&source, &target).unwrap();
    assert!(retype(&source, quote!(entry(100) => DeviceColumns)).is_err());
    assert!(retype(&source, quote!(entry(4, 5) => DeviceColumns)).is_err());
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
    for (index, entry) in [
        (0, syn::parse_quote!(r = OtherTrie [b][c];)),
        (1, syn::parse_quote!(s = OtherTrie [a][c];)),
        (
            0,
            syn::parse_quote!(r = ::cq_storage::SortedRun<_, _> [b][c];),
        ),
    ] {
        let mut source = smallest_expand_source();
        source.structures.entries[index] = entry;
        ir::contract::check(&source).unwrap();
        assert_eq!(
            cq_rewrite_passes::expand_lookups(&source, quote!(256)).unwrap(),
            source
        );
    }
    let mut mixed = smallest_expand_source();
    mixed.structures.entries[1] = syn::parse_quote!(s = OtherTrie [a][c];);
    mixed
        .structures
        .entries
        .push(syn::parse_quote!(r = ::cq_storage::SortedRun<_, _> [b];));
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
        l = pipe rename L@0 {a -> a};
        r = let rename R@1 {a -> a,b -> b};
        m = pipe rename M@2 {b -> b,tag -> tag};
        selected = let point select m if (tag = 7);
        s = let rename S@3 {b -> b,c -> c};
        t = let rename T@4 {c -> c,d -> d};
        first = pipe probe first [a] l,r;
        member = pipe probe first [b] first,selected;
        after = pipe probe first [b] member,s;
        last = pipe probe first [c] after,t;
        out = pipe canon project last keep {a,b,c,d,tag};
        return out as answer(a,b,c,d,tag);
    } structures {
        selected = ::cq_storage::AdaptiveKeySet<_,::cq_storage::I64Codec,64>
            [b] membership payload [b] constants {tag = 7};
        r = ::cq_storage::SortedRun<_,_> [a] supports [expand];
        s = ::cq_storage::SortedRun<_,_> [b] supports [expand];
        t = ::cq_storage::SortedRun<_,_> [c] supports [expand];
        l host scan = ::core::iter::Map<_,_> [];
        r host scan = ::core::iter::Map<_,_> [];
        m host scan = ::core::iter::Map<_,_> [];
        m host result(selected) = ::core::iter::Map<_,_> [tag];
        s host scan = ::core::iter::Map<_,_> [];
        t host scan = ::core::iter::Map<_,_> [];
        out canon = sort;
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
        selected = ::cq_storage::HashIndex<'_,_,_> [b] payload [b] constants {tag = 7};
    );
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
            quote!(r => Replacement<'_, _, _>),
            include_str!("fixtures/retype_result.structures"),
        ),
        (
            quote!(payload => Replacement<'_, _, _>),
            include_str!("fixtures/retype_payload.structures"),
        ),
        (
            quote!(membership => Replacement<_>),
            include_str!("fixtures/retype_membership.structures"),
        ),
        (
            quote!(ordered => Replacement<_, Vec<_>>),
            include_str!("fixtures/retype_ordered.structures"),
        ),
        (
            quote!(trie => Replacement<'_, _, Inner<_>>),
            include_str!("fixtures/retype_trie.structures"),
        ),
        (
            quote!(rows => ::std::collections::VecDeque<_>),
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
    assert_eq!(
        retype(&original, quote!(r#r => Replacement<'_, _, _>)).unwrap(),
        fixture(include_str!("fixtures/retype_result.structures"))
    );
    let renamed = retype(&original, quote!(trie => Alias<'_, _, _>)).unwrap();
    let target = retype(&renamed, quote!(trie => Replacement<'_, _, Inner<_>>)).unwrap();
    assert_eq!(
        target,
        fixture(include_str!("fixtures/retype_trie.structures"))
    );
    assert_eq!(
        retype(&original, quote!(result(r) => Replacement<'_, _, _>)).unwrap(),
        fixture(include_str!("fixtures/retype_result.structures"))
    );
    assert_eq!(
        retype(
            &original,
            quote!(kind(rows) => ::std::collections::VecDeque<_>)
        )
        .unwrap(),
        fixture(include_str!("fixtures/retype_rows.structures"))
    );
}

#[test]
fn unmatched_or_malformed_selectors_are_errors() {
    let source = source();
    for arguments in [
        quote!(dedup => _),
        quote!(dedup => some_macro!()),
        quote!(trie => Generic<some_macro!()>),
        quote!(missing => Valid<_>),
        quote!(kind(scan) => Valid<_>),
        quote!(result(out) => Valid<_>),
        quote!(result(r, s) => Valid<_>),
        quote!(payload),
        quote!(payload => Valid<_>;),
        quote!(),
    ] {
        assert!(retype(&source, arguments.clone()).is_err(), "{arguments}");
    }
}

#[test]
fn provider_trait_compatibility_is_not_inferred_from_type_spelling() {
    let source = source();
    for arguments in [
        quote!(payload => ::std::collections::HashSet<_>),
        quote!(ordered => ::cq_storage::HashIndex<'_, _, _>),
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
        relation R(a:i32); r = pipe canon rename R@0 {a -> x};
        return r as answer(*x % 2);
    });
    let source = cq_structure_selection::compile(&source).unwrap();
    let expected: ir::Program = syn::parse_quote! {
        physical {
            relation R(a:i32); r = pipe canon rename R@0 {a -> x};
            return r as answer(*x % 2);
        }
        structures {
            r canon = sort;
            r host scan = ::core::iter::Map<_, _> [];
            r head = sort;
        }
    };
    assert_eq!(source, expected);
    ir::contract::check(&source).unwrap();
}

#[test]
fn shared_host_fallbacks_cannot_disagree_after_retyping_one_kind() {
    let source: cq_testkit::physical::Program = syn::parse_quote!(physical {
        relation L(a:i32); relation R(a:i32);
        l = pipe rename L@0 {a -> x}; r = pipe rename R@1 {a -> x};
        s = pipe semi eager [x] l,r;
        p = pipe canon probe first [x] s,r; return p as answer(x);
    });
    let source = cq_structure_selection::compile(&source).unwrap();
    assert!(retype(&source, quote!(payload => Shared<'_, _, _>)).is_err());
    let target = retype(&source, quote!(r => Shared<'_, _, _>)).unwrap();
    let expected: ir::Program = syn::parse_quote! {
        physical {
            relation L(a:i32); relation R(a:i32);
            l = pipe rename L@0 {a -> x}; r = pipe rename R@1 {a -> x};
            s = pipe semi eager [x] l,r;
            p = pipe canon probe first [x] s,r; return p as answer(x);
        }
        structures {
            l host scan = ::core::iter::Map<_, _> [];
            r host scan = ::core::iter::Map<_, _> [];
            r host result(s) = Shared<'_, _, _> [x] membership;
            r host result(p) = Shared<'_, _, _> [x] membership;
            p canon = sort;
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
        l = pipe rename L@0 {a -> x,b -> y}; r = pipe rename R@1 {b -> x,c -> z};
        s = let point select r if (z = 1);
        t = let semi eager [x] l,s;
        u = pipe semi eager [x] s,t; j = pipe probe first [x] u,t;
        out = pipe canon project j keep {x,y}; return out as answer(x,y);
    }};
    let source = cq_structure_selection::compile_with(&physical, quote!(positional(L, R))).unwrap();
    let membership = retype(
        &source,
        quote!(kind(membership) => ::cq_storage::AdaptiveKeySet<_,::cq_storage::I64Codec,64>),
    )
    .unwrap();
    assert_eq!(
        ir::contract::check(&source).unwrap().built,
        ir::contract::check(&membership).unwrap().built,
        "supplementary keysets serve membership without retyping payload indices"
    );
    let target = retype(&source, quote!(kind(selection) => Mask<_>)).unwrap();
    assert_eq!(source.physical, target.physical);
    let target = retype(
        &source,
        quote!(kind(keyset) => ::cq_storage::AdaptiveKeySet<_,::cq_storage::I64Codec,64>),
    )
    .unwrap();
    assert_eq!(source.physical, target.physical);
    let target = retype(
        &source,
        quote!(kind(membership) => ::std::collections::HashSet<_>),
    )
    .unwrap();
    assert_eq!(source.physical, target.physical);
    assert!(retype(&source, quote!(kind(rows) => ::std::vec::Vec<_>)).is_err());
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
        quote!(kind(keyset) => ::std::collections::HashSet<_>),
    )
    .unwrap();
    ir::check_rewrite(&source, &row_only).unwrap();
}

#[test]
fn sorted_runs_keeps_two_column_payload_keys_hashed() {
    let physical = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(a:i64,b:i64,c:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = let rename R@1 {a -> x,b -> y,c -> z};
        p = pipe canon probe first [x,y] l,r;
        return p as answer(x,y,z);
    }};
    let source = cq_structure_selection::compile(&physical).unwrap();
    assert!(source.structures.entries.iter().any(|entry| {
        entry.name == "r"
            && matches!(&entry.structure, Structure::Built { keyed, .. }
                if keyed.level_names() == [vec!["x".to_owned(), "y".to_owned()]]
                    && keyed.ty == syn::parse_quote!(::cq_storage::HashIndex<'_, _, _>))
    }));
    let target = cq_rewrite_passes::sorted_runs(&source, quote!()).unwrap();
    assert_eq!(target, source);
    ir::check_rewrite(&source, &target).unwrap();
}

#[test]
fn hashed_runs_records_composite_keys_and_preserves_single_key_selection() {
    let physical = syn::parse_quote! { physical {
        relation L(a:i64,b:i64); relation R(a:i64,b:i64,c:i64);
        l = pipe rename L@0 {a -> x,b -> y};
        r = let rename R@1 {a -> x,b -> y,c -> z};
        p = pipe canon probe first [x,y] l,r;
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
        r = let rename R@0 {a -> x,b -> y};
        p = pipe canon probe first[x] l,r;
        return p as answer(x,y);
    }};
    let source = cq_structure_selection::compile(&physical).unwrap();
    let target = cq_rewrite_passes::sorted_runs(&source, quote!()).unwrap();
    let text = target.to_token_stream().to_string();
    assert!(
        text.contains("SortedRun < _ , _ , :: cq_storage :: AutoDense > [x]"),
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
            l = pipe rename L@0 {a -> x,b -> y};
            r = let rename R@1 {a -> x,b -> y,c -> z};
            p = pipe canon probe first[x,y] l,r;
            return p as answer(x,y,z);
        }};
        let source = cq_structure_selection::compile(&physical).unwrap();
        let target = cq_rewrite_passes::hashed_runs(&source, quote!()).unwrap();
        assert!(target.to_token_stream().to_string().contains("HashedRun"));
        ir::check_rewrite(&source, &target).unwrap();
    }
}
