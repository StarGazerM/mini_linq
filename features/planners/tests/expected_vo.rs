//! Rule-derived CQ/VO fixture pairs are checked independently, then compared
//! structurally with their producer. Producer inputs are original CQ, never the
//! normalized CQ recorded beside expected VO. `pin_plans` must not regenerate
//! `.vo.expected` files.
//! TPC-H pairs receive the same checked SF1 manifest attachment on their CQ
//! sources; expected VO contains only the independently authored schedule.
use std::fmt::Write;
use std::path::{Path, PathBuf};

use cq_planners::{Planner, check_valuation, compile_valuation, cq, vo};
use cq_planning::{PlanOptions, prefix_valuation};
use quote::ToTokens;
use syn::spanned::Spanned;

fn expected_files(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            expected_files(&path, files);
        } else if path.file_name().unwrap().to_str().unwrap().ends_with(".vo.expected") {
            files.push(path);
        }
    }
}

fn source_file(path: &Path) -> cq::Program {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    syn::parse_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn planner_input(path: &Path, stem: &str) -> (cq::Program, Planner) {
    let authored = path.with_file_name(format!("{stem}.cq"));
    if authored.exists() {
        assert!(stem.starts_with("le_"), "unknown authored route: {stem}");
        return (source_file(&authored), Planner::Le);
    }
    let (planner, scenario) = stem.rsplit_once('_').unwrap();
    let planner = Planner::ALL.into_iter().find(|p| p.name() == planner)
        .unwrap_or_else(|| panic!("unknown planner in {}", path.display()));
    let scenario = match scenario {
        "chain" => "oriented_chain",
        "triangle" => "triangle",
        _ => panic!("unknown scenario in {}", path.display()),
    };
    let source = cq_testkit::SCENARIOS.iter().find(|case| case.name == scenario)
        .unwrap().parse_cq().unwrap();
    (source, planner)
}

fn erase_reductions(steps: &mut Vec<vo::Step>) {
    steps.retain_mut(|step| match step {
        vo::Step::Reduce(..) => false,
        vo::Step::Block(block, _) => {
            for branch in &mut block.branches {
                erase_reductions(&mut branch.steps);
            }
            true
        }
        _ => true,
    });
}

// The default route is Plan-Prefix with optional O-EvidenceLe and R-Masks.
// Check those obligations on the authored target without calling its producer.
fn check_default(source: &cq::Program, expected: &vo::Program) -> syn::Result<()> {
    cq::contract::check(source)?;
    let normal = cq::normal::normalize(source)?;
    let mut ordered = expected.clone();
    erase_reductions(&mut ordered.valuation.steps);
    let ordering = cq_order::Options {
        heuristic: cq_order::Heuristic::Prefix(cq_order::Ranking::Connected),
        split: cq_order::Split::Coarse,
    };
    if normal.declared_evidence.is_some() {
        cq_order::evidence::check_evidence(&normal, &ordered, &ordering)?;
    } else {
        cq_order::check(&normal, &ordered, &ordering)?;
    }
    cq_reduce::check(&normal, &ordered, expected, &cq_reduce::Options {
        semijoins: cq_reduce::Reductions::Masks,
    })
}

fn rule_notes<'a>(lines: &'a [&str], step: &vo::Step) -> &'a [&'a str] {
    let end = step.span().start().line.saturating_sub(1).min(lines.len());
    let mut start = end;
    while start > 0 && lines[start - 1].trim_start().starts_with("//") {
        start -= 1;
    }
    &lines[start..end]
}

fn step_diff(
    output: &mut String,
    scope: &str,
    expected: &[vo::Step],
    actual: &[vo::Step],
    lines: &[&str],
) {
    for index in 0..expected.len().max(actual.len()) {
        let left = expected.get(index);
        let right = actual.get(index);
        let marker = if left == right { "=" } else { "!" };
        writeln!(output, "{marker} {scope}/step{index}").unwrap();
        if let Some(step) = left {
            for note in rule_notes(lines, step) {
                writeln!(output, "  {}", note.trim()).unwrap();
            }
            writeln!(output, "  expected: {}", step.to_token_stream()).unwrap();
        } else {
            writeln!(output, "  expected: <missing>").unwrap();
        }
        if left != right {
            if let Some(step) = right {
                writeln!(output, "  actual:   {}", step.to_token_stream()).unwrap();
            } else {
                writeln!(output, "  actual:   <missing>").unwrap();
            }
        }
        if let (Some(vo::Step::Block(left, _)), Some(vo::Step::Block(right, _))) = (left, right) {
            for branch in 0..left.branches.len().max(right.branches.len()) {
                let left = left.branches.get(branch).map_or(&[][..], |b| b.steps.as_slice());
                let right = right.branches.get(branch).map_or(&[][..], |b| b.steps.as_slice());
                step_diff(output, &format!("{scope}/step{index}/branch{branch}"), left, right, lines);
            }
        }
    }
}

fn structural_diff(expected: &vo::Program, actual: &vo::Program, text: &str) -> String {
    let mut output = String::new();
    if expected.valuation.ordering != actual.valuation.ordering {
        writeln!(output, "Ordering inputs differ:\n  expected: {:?}\n  actual:   {:?}",
            expected.valuation.ordering, actual.valuation.ordering).unwrap();
    }
    step_diff(&mut output, "valuation", &expected.valuation.steps, &actual.valuation.steps,
        &text.lines().collect::<Vec<_>>());
    output
}

#[test]
fn rule_derived_expected_vos_match_their_source_and_route() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut files = Vec::new();
    expected_files(&root, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no rule-derived VO fixtures found");
    let mut failures = Vec::new();
    for path in files {
        let result = (|| -> syn::Result<()> {
            let text = std::fs::read_to_string(&path).unwrap();
            let tpch = path.parent().unwrap() == root.join("tpch_default");
            let (expected_cq, expected) = syn::parse::Parser::parse_str(
                |input: syn::parse::ParseStream| {
                    let mut cq: cq::Program = input.parse()?;
                    if tpch {
                        cq.declared_evidence =
                            cq_workload_authored::tpch_evidence::load_sf1(&cq.declarations)?;
                    }
                    let vo = vo::Program::parse(input, &cq)?;
                    Ok((cq, vo))
                }, &text,
            )?;
            // The fixture records CQ input and expected VO as separate programs.
            // WF-VO includes VO-Order I1-I3, including nested branch scopes.
            vo::contract::check(&expected_cq, &expected)?;
            let expected_cq = cq::normal::normalize(&expected_cq)?;
            let stem = path.file_name().unwrap().to_str().unwrap()
                .strip_suffix(".vo.expected").unwrap();
            let (normal, actual) = if tpch {
                let mut source = source_file(&path.with_file_name(format!("{stem}.cq")));
                source.declared_evidence =
                    cq_workload_authored::tpch_evidence::load_sf1(&source.declarations)?;
                assert_eq!(source.declared_evidence, expected_cq.declared_evidence,
                    "{}: source evidence differs", path.display());
                check_default(&source, &expected)?;
                prefix_valuation(&source, &PlanOptions::default())?
            } else {
                let (source, planner) = planner_input(&path, stem);
                check_valuation(&source, &expected, planner)?;
                compile_valuation(&source, planner)?
            };
            vo::contract::check(&normal, &actual)?;
            assert_eq!(normal, expected_cq, "{}: normalized CQ differs", path.display());
            if actual != expected {
                failures.push(format!("{}:\n{}", path.display(),
                    structural_diff(&expected, &actual, &text)));
            }
            Ok(())
        })();
        if let Err(error) = result {
            failures.push(format!("{}: {error}", path.display()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
