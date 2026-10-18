use quote::{ToTokens, quote};

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
    let printed = cq_physical_realization::compile(&represented)
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
    let error = cq_physical_realization::compile(&unsupported)
        .unwrap_err()
        .to_string();
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
    let printed = cq_physical_realization::compile(&program)
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
    let printed = cq_physical_realization::compile(&scalar)
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
