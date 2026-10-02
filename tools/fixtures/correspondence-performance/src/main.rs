use cq_planners_plan::{Planner, cq};
use std::path::Path;
use std::time::{Duration, Instant};

fn cases() -> Vec<(String, cq::Program)> {
    let queries =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../features/workloads/src/queries");
    let mut cases: Vec<(String, cq::Program)> = Vec::new();
    for suite in ["classic", "retail"] {
        for entry in std::fs::read_dir(queries.join(suite)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "rs") || path.ends_with("mod.rs") {
                continue;
            }
            let file = syn::parse_file(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let macros: Vec<_> = file
                .items
                .iter()
                .filter_map(|item| match item {
                    syn::Item::Macro(item) => Some(item),
                    _ => None,
                })
                .collect();
            assert_eq!(macros.len(), 1, "{}", path.display());
            cases.push((
                path.file_stem().unwrap().to_str().unwrap().into(),
                syn::parse2(macros[0].mac.tokens.clone()).unwrap(),
            ));
        }
    }
    cases.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        cases.len(),
        20,
        "the full authored workload corpus is mandatory"
    );
    cases.push((
        "fuzz31".into(),
        syn::parse_str(&cq_testkit::fuzz::generate(31).cq()).unwrap(),
    ));
    cases
}

fn check(name: &str, source: &cq::Program, bounded: bool) {
    eprintln!("BEGIN {name}");
    let started = Instant::now();
    let written = cq_planners_plan::compile(source, Planner::WrittenCoarse).unwrap();
    let target = cq_planners_plan::compile(source, Planner::Hypergraph).unwrap();
    cq_lang_ra::check_rewrite(&written, &target).unwrap();
    let elapsed = started.elapsed();
    eprintln!("PASS {name}: {:.6}s", elapsed.as_secs_f64());
    if bounded {
        assert!(
            elapsed < Duration::from_secs(2),
            "{name}: {elapsed:?} exceeds the debug proof budget"
        );
    }
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    for (name, source) in cases() {
        if args.is_empty() || args.iter().any(|arg| arg == &name) {
            check(&name, &source, !args.iter().any(|arg| arg == "--unbounded"));
        }
    }
}

#[test]
fn hypergraph_full_workload_corpus_and_seed_31() {
    for (name, source) in cases() {
        check(&name, &source, true);
    }
}
