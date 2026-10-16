use cq_planners_plan::{Planner, cq};
use proc_macro2::{TokenStream, TokenTree};
use std::path::Path;
use std::time::{Duration, Instant};

/// The one literal `compile! { [pipeline {..}] <CQ> }` inside a case file
/// (the same reader as `cq_workloads::authored`, which this fixture cannot
/// depend on without its oracle).
fn literal_query(source: &str) -> TokenStream {
    fn visit(tokens: TokenStream, queries: &mut Vec<TokenStream>) {
        let tokens: Vec<_> = tokens.into_iter().collect();
        let mut i = 0;
        while i < tokens.len() {
            if let [TokenTree::Ident(name), TokenTree::Punct(bang), TokenTree::Group(body), ..] =
                &tokens[i..]
                && name == "compile"
                && bang.as_char() == '!'
            {
                queries.push(body.stream());
                i += 3;
                continue;
            }
            if let TokenTree::Group(group) = &tokens[i] {
                visit(group.stream(), queries);
            }
            i += 1;
        }
    }
    let mut queries = Vec::new();
    visit(source.parse().unwrap(), &mut queries);
    assert_eq!(queries.len(), 1, "exactly one literal compile! query");
    let mut tokens: Vec<_> = queries.remove(0).into_iter().collect();
    if matches!(tokens.first(), Some(TokenTree::Ident(name)) if name == "pipeline") {
        tokens.drain(..2);
    }
    tokens.into_iter().collect()
}

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
            let source = std::fs::read_to_string(&path).unwrap();
            cases.push((
                path.file_stem().unwrap().to_str().unwrap().into(),
                syn::parse2(literal_query(&source)).unwrap(),
            ));
        }
    }
    cases.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        cases.len(),
        20,
        "the full authored workload corpus is mandatory"
    );
    for seed in [31, 997] {
        cases.push((
            format!("fuzz{seed}"),
            syn::parse_str(&cq_testkit::fuzz::generate(seed).cq()).unwrap(),
        ));
    }
    cases
}

fn check(name: &str, source: &cq::Program, bounded: bool) {
    eprintln!("BEGIN {name}");
    let started = Instant::now();
    let written = cq_planners_plan::compile(source, Planner::WrittenCoarse).unwrap();
    let target = cq_planners_plan::compile(source, Planner::Hypergraph).unwrap();
    cq_lang_ra::check_rewrite(&written, &target).unwrap();
    let vo = cq_planners_plan::compile_valuation(source, Planner::Hypergraph).unwrap();
    // Hypergraph now absorbs redundant reductions after elaboration. Check
    // elaboration against its own output, then validate the simplified target.
    let elaborated = cq_elaborate::compile(&vo).unwrap();
    cq_lang_ra::check_rewrite(&elaborated, &target).unwrap();
    let elapsed = started.elapsed();
    let iterations = 100;
    let elaborate_started = Instant::now();
    for _ in 0..iterations {
        cq_elaborate::check(&vo, &elaborated).unwrap();
    }
    let elaborate = elaborate_started.elapsed().as_secs_f64() / f64::from(iterations);
    let rewrite_started = Instant::now();
    for _ in 0..iterations {
        cq_lang_ra::check_rewrite(&written, &target).unwrap();
    }
    let rewrite = rewrite_started.elapsed().as_secs_f64() / f64::from(iterations);
    eprintln!("TIMING\t{name}\t{iterations}\t{elaborate:.9}\t{rewrite:.9}");
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
        if args.iter().all(|arg| arg.starts_with("--")) || args.iter().any(|arg| arg == &name) {
            check(&name, &source, !args.iter().any(|arg| arg == "--unbounded"));
        }
    }
}

#[test]
fn hypergraph_full_workload_corpus_and_regression_seeds() {
    for (name, source) in cases() {
        check(&name, &source, true);
    }
}
