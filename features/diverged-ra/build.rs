use quote::quote;
fn main() {
    println!("cargo:rerun-if-changed=query.theta");
    let query: ra_theta::Query = syn::parse_str(include_str!("query.theta")).unwrap();
    let indexes: cq_lang_ix::Program = query.indexes().unwrap();
    let mut functions = Vec::new();
    for (name, policy) in [
        (syn::parse_quote!(run_probe), syn::parse_quote!(policy {})),
        (
            syn::parse_quote!(run_merge),
            syn::parse_quote!(policy {   join = merge; }),
        ),
    ] {
        let name: syn::Ident = name;
        let policy: cq_lang_physical::policy::Policy = policy;
        let physical = cq_physical_lowering::compile(&indexes, &policy).unwrap();
        let structures: cq_lang_structures::Program =
            cq_structure_selection::compile(&physical, &policy).unwrap();
        let expression = cq_physical_realization::compile(
            &structures,
            &cq_physical_realization::Endpoints {
                distinct_state: syn::parse_quote!(distinct_state),
                index_provider: syn::parse_quote!(index_provider),
                intersect: false,
            },
        )
        .unwrap();
        let inputs = policy.merge_joins().then(|| {
            quote! {
                let mut lm = ::std::collections::BTreeMap::<_,Vec<_>>::new();
                let mut rm = ::std::collections::BTreeMap::<_,Vec<_>>::new();
                for &(k,v) in L { lm.entry((k,)).or_default().push((v,)); }
                for &(k,v) in R { rm.entry((k,)).or_default().push((v,)); }
                let L = &lm; let R = &rm;
            }
        });
        functions.push(quote! {
            #[allow(non_snake_case)]
            pub fn #name(L: &[(i32, i32)], R: &[(i32, i32)]) -> Vec<(i32, i32)> {
                #inputs
                (#expression).map(|(x, y)| (*x, *y)).collect()
            }
        });
    }
    let path = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("query.rs");
    std::fs::write(path, quote!(#(#functions)*).to_string()).unwrap();
}
