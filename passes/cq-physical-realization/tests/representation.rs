use quote::{ToTokens, quote};

#[test]
fn native_canon_counts_actual_inserts_without_wrapping_custom_providers() {
    for ty in [
        syn::parse_quote!(::cq_storage::HashSet<_>),
        syn::parse_quote!(::std::collections::HashSet<_, CustomHashState>),
    ] {
        let rows = cq_physical_realization::RowEmitter::canon(&ty, quote!(rows), 1);
        syn::parse2::<syn::Expr>(rows.clone()).unwrap();
        let printed = rows.to_string();
        assert!(printed.contains("profile_select"), "{printed}");
        assert!(printed.contains("DistinctSet :: from_set"), "{printed}");
        let constructor = quote!(<#ty>::new()).to_string();
        assert_eq!(printed.matches(&constructor).count(), 2, "{printed}");
    }
    for ty in [
        syn::parse_quote!(::cq_storage::counters::DistinctSet<_>),
        syn::parse_quote!(crate::tpch_runtime::Dedup<_>),
        syn::parse_quote!(::std::collections::BTreeSet<_>),
    ] {
        let rows = cq_physical_realization::RowEmitter::canon(&ty, quote!(rows), 1);
        let printed = rows.to_string();
        assert!(!printed.contains("from_set"), "{printed}");
        assert!(!printed.contains("profile_select"), "{printed}");
    }
}

fn source_tokens(module: &cq_lang_structures::Program) -> syn::Result<proc_macro2::TokenStream> {
    let analysis = cq_lang_structures::contract::check(module)?;
    let context = cq_physical_realization::sources_with(module, &analysis, |_, _| {})?;
    let mut emitter = cq_physical_realization::RowEmitter::new(context);
    let rows = emitter.scan(&module.physical.program.returned)?;
    let statements = &emitter.context().statements;
    Ok(quote!(#(#statements)* #rows))
}

#[test]
fn grouped_adapter_emits_one_typed_hash_fold_over_complete_valuations() {
    let physical = syn::parse_quote! { physical {
        relation R(k:i32,id:i32,v:i32);
        relation answer(k:i32,n:usize,total:i32,lo:Option<i32>,hi:Option<i32>);
        r = pipe canon rename R@0 {k -> k,id -> id,v -> v};
        grouped = pipe group r by [k] {
            n:usize = count(); total:i32 = sum(v);
            lo:Option<i32> = min(v); hi:Option<i32> = max(v);
        };
        return grouped as answer(k,n,total,lo,hi);
    }};
    let selected = cq_structure_selection::compile(&physical).unwrap();
    let facts = cq_lang_structures::contract::check(&selected).unwrap();
    let mut context = cq_print::Context::new(&selected, &facts);
    context.register("r");
    context.register("grouped");
    assert_eq!(context.carried("r"), ["id", "k", "v"]);
    let mut emitter = cq_physical_realization::RowEmitter::new(context);
    let rows = emitter
        .operator("grouped", Some(("r".into(), quote!(__canonical_rows))))
        .unwrap();
    syn::parse2::<syn::Expr>(rows.clone()).unwrap();
    let printed = rows.to_string();
    assert_eq!(printed.matches("for ").count(), 1, "{printed}");
    assert!(printed.contains("HashMap"), "{printed}");
    assert!(printed.contains("cq_storage :: HashState"), "{printed}");
    assert!(
        printed.contains("usize , i32 , Option < i32 > , Option < i32 >"),
        "{printed}"
    );
    assert!(printed.contains("or_insert_with"), "{printed}");
    assert!(printed.contains("Option :: Some"), "{printed}");
    assert!(printed.contains("< __cq_previous"), "{printed}");
    assert!(printed.contains("> __cq_previous"), "{printed}");
    let complete = source_tokens(&selected).unwrap();
    syn::parse2::<syn::Expr>(quote!({ #complete })).unwrap();
    let authored = cq_structure_selection::retype(
        &selected,
        quote!(group => ::std::collections::BTreeMap<_, (usize,i32,Option<i32>,Option<i32>)>),
    )
    .unwrap();
    let printed = source_tokens(&authored).unwrap().to_string();
    assert!(printed.contains("BTreeMap"), "{printed}");
    assert!(!printed.contains("HashMap"), "{printed}");
    assert!(printed.contains("Default :: default"), "{printed}");
    let standard = cq_structure_selection::retype(
        &selected,
        quote!(group => ::std::collections::HashMap<_, (usize,i32,Option<i32>,Option<i32>)>),
    )
    .unwrap();
    let printed = source_tokens(&standard).unwrap().to_string();
    assert!(
        printed.contains("std :: collections :: HashMap"),
        "{printed}"
    );
    assert!(!printed.contains("cq_storage :: HashState"), "{printed}");
}

#[test]
fn global_group_emits_an_identity_row_even_when_the_input_loop_is_empty() {
    let physical = syn::parse_quote! { physical {
        relation R(v:i32);
        relation answer(n:usize,total:i32,lo:Option<i32>,hi:Option<i32>);
        r = pipe canon rename R@0 {v -> v};
        grouped = pipe group r by [] {
            n:usize = count(); total:i32 = sum(v);
            lo:Option<i32> = min(v); hi:Option<i32> = max(v);
        };
        return grouped as answer(n,total,lo,hi);
    }};
    let selected = cq_structure_selection::compile(&physical).unwrap();
    let facts = cq_lang_structures::contract::check(&selected).unwrap();
    let mut context = cq_print::Context::new(&selected, &facts);
    context.register("r");
    context.register("grouped");
    let mut emitter = cq_physical_realization::RowEmitter::new(context);
    let rows = emitter
        .operator(
            "grouped",
            Some(("r".into(), quote!(::core::iter::empty::<(&i32,)>()))),
        )
        .unwrap();
    syn::parse2::<syn::Expr>(rows.clone()).unwrap();
    let printed = rows.to_string();
    assert!(!printed.contains("HashMap"), "{printed}");
    assert!(printed.contains("0usize"), "{printed}");
    assert!(printed.contains("default"), "{printed}");
    assert_eq!(printed.matches("Option :: None").count(), 2, "{printed}");
    assert!(printed.contains("iter :: once"), "{printed}");
}

#[test]
fn positions_print_fetches_and_tuple_boundaries_after_probes() {
    let physical = syn::parse_quote! {
        physical {
            relation L(k:i32); relation R(k:i32,v:i32);
            l = pipe rename L@0 {k -> k};
            s = pipe select l if (k >= 2);
            r = let rename R@1 {k -> k,v -> v};
            j = pipe canon probe first [k] s,r;
            return j as answer(k,v);
        }
    };
    let selected = cq_structure_selection::compile(&physical).unwrap();
    let represented = cq_represent::compile_with(&selected, quote!(positions)).unwrap();
    let printed = source_tokens(&represented)
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(printed.contains("PositionalRows :: positions"));
    assert!(printed.contains("PositionalRows :: row_at"));
    assert!(printed.contains("expect (\"host-issued live ID\") . 0"));
    assert!(!represented.to_token_stream().to_string().contains("j repr"));
    let mut unsupported = represented;
    unsupported.structures.entries.push(syn::parse_quote! {
        j repr = positions { L lineage { k -> k }, R lineage { k -> k, v -> v } } owned [];
    });
    let error = source_tokens(&unsupported).unwrap_err().to_string();
    assert!(error.contains("j repr positions with 2 bases"), "{error}");
}

#[test]
fn retained_lookup_is_a_pruned_non_column_cursor() {
    let program: cq_lang_structures::Program = syn::parse_quote! {
        physical {
            relation L(k:i32); relation R(k:i32,v:i32);
            l = pipe rename L@0 {k -> k};
            r = let rename R@1 {k -> k,v -> v};
            s = pipe semi eager [k] l,r;
            kept = pipe select s if (k >= 1);
            projected = pipe project kept keep {k};
            p = pipe probe first [k] projected,r;
            out = pipe canon project p keep {k,v};
            return out as answer(k,v);
        }
        structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            r = ForeignIndex<_,_> [k] supports [expand, keys];
            s share = lookup r [k] first s force p;
            s repr = tuples witness { r [k] at s as AuthoredBookmark<'_, _> };
            p stage = batch 4 expand;
            out canon = ::std::collections::HashSet<_>;
        }
    };
    let analysis = cq_lang_structures::contract::check(&program).unwrap();
    let context = cq_print::Context::new(&program, &analysis);
    for result in ["s", "kept", "projected"] {
        assert_eq!(context.carried_witnesses[result], ["s"]);
        assert_eq!(context.carried(result), ["k"]);
    }
    assert!(context.carried_witnesses["p"].is_empty());
    let printed = source_tokens(&program)
        .unwrap()
        .to_token_stream()
        .to_string();
    assert_eq!(
        printed.matches("provider_contract :: retain (").count(),
        1,
        "{printed}"
    );
    assert!(!printed.contains("IndexRead :: probe"), "{printed}");
    assert!(printed.contains("stage :: segmented :: < 4"), "{printed}");
    assert!(printed.contains("__cq_witness_s"), "{printed}");
    assert!(printed.contains("AuthoredBookmark < '_ , _ >"), "{printed}");
    assert!(
        printed.contains("provider_contract :: require_expand"),
        "{printed}"
    );
    assert!(
        printed.contains("provider_contract :: require_keys"),
        "{printed}"
    );
    let mut scalar = program.clone();
    scalar
        .structures
        .entries
        .retain(|entry| !matches!(entry.structure, cq_lang_structures::Structure::Stage { .. }));
    let printed = source_tokens(&scalar)
        .unwrap()
        .to_token_stream()
        .to_string();
    assert_eq!(
        printed.matches("provider_contract :: retain (").count(),
        1,
        "{printed}"
    );
    assert!(!printed.contains("stage :: segmented"), "{printed}");
    let witness = scalar
        .structures
        .entries
        .iter_mut()
        .find(|entry| matches!(entry.structure, cq_lang_structures::Structure::Repr { .. }))
        .unwrap();
    *witness = syn::parse_quote!(s repr = tuples witness { r [k] at s as _ };);
    let inferred = source_tokens(&scalar).unwrap().to_string();
    assert!(inferred.contains("let __cq_witness_s : _"), "{inferred}");
}

#[test]
fn smallest_witness_selection_is_exact_and_native_prefix_retention_is_rejected() {
    let program: cq_lang_structures::Program = syn::parse_quote! {
        physical {
            relation L(a:i32,b:i32); relation R(b:i32,c:i32); relation S(a:i32,c:i32);
            l = pipe rename L@0 {a -> a,b -> b};
            r = let rename R@1 {b -> b,c -> c};
            s = let rename S@2 {a -> a,c -> c};
            pre_r = pipe semi eager [b] l,r;
            pre_s = pipe semi eager [a] pre_r,s;
            candidate = pipe probe smallest [b] pre_s,r;
            checked = pipe canon semi eager [a,c] candidate,s;
            return checked as answer(a,b,c);
        } structures {
            l host scan = ::core::iter::Map<_, _> [];
            r host scan = ::core::iter::Map<_, _> [];
            s host scan = ::core::iter::Map<_, _> [];
            r = ForeignDirectory [b][c] supports [retain, expand];
            s = ForeignDirectory [a][c] supports [retain, expand];
            r rows = ::std::vec::Vec<_>;
            s rows = ::std::vec::Vec<_>;
            checked canon = ::std::collections::HashSet<_>;
            pre_r share = lookup r [b] first pre_r force candidate;
            pre_s share = lookup s [a] first pre_s force candidate;
            pre_r repr = tuples witness { r [b] at pre_r as ::cq_storage::RunCursor<'_, _> };
            pre_s repr = tuples witness { s [a] at pre_s as ::cq_storage::RunCursor<'_, _> };
        }
    };
    let analysis = cq_lang_structures::contract::check(&program).unwrap();
    let context = cq_print::Context::new(&program, &analysis);
    assert_eq!(context.carried_witnesses["pre_s"], ["pre_r", "pre_s"]);
    for (build, key, first) in [("r", "b", "pre_r"), ("s", "a", "pre_s")] {
        assert_eq!(
            context.forcing_witness("candidate", build, &[key.into()]),
            Some(context.witness_ident(first))
        );
    }
    assert!(
        context
            .forcing_witness("checked", "s", &["a".into()])
            .is_none()
    );
    assert!(
        context
            .forcing_witness("candidate", "r", &["a".into()])
            .is_none()
    );
    assert!(
        context
            .forcing_witness("candidate", "l", &["b".into()])
            .is_none()
    );
    let error = source_tokens(&program).unwrap_err().to_string();
    assert!(
        error.contains("candidate-prefix witnesses are unsupported"),
        "{error}"
    );
    let mut staged = program;
    staged.structures.entries.retain(|entry| {
        !matches!(
            entry.structure,
            cq_lang_structures::Structure::Repr { .. }
                | cq_lang_structures::Structure::Share { .. }
        )
    });
    staged
        .structures
        .entries
        .push(syn::parse_quote!(candidate stage = batch 4 expand;));
    let error = source_tokens(&staged).unwrap_err().to_string();
    assert!(
        error.contains("native smallest candidate selection"),
        "{error}"
    );
}

#[test]
fn retained_host_uses_borrowed_dispatch_and_consumes_the_same_cursor() {
    let program: cq_lang_structures::Program = syn::parse_quote! {
        physical {
            relation L(a:i64); relation R(a:i64,b:i64);
            l = pipe rename L@0 {a -> x};
            r = pipe rename R@1 {a -> x,b -> y};
            s = pipe semi eager [x] l,r;
            p = pipe canon probe first [x] s,r;
            return p as answer(x,y);
        } structures {
            l host scan = ::core::iter::Map<_,_> [];
            r host scan = ::core::iter::Map<_,_> [];
            r host result(s) = SnapshotAlias<_,_> [x] supports [expand, keys];
            r host result(p) = SnapshotAlias<_,_> [x] supports [expand, keys];
            s share = lookup r [x] first s force p;
            s repr = tuples witness { r [x] at s as _ };
            p stage = batch 4 expand;
            p canon = ::std::collections::HashSet<_>;
        }
    };
    let printed = source_tokens(&program).unwrap().to_string();
    assert!(printed.contains("BorrowedHost"), "{printed}");
    assert!(printed.contains("expanded_read"), "{printed}");
    assert!(printed.contains("let __cq_witness_s : _"), "{printed}");
    assert_eq!(
        printed.matches("provider_contract :: retain (").count(),
        1,
        "{printed}"
    );
    assert!(printed.contains("require_expand"), "{printed}");
    assert!(printed.contains("require_keys"), "{printed}");
    assert!(printed.contains("stage :: segmented"), "{printed}");
}
