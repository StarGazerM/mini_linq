mod matrix;

use std::{fmt::Write, path::PathBuf};

fn main() {
    matrix::verify_cover();
    println!("cargo:rerun-if-changed=matrix.rs");
    println!("cargo:rerun-if-changed=../../src/queries");
    println!("cargo:rerun-if-changed=../../src/pipelines.rs");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let mut census = String::from("query\tconfig\tstatus\tscenarios\treason\n");
    let mut differential = 0;
    let mut compile_fail = 0;
    let mut rejections = std::collections::BTreeMap::new();
    let mut baseline = std::collections::BTreeMap::new();
    let mut accepted = Vec::new();
    for planner in 0..matrix::AXES[0].len() {
        let mut code = String::new();
        let mut negative = String::from("fn main() {}\n");
        let mut expected = String::new();
        for case in cq_workloads::catalog::all() {
            for c in matrix::configs(case.name)
                .into_iter()
                .filter(|c| c[0] == planner)
            {
                let leaf = case.name.replace('-', "_");
                let v = matrix::values(c);
                let label = v.join("_");
                let name = format!("{leaf}_{label}");
                // Canonical boundaries and lazy membership do not change merge
                // keys or the printer's supported operator shapes. Type rewrites
                // preserve Physical. Cache only this rejection classification;
                // every accepted invocation still compiles its complete pipeline.
                let key = (case.name, c[0], c[1], c[3], c[4], c[8], c[9]);
                let base = baseline.entry((case.name, c[0])).or_insert_with(|| {
                    cq_workloads::pipelines::Pipeline {
                        plan: matrix::planner(c),
                        ..Default::default()
                    }
                    .physical(&case.module().unwrap())
                    .unwrap()
                });
                let reason = rejections
                    .entry(key)
                    .or_insert_with(|| {
                        let selected = matrix::pipeline(c);
                        let target = selected.rewrite_physical(base).unwrap();
                        selected
                            .rejection_physical(&target, c[4])
                            .unwrap()
                            .map(str::to_owned)
                    })
                    .clone();
                let (target, entry) = if let Some(reason) = &reason {
                    compile_fail += 1;
                    let line = negative.lines().count() + 1;
                    let needle = if reason == cq_workloads::pipelines::SMALLEST_LAYOUT_REJECTION {
                        reason.as_str()
                    } else if reason.starts_with("push printer") {
                        "own_push:"
                    } else if c[4] == 2 {
                        "leading declaration prefix"
                    } else {
                        "OrderedRead"
                    };
                    writeln!(expected, "{line}\t{name}\t{needle}").unwrap();
                    writeln!(census, "{}\t{label}\tcompile-fail\t0\t{reason}", case.name).unwrap();
                    (&mut negative, "compile_query")
                } else {
                    let scenarios = cq_workloads::execution::scenarios(case.name).len();
                    differential += scenarios;
                    writeln!(census, "{}\t{label}\trun\t{scenarios}\t", case.name).unwrap();
                    (&mut code, "instantiate")
                };
                // One line per invocation lets the UI runner attribute every
                // diagnostic to its configuration through expansion spans.
                writeln!(target,
                    "mod {name} {{ macro_rules! instantiate {{ ($($cq:tt)*) => {{ cq_workload_e2e::{entry}! {{ [{}; {:?}] $($cq)* }} }}; }} cq_workloads::queries::{}::{leaf}::with_query!(instantiate); }}",
                    v.join(" "), case.name, case.suite.as_str()).unwrap();
            }
        }
        accepted.extend(code.lines().map(str::to_owned));
        std::fs::write(out.join(format!("rejected_{planner}.rs")), negative).unwrap();
        std::fs::write(out.join(format!("rejected_{planner}.tsv")), expected).unwrap();
    }
    // Accepted invocations are dealt round-robin into SHARDS small test crates
    // (Cargo's `shard_NNN` targets, all compiled from tests/shard.rs), so each
    // rustc front end is small and Cargo can compile shards in parallel.
    let manifest = std::fs::read_to_string("Cargo.toml").unwrap();
    let declared = manifest.matches("name = \"shard_").count();
    assert_eq!(
        declared,
        matrix::SHARDS,
        "Cargo.toml must declare every shard target"
    );
    println!("cargo:rerun-if-changed=Cargo.toml");
    let mut shards = vec![String::new(); matrix::SHARDS];
    for (index, line) in accepted.iter().enumerate() {
        writeln!(shards[index % matrix::SHARDS], "{line}").unwrap();
    }
    for (index, code) in shards.iter().enumerate() {
        std::fs::write(out.join(format!("shard_{index:03}.rs")), code).unwrap();
    }
    std::fs::write(out.join("cases.tsv"), census).unwrap();
    println!(
        "cargo:rustc-env=E2E_PAIRWISE_ROWS={}",
        matrix::covering().len()
    );
    println!("cargo:rustc-env=E2E_DIFFERENTIAL_CASES={differential}");
    println!("cargo:rustc-env=E2E_COMPILE_FAIL_CASES={compile_fail}");
}
