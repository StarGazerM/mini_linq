use quote::quote;
fn main() {
    println!("cargo:rerun-if-changed=query.theta");
    let query: ra_theta::Query = syn::parse_str(include_str!("query.theta")).unwrap();
    let indexes: cq_lang_ix::Section = query.indexes().unwrap();
    let mut functions = Vec::new();
    for (name, policy) in [
        (
            syn::parse_quote!(run_probe),
            syn::parse_quote!(policy { L; R; }),
        ),
        (
            syn::parse_quote!(run_merge),
            syn::parse_quote!(policy { L; R; join = merge; }),
        ),
    ] {
        let name: syn::Ident = name;
        let policy: cq_lang_physical::policy::Policy = policy;
        let physical = cq_physical_lowering::compile(&indexes, &query, &policy).unwrap();
        let physical_env = cq_physical_lowering::Env::new(&indexes, &query).unwrap();
        let structures: cq_lang_structures::Section =
            cq_structure_selection::compile(&physical, &physical_env, &policy).unwrap();
        let structures_env = cq_structure_selection::Env::new(&physical, &physical_env).unwrap();
        let expression = cq_physical_realization::compile(
            &structures,
            &structures_env,
            &cq_physical_realization::Endpoints {
                distinct_state: syn::parse_quote!(distinct_state),
                intersect: false,
            },
        )
        .unwrap();
        functions.push(quote! {
            #[allow(non_snake_case)]
            pub fn #name(L: &[(i32, i32)], R: &[(i32, i32)]) -> Vec<(i32, i32)> {
                (#expression).map(|(x, y)| (*x, *y)).collect()
            }
        });
    }
    let path = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("query.rs");
    std::fs::write(path, quote!(#(#functions)*).to_string()).unwrap();
}
