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
