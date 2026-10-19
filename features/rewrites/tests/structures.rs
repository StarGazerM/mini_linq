use cq_lang_structures::{self as ir, Structure};
use cq_rewrite_passes::retype;
use cq_testkit::PassCase;
use quote::{ToTokens, quote};

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

#[test]
fn default_selection_omits_the_authored_unneeded_buffers() {
    let mut expected = source();
    expected
        .structures
        .entries
        .retain(|e| !matches!(e.structure, Structure::Rows { .. }));
    assert_eq!(
        cq_structure_selection::compile(&expected.physical).unwrap(),
        expected
    );
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
            quote!(dedup => ::std::collections::BTreeSet<_>),
            include_str!("fixtures/retype_dedup.structures"),
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
fn incompatible_known_types_and_unmatched_or_malformed_selectors_are_errors() {
    let source = source();
    for arguments in [
        quote!(payload => ::std::collections::HashSet<_>),
        quote!(ordered => ::cq_storage::HashIndex<'_, _, _>),
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
fn computed_head_dedup_is_retyped_without_touching_physical() {
    let source: cq_testkit::physical::Program = syn::parse_quote!(physical {
        relation R(a:i32); r = pipe canon rename R@0 {a -> x};
        return r as answer(*x % 2);
    });
    let source = cq_structure_selection::compile(&source).unwrap();
    let target = retype(&source, quote!(dedup => ::std::collections::BTreeSet<_>)).unwrap();
    let expected: ir::Program = syn::parse_quote! {
        physical {
            relation R(a:i32); r = pipe canon rename R@0 {a -> x};
            return r as answer(*x % 2);
        }
        structures {
            r canon = ::std::collections::BTreeSet<_>;
            r host scan = ::core::iter::Map<_, _> [];
            r head = ::std::collections::BTreeSet<_>;
        }
    };
    assert_eq!(target, expected);
    ir::check_rewrite(&source, &target).unwrap();
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
    let error = retype(&source, quote!(payload => Shared<'_, _, _>)).unwrap_err();
    assert!(error.to_string().contains("shared host fallback types"));
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
            r host s = Shared<'_, _, _> [x];
            r host p = Shared<'_, _, _> [x];
            p canon = ::cq_storage::HashSet<_>;
        }
    };
    assert_eq!(target, expected);
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
    assert!(
        retype(
            &source,
            quote!(kind(membership) => ::std::collections::HashSet<_>)
        )
        .is_err()
    );
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
    assert!(text.contains("cq_gpu_runtime :: HashedRun < _ , _ > [x , y]"), "{text}");
    assert_eq!(cq_rewrite_passes::hashed_runs(&target, quote!()).unwrap(), target);
    assert!(cq_rewrite_passes::hashed_runs(&source, quote!(unexpected)).is_err());
}

#[test]
fn sorted_runs_records_owned_singleton_probe_as_no_dense() {
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
    assert!(text.contains("SortedRun < _ , _ , :: cq_storage :: NoDense > [x]"), "{text}");
    assert_eq!(cq_rewrite_passes::hashed_runs(&target, quote!()).unwrap(), target);
    ir::check_rewrite(&source, &target).unwrap();
}

#[test]
fn hashed_runs_accepts_dictionary_and_boolean_keys_but_not_opaque_types() {
    for (ty, eligible) in [
        (quote!(String), true), (quote!(&str), true), (quote!(bool), true),
        (quote!(CustomKey), false),
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
        assert_eq!(target.to_token_stream().to_string().contains("HashedRun"), eligible);
        ir::check_rewrite(&source, &target).unwrap();
    }
}
