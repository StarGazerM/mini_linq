//! R1: generated cases compiled pass by pass; every stage's denotation must
//! equal `Case::reference()`. The first differing stage names the broken pass.
//! Seeds `0..CQ_STAGE_CASES` (default 1000) under `CQ_FUZZ_PROFILE` (quick, or
//! full) plus one rotating seed (`CQ_FUZZ_SEED`, else the clock), printed for
//! replay.

use std::collections::BTreeMap;

use super::{Outcome, Rewrites, Route, compile, failure, reference};

/// Seeds whose failure is recorded in design-notes/test-layer-findings.md and
/// awaits a compiler fix. They are reported, not failed, by
/// `every_stage_denotes_the_reference_answer` and asserted by
/// `recorded_findings_are_fixed`.
const RECORDED_FINDINGS: &[u64] = &[37, 877, 941]; // F1 (quick profile)

#[derive(Default)]
struct Tally {
    judged: BTreeMap<String, usize>,
    unsupported: BTreeMap<String, BTreeMap<String, usize>>,
}

fn run(seed: u64, routes: &[Route], rewrites: Rewrites, tally: &mut Tally) -> Option<String> {
    let case = super::generate(seed);
    let route = routes[(seed as usize) % routes.len()];
    let compiled = match compile(&case, &route, rewrites) {
        Ok(compiled) => compiled,
        Err(error) if error.inapplicable => {
            *tally.judged.entry("inapplicable route".into()).or_default() += 1;
            return None;
        }
        Err(error) => {
            return Some(failure(&case, &route.name(), &error.stage, &error.message));
        }
    };
    for stage in &compiled.stages {
        match &stage.outcome {
            Outcome::Rows(_) => *tally.judged.entry(stage.name.clone()).or_default() += 1,
            Outcome::Unsupported(why) => {
                let why: String = why.chars().take(60).collect();
                *tally
                    .unsupported
                    .entry(stage.name.clone())
                    .or_default()
                    .entry(why)
                    .or_default() += 1;
            }
        }
    }
    let (origin, expected) = match super::quietly(|| case.reference()) {
        Ok(rows) => ("reference".to_owned(), rows),
        Err(why) => {
            *tally
                .unsupported
                .entry("reference".into())
                .or_default()
                .entry(why)
                .or_default() += 1;
            reference(&compiled.stages)?
        }
    };
    for stage in &compiled.stages {
        if let Outcome::Rows(rows) = &stage.outcome {
            if *rows != expected {
                return Some(failure(
                    &case,
                    &route.name(),
                    &stage.name,
                    &format!("{origin} = {expected:?}\n{} = {rows:?}", stage.name),
                ));
            }
        }
    }
    None
}

pub fn every_stage_denotes_the_reference_answer(rewrites: Rewrites) {
    let routes = Route::all();
    let rotating = super::rotating_seed();
    println!("rotating seed {rotating} (replay with CQ_FUZZ_SEED={rotating})");
    let count = super::case_count(1000);
    let mut failures = Vec::new();
    let mut tally = Tally::default();
    for seed in (0..count).chain([rotating]) {
        if let Some(report) = run(seed, &routes, rewrites, &mut tally) {
            if RECORDED_FINDINGS.contains(&seed) {
                println!("recorded finding (not failed):\n{report}");
            } else {
                failures.push(report);
            }
        }
    }
    println!(
        "{count} cases; judged answers per stage: {:?}",
        tally.judged
    );
    for (stage, reasons) in &tally.unsupported {
        let mut reasons: Vec<_> = reasons.iter().collect();
        reasons.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        reasons.truncate(3);
        println!("unjudged {stage}: {reasons:?}");
    }
    assert!(
        failures.is_empty(),
        "{} failures\n{}",
        failures.len(),
        failures.join("\n")
    );
}

pub fn recorded_findings_are_fixed(rewrites: Rewrites) {
    let routes = Route::all();
    let mut tally = Tally::default();
    let failures: Vec<String> = RECORDED_FINDINGS
        .iter()
        .filter_map(|seed| run(*seed, &routes, rewrites, &mut tally))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
