//! Bounded probe lookahead must never force an unvisited opaque guard.
use quote::ToTokens;

#[test]
fn only_coarse_total_scans_use_lookahead() {
    for (outer, guard, expected) in [
        ("l", "", true),
        (
            "g",
            "g = pipe guard l if ({ assert!(*k != 2); true });",
            false,
        ),
        ("g", "g = pipe semi eager [k] l,r;", false),
    ] {
        let physical = syn::parse_str(&format!(
            "physical {{
                relation L(k:i32); relation R(k:i32,v:i32);
                l = pipe rename L@0 {{k -> k}};
                r = let rename R@1 {{k -> k,v -> v}};
                {guard}
                j = pipe canon probe first [k] {outer},r;
                return j as answer(k,v);
            }}"
        ))
        .unwrap();
        let structures = cq_structure_selection::compile(&physical).unwrap();
        let structures = cq_staging::compile(&structures).unwrap();
        let expression = cq_physical_realization::compile(&structures).unwrap();
        let printed = expression.to_token_stream().to_string();
        assert!(printed.contains("IndexRead :: probe"));
        assert_eq!(printed.contains("probe :: < 128usize"), expected);
        assert_eq!(printed.contains("probe :: < 1usize"), !expected);
    }
}

#[test]
fn recorded_scan_selections_use_chunk_masks_before_partial_rust() {
    let physical = syn::parse_quote! {
        physical {
            relation L(k:i32);
            l = pipe rename L@0 {k -> k};
            s = pipe select l if (k >= 2);
            t = pipe select s if (k <= 9);
            g = pipe canon guard t if (*k != 7);
            return g as answer(k);
        }
    };
    let selected = cq_structure_selection::compile(&physical).unwrap();
    let scalar = cq_physical_realization::compile(&selected)
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(!scalar.contains("for_chunks"));
    let staged = cq_staging::compile_with(&selected, quote::quote!(batch(64))).unwrap();
    let printed = cq_physical_realization::compile(&staged)
        .unwrap()
        .to_token_stream()
        .to_string();
    assert!(printed.contains("for_chunks :: < 64usize"));
    // Both total predicates run in one selection pass over one fetched row.
    assert_eq!(printed.matches("select_with").count(), 1);
    assert_eq!(printed.matches("relational_cmp").count(), 2);
    assert_eq!(printed.matches("PositionalRows :: row_at").count(), 2);
}
