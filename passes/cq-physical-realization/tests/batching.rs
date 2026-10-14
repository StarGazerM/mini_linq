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
        let expression = cq_physical_realization::compile(&structures).unwrap();
        let printed = expression.to_token_stream().to_string();
        assert!(printed.contains("IndexRead :: probe"));
        assert_eq!(printed.contains("probe :: < 128usize"), expected);
        assert_eq!(printed.contains("probe :: < 1usize"), !expected);
    }
}
