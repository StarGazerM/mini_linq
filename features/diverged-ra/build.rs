use quote::quote;

fn main() {
    println!("cargo:rerun-if-changed=query.theta");
    println!("cargo:rerun-if-changed=partial.theta");
    let mut functions = Vec::new();
    for (text, probe_name, merge_name, partial) in [
        (include_str!("query.theta"), "run_probe", "run_merge", false),
        (
            include_str!("partial.theta"),
            "run_partial_probe",
            "run_partial_merge",
            true,
        ),
    ] {
        let query: ra_theta::Query = syn::parse_str(text).unwrap();
        let indexes = query.indexes().unwrap();
        for (name, merge) in [(probe_name, false), (merge_name, true)] {
            let name: syn::Ident = syn::parse_str(name).unwrap();
            let parameters = partial.then(|| quote!(, partial: fn(&i32, &i32) -> bool));
            let rewrite = merge.then(|| quote!(theta_passes::merge,));
            let inputs = merge.then(|| {
                quote! {
                    let mut lm = ::std::collections::BTreeMap::<_, Vec<_>>::new();
                    let mut rm = ::std::collections::BTreeMap::<_, Vec<_>>::new();
                    for &(k, v) in L { lm.entry((k,)).or_default().push((v,)); }
                    for &(k, v) in R { rm.entry((k,)).or_default().push((v,)); }
                    let L = &lm;
                    let R = &rm;
                }
            });
            functions.push(quote! {
                #[allow(non_snake_case)]
                pub fn #name(L: &[(i32, i32)], R: &[(i32, i32)] #parameters)
                    -> Vec<(i32, i32)>
                {
                    #inputs
                    theta_passes::lower! {
                        pipeline {
                            #rewrite theta_passes::select, theta_passes::sequence, theta_passes::sequential, theta_passes::translate_pull
                        };
                        #indexes
                    }
                    .map(|(x, y)| (*x, *y))
                    .collect()
                }
            });
        }
    }
    let path = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("query.rs");
    std::fs::write(path, quote!(#(#functions)*).to_string()).unwrap();
}
