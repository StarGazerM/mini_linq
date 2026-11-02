//! C1: every one-fact mutant of a compiled Physical (and Structures) record
//! must be rejected by its producing pass's checker, or still denote the
//! reference answer under the independent Physical interpreter. An accepted
//! mutant with a different answer is a checker hole.
//!
//! `authored` runs the 25 authored `cq_testkit::CASES` plus fixed records
//! (authored answers) on a fixed route set; `generated` runs generated seeds
//! `0..CQ_MUTANT_CASES` (default 120) plus one rotating seed.

use std::collections::BTreeMap;

use super::{Compiled, Outcome, Rewrites, Route, physical_outcome};
use cq_testkit::mutate::{self, Operator};
use cq_testkit::physical_semantics::TypedDatabase;
use cq_testkit::values::{TypedRows, Value};

#[derive(Default, Debug)]
struct Counts {
    rejected: usize,
    accepted_same: usize,
    accepted_unjudged: usize,
    holes: usize,
}

#[derive(Default)]
struct Tally {
    physical: BTreeMap<Operator, Counts>,
    /// Accepted pure-Structures mutants need an executable route (R2).
    structures_accepted: BTreeMap<Operator, usize>,
    structures_rejected: BTreeMap<Operator, usize>,
    holes: Vec<String>,
}

impl Tally {
    fn report(&self) {
        for (operator, counts) in &self.physical {
            println!("physical {operator:?}: {counts:?}");
        }
        println!("structures rejected: {:?}", self.structures_rejected);
        println!(
            "structures accepted (need R2): {:?}",
            self.structures_accepted
        );
        assert!(
            self.physical.values().map(|c| c.rejected).sum::<usize>() > 0,
            "no mutant was generated"
        );
        assert!(
            self.holes.is_empty(),
            "{} checker holes\n{}",
            self.holes.len(),
            self.holes.join("\n")
        );
    }

    fn judge(
        &mut self,
        label: &str,
        compiled: &Compiled,
        expected: &TypedRows,
        database: &TypedDatabase,
        parameters: &BTreeMap<String, Value>,
    ) {
        for (position, (stage, program)) in compiled.physical.iter().enumerate() {
            let accepts = |candidate: &cq_lang_physical::Program| -> bool {
                cq_lang_physical::contract::check(candidate).is_ok()
                    && if position == 0 {
                        cq_physical_lowering::check(&compiled.ix, candidate).is_ok()
                    } else {
                        cq_lang_physical::check_rewrite(
                            &compiled.physical[position - 1].1,
                            candidate,
                        )
                        .is_ok()
                    }
            };
            for mutant in mutate::physical(program) {
                let counts = self.physical.entry(mutant.operator).or_default();
                if !accepts(&mutant.program) {
                    counts.rejected += 1;
                    continue;
                }
                match physical_outcome(&mutant.program, database, parameters) {
                    Outcome::Rows(rows) if rows == *expected => counts.accepted_same += 1,
                    Outcome::Rows(rows) => {
                        counts.holes += 1;
                        self.holes.push(format!(
                            "CHECKER HOLE {label} stage={stage}: accepted {:?} mutant at `{}`\nexpected {expected:?}\nmutant   {rows:?}\nsource:  {}\nmutant:  {}\n",
                            mutant.operator,
                            mutant.site,
                            quote::ToTokens::to_token_stream(program),
                            quote::ToTokens::to_token_stream(&mutant.program)
                        ));
                    }
                    Outcome::Unsupported(_) => counts.accepted_unjudged += 1,
                }
            }
        }
        // Structures: the selection checker judges every mutant of the record
        // selected for the final Physical program.
        let physical = &compiled.physical.last().unwrap().1;
        let Ok(selected) = cq_structure_selection::compile(physical) else {
            return;
        };
        if cq_structure_selection::check(physical, &selected).is_err() {
            return;
        }
        for mutant in mutate::structures(&selected) {
            if mutant.site.starts_with("physical ") {
                continue; // judged above on the Physical record itself
            }
            let accepted = cq_structure_selection::check(physical, &mutant.program).is_ok();
            let map = if accepted {
                &mut self.structures_accepted
            } else {
                &mut self.structures_rejected
            };
            *map.entry(mutant.operator).or_default() += 1;
        }
    }
}

fn fixed_routes() -> Vec<Route> {
    use cq_order::{Heuristic, Ranking, Split};
    use cq_reduce::Reductions;
    [
        (Heuristic::Written, Split::Coarse, Reductions::None),
        (
            Heuristic::Prefix(Ranking::GreedyWidth),
            Split::Fine,
            Reductions::Masks,
        ),
    ]
    .into_iter()
    .map(|(heuristic, split, semijoins)| Route {
        order: cq_order::Options { heuristic, split },
        reduce: cq_reduce::Options { semijoins },
    })
    .collect()
}

type Rows = &'static [&'static [i32]];

/// Fixed records exercising intervals, contradictions, constants, guards,
/// two-column keys in swapped order, membership and computed lets, with
/// hand-computed answers.
const FIXED: &[(&str, &str, &[(&str, Rows)], Rows)] = &[
    (
        "interval",
        "struct Q; relation R(k: i32, v: i32); relation answer(k: i32, v: i32);
         answer(k, v) :- R(k, v), where k >= 1, where k < 3;",
        &[("R", &[&[0, 1], &[1, 2], &[2, 3], &[3, 4]])],
        &[&[1, 2], &[2, 3]],
    ),
    (
        "contradiction",
        "struct Q; relation R(x: i32, y: i32); relation answer(x: i32);
         answer(x) :- R(x, y), where x < 2, where x == 3;",
        &[("R", &[&[1, 0], &[3, 0]])],
        &[],
    ),
    (
        "constants_guard",
        "struct Q; relation R(a: i32, b: i32); relation S(b: i32, c: i32);
         relation answer(a: i32, c: i32);
         answer(a, c) :- R(a, 2), S(2, c), if (a.to_owned() != c.to_owned());",
        &[
            ("R", &[&[1, 2], &[5, 3], &[7, 2]]),
            ("S", &[&[2, 1], &[2, 9], &[3, 4]]),
        ],
        &[&[1, 9], &[7, 1], &[7, 9]],
    ),
    (
        "two_key_swapped",
        "struct Q; relation R(a: i32, b: i32); relation S(b: i32, a: i32, c: i32);
         relation answer(c: i32);
         answer(c) :- R(a, b), S(b, a, c);",
        &[
            ("R", &[&[1, 2], &[2, 1]]),
            ("S", &[&[2, 1, 10], &[1, 1, 20], &[2, 2, 30]]),
        ],
        &[&[10]],
    ),
    (
        "membership_let",
        "struct Q; relation R(x: i32); relation answer(y: i32);
         answer(y) :- R(x), where x in [1, 3], let y: i32 = (x.to_owned() * 2);",
        &[("R", &[&[1], &[2], &[3]])],
        &[&[2], &[6]],
    ),
];

fn database(inputs: &[(&str, Rows)]) -> cq_testkit::values::Database {
    inputs
        .iter()
        .map(|(name, rows)| {
            (
                (*name).to_owned(),
                rows.iter().map(|r| r.to_vec()).collect(),
            )
        })
        .collect()
}

pub fn authored_records_reject_mutants_or_keep_the_answer(rewrites: Rewrites) {
    let mut tally = Tally::default();
    let records = cq_testkit::CASES
        .iter()
        .map(|case| {
            (
                case.name,
                case.parse_cq().unwrap(),
                case.database(),
                case.expected_rows(),
            )
        })
        .chain(FIXED.iter().map(|(name, cq, inputs, expected)| {
            (
                *name,
                syn::parse_str(cq).unwrap(),
                database(inputs),
                expected.iter().map(|r| r.to_vec()).collect(),
            )
        }));
    for (name, source, database, expected) in records {
        let typed: TypedDatabase = database
            .iter()
            .map(|(name, rows)| (name.clone(), cq_testkit::values::typed(rows)))
            .collect();
        let expected = cq_testkit::values::typed(&expected);
        for (index, route) in fixed_routes().into_iter().enumerate() {
            let compiled = match super::compile_source(
                &source,
                Some(database.clone()),
                &typed,
                &BTreeMap::new(),
                &route,
                rewrites,
            ) {
                Ok(compiled) => compiled,
                // An optional route may refuse a query (e.g. Tree on a cycle);
                // the written route must compile every authored record.
                Err(_) if index > 0 => continue,
                Err(error) => panic!(
                    "case {name} route {}: {} {}",
                    route.name(),
                    error.stage,
                    error.message
                ),
            };
            for stage in &compiled.stages {
                if let Outcome::Rows(rows) = &stage.outcome {
                    assert_eq!(
                        rows,
                        &expected,
                        "case {name} route {} stage {}",
                        route.name(),
                        stage.name
                    );
                }
            }
            let label = format!("case={name} route={}", route.name());
            tally.judge(&label, &compiled, &expected, &typed, &BTreeMap::new());
        }
    }
    tally.report();
}

pub fn generated_records_reject_mutants_or_keep_the_answer(rewrites: Rewrites) {
    let routes = Route::all();
    let rotating = super::rotating_seed();
    println!("rotating seed {rotating} (replay with CQ_FUZZ_SEED={rotating})");
    let count = std::env::var("CQ_MUTANT_CASES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120u64);
    let mut tally = Tally::default();
    for seed in (0..count).chain([rotating]) {
        let case = super::generate(seed);
        let route = routes[(seed as usize) % routes.len()];
        let Ok(compiled) = super::compile(&case, &route, rewrites) else {
            continue; // pass failures belong to the stage differential
        };
        let Ok(expected) = super::quietly(|| case.reference()) else {
            continue;
        };
        let label = format!("seed={seed} route={}", route.name());
        tally.judge(
            &label,
            &compiled,
            &expected,
            &super::typed_database(&case),
            &super::parameters(&case),
        );
    }
    tally.report();
}
