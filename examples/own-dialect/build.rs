// These queries use only unit/rename/project/select/probe/semi in Written/Coarse
// with probe joins. Review this explicit allow-list when the corpus changes.
const SUPPORTED: [&str; 20] = [
    "intersection",
    "cartesian-product",
    "oriented-chain",
    "claw",
    "fact-star",
    "triangle",
    "cycle-4",
    "spade",
    "barbell",
    "loomis-whitney-4",
    "retail_h03",
    "retail_h05",
    "retail_h07",
    "retail_h08",
    "retail_h09",
    "retail_h21_positive",
    "retail_d19",
    "retail_d27",
    "retail_d72_inner",
    "retail_d85",
];

fn main() {
    println!("cargo:rerun-if-changed=../../workloads/src/queries");
    let catalog: std::collections::BTreeSet<_> =
        cq_workloads::catalog::all().map(|c| c.name).collect();
    assert_eq!(
        catalog,
        SUPPORTED.into_iter().collect(),
        "review push support for changed corpus"
    );
    let mut code = String::new();
    for name in SUPPORTED {
        let case = cq_workloads::case(name).unwrap();
        assert!(case.module().unwrap().query.is_basic());
        for host in ["slice", "hashmap", "btreemap"] {
            let leaf = name.replace('-', "_");
            code.push_str(&format!(
                "mod {leaf}_{host} {{ macro_rules! instantiate {{ ($($cq:tt)*) => {{ cq_workload_e2e::compare_push! {{ [cq_to_ra probe keep first {host} hash_distinct default; {name:?}] $($cq)* }} }}; }} cq_workloads::queries::{}::{leaf}::with_query!(instantiate); }}\n",
                case.suite.as_str()));
        }
    }
    // 20 queries x 3 hosts x 3 scenarios minus 3 Cartesian no-match: 177 pairs.
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out.join("corpus.rs"), code).unwrap();
}
