mod matrix;

use std::{fmt::Write, path::PathBuf};

// Classify only the order premise that rustc must reject. All accepted cases
// still go through the public facade macros and the original CQ callback.
fn rejected(case: &cq_workloads::QueryCase, c: matrix::Config) -> Option<String> {
    let normal = cq_lang_cq::normal::normalize(&case.module().unwrap()).unwrap();
    let plan = cq_order::compile(&normal, &matrix::planner(c[0])).unwrap();
    let plan = if c[0] == 2 {
        // These corpus queries have no guards; the reduction still follows the
        // public hypergraph route's acyclicity decision.
        let reductions = if cq_lang_cq::hypergraph::Analysis::of(&normal).acyclic {
            cq_reduce::Reductions::Tree
        } else {
            cq_reduce::Reductions::None
        };
        cq_reduce::compile(
            &plan,
            &cq_reduce::Options {
                guards: cq_reduce::Pushdown::Atoms,
                semijoins: reductions,
            },
        )
        .unwrap()
    } else {
        plan
    };
    let ra = cq_elaborate::compile(&plan).unwrap();
    let ix = cq_index_policy::compile(&ra).unwrap();
    let v = matrix::values(c);
    let policy = syn::parse_str(&format!(
        "policy {{ join = {}; project = {}; intersect = {}; }}",
        if c[1] == 0 { "seek" } else { "merge" },
        v[2],
        v[3]
    ))
    .unwrap();
    let physical = cq_physical_lowering::compile(&ix, &policy).unwrap();
    let structures = cq_structure_selection::compile(&physical, &policy).unwrap();
    let analysis = cq_lang_structures::contract::check(&structures).unwrap();
    for (result, reads) in &analysis.physical.merge_reads {
        if !analysis.host_read(result) {
            continue;
        }
        let host = &analysis.physical.hosts[result];
        let inverse: std::collections::BTreeMap<_, _> = analysis.physical.renames[result]
            .iter()
            .map(|(t, s)| (s, t))
            .collect();
        let layout: Vec<_> = analysis.host_layouts[host]
            .iter()
            .map(|c| inverse[c].clone())
            .collect();
        for (_, columns) in reads {
            // A nonleading merge read is explicitly built in key order. A
            // leading read is direct and requires exactly that native key.
            if layout[..columns.len()] == *columns && (c[4] != 2 || columns.len() != 1) {
                return Some(format!(
                    "{} cannot directly merge {host} on {columns:?}",
                    v[4]
                ));
            }
        }
    }
    None
}

fn main() {
    matrix::verify_cover();
    println!("cargo:rerun-if-changed=matrix.rs");
    println!("cargo:rerun-if-changed=../src/queries");
    println!("cargo:rerun-if-changed=tests/support/mod.rs");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let mut census = String::from("query\tconfig\tstatus\tscenarios\treason\n");
    let mut differential = 0;
    let mut compile_fail = 0;
    for planner in 0..matrix::AXES[0].len() {
        let mut code = String::new();
        let mut negative = format!(
            "#[path = {:?}] mod support;\nfn main() {{}}\n",
            PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
                .join("tests/support/mod.rs")
        );
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
                let reason = rejected(case, c);
                let (target, entry) = if let Some(reason) = &reason {
                    compile_fail += 1;
                    let line = negative.lines().count() + 1;
                    let needle = if c[4] == 2 {
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
        std::fs::write(out.join(format!("planner_{planner}.rs")), code).unwrap();
        std::fs::write(out.join(format!("rejected_{planner}.rs")), negative).unwrap();
        std::fs::write(out.join(format!("rejected_{planner}.tsv")), expected).unwrap();
    }
    std::fs::write(out.join("cases.tsv"), census).unwrap();
    println!("cargo:rustc-env=E2E_DIFFERENTIAL_CASES={differential}");
    println!("cargo:rustc-env=E2E_COMPILE_FAIL_CASES={compile_fail}");
}
