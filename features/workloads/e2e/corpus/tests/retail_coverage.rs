//! Catalog, shard union, and pair coverage without compiling any query pipeline.
#[macro_use]
#[path = "support/le_configurations.rs"]
mod le_configurations;

// Retain configuration witnesses but do not instantiate their query consumers.
macro_rules! pipeline_configuration {
    ($($tokens:tt)*) => {};
}
#[macro_use]
#[path = "support/configurations.rs"]
mod configurations;
#[path = "../matrix.rs"]
mod matrix;

macro_rules! retail_shards {
    ($($query:ident => $runner:ident),+ $(,)?) => {
        $(mod $query { case_configurations!($query, $runner); })+
        const CASES: &[(&str, &[[usize; 9]], &str)] = &[
            $((stringify!($query), $query::CONFIGURATIONS,
                include_str!(concat!(stringify!($query), "_matrix.rs")))),+
        ];
    };
}
retail_shards! {
    retail_d19 => run_retail_d19,
    retail_d27 => run_retail_d27,
    retail_d72_inner => run_retail_d72_inner,
    retail_d85 => run_retail_d85,
    retail_h03 => run_retail_h03,
    retail_h05 => run_retail_h05,
    retail_h07 => run_retail_h07,
    retail_h08 => run_retail_h08,
    retail_h09 => run_retail_h09,
    retail_h21_positive => run_retail_h21_positive,
}

#[test]
fn every_catalog_case_axis_value_and_pair_is_exercised() {
    assert_eq!(CASES.len(), 10);
    let mut included = Vec::new();
    let mut rows = Vec::new();
    for &(query, configurations, source) in CASES {
        // Inspect the actual thin binary source, not a second claimed query list.
        let file = syn::parse_file(source).expect("valid matrix entry point");
        let query_modules: Vec<_> = file.items.iter().filter_map(|item| {
            let syn::Item::Mod(module) = item else { return None };
            module.attrs.iter().find_map(|attribute| {
                if !attribute.path().is_ident("path") { return None; }
                let syn::Meta::NameValue(value) = &attribute.meta else { return None };
                let syn::Expr::Lit(value) = &value.value else { return None };
                let syn::Lit::Str(path) = &value.lit else { return None };
                path.value().contains("/queries/").then(|| (module.ident.to_string(), path.value()))
            })
        }).collect();
        assert_eq!(query_modules, vec![(query.to_owned(), format!("../../../src/queries/retail/{query}.rs"))]);
        included.push(query);
        rows.extend_from_slice(configurations);
    }
    // verify_catalog rejects duplicates as well as missing or extra query names.
    matrix::verify_catalog("retail", &included);
    matrix::verify_cover(&rows);
    eprintln!("{} disjoint query binaries, {} configuration cases", included.len(), rows.len());
}
