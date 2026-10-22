use quote::{ToTokens, quote};

fn source_tokens(module: &cq_lang_structures::Program) -> syn::Result<proc_macro2::TokenStream> {
    let analysis = cq_lang_structures::contract::check(module)?;
    let context = cq_physical_realization::sources(module, &analysis)?;
    let mut emitter = cq_physical_realization::RowEmitter::new(context);
    let rows = emitter.scan(&module.physical.program.returned)?;
    let statements = &emitter.context().statements;
    Ok(quote!(#(#statements)* #rows))
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
            r = ::cq_storage::SortedRun<_,_> [k];
            s share = lookup r [k] first s force p;
            s repr = tuples witness { r [k] at s as ::cq_storage::RunCursor<'_, _> };
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
        printed.matches("IndexRead :: lookup").count(),
        1,
        "{printed}"
    );
    assert!(!printed.contains("IndexRead :: probe"), "{printed}");
    assert!(printed.contains("stage :: segmented :: < 4"), "{printed}");
    assert!(printed.contains("__cq_witness_s"), "{printed}");
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
        printed.matches("IndexRead :: lookup").count(),
        1,
        "{printed}"
    );
    assert!(!printed.contains("stage :: segmented"), "{printed}");
}

#[test]
fn smallest_witness_selection_is_exact_and_host_prefix_retention_is_rejected() {
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
            r = ::cq_gpu::PrefixRun [b][c];
            s = ::cq_gpu::PrefixRun [a][c];
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
        error.contains("PrefixRun directory witnesses require the GPU printer"),
        "{error}"
    );
}
