//! The order the tool selects against the optimal order. The oracle counts
//! the exact frames every candidate valuation processes over a semi-naive
//! run; it shares no code with the cost model.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicUsize, Ordering};

use cq_cost::cost::{self, estimate, Candidate, PartialReads, Space};
use cq_cost::export::{self, Configuration, Origin, PlannerInput, Target, Version};
use cq_evidence::{self as ev, Claim, DatasetEvidence, Fact, Provenance, Rational, RelationSemantics, ScalarDomain, Source, TargetEvidence, Unit};
use cq_load::{stats::{ColumnSpec, DistinctMode, RelationSpec}, stats_manifest::{DatasetRequest, RelationInput, REQUEST_SCHEMA}, stats_source::SourceOptions};

mod support;
use support::{Database, History, Program, Round, Rows, atom, rule, Var};

fn config() -> Configuration {
    Configuration { compiler_hash: export::sha256("optimal order fixture"), target: Target::Cpu, invocation: "optimal order fixture".into() }
}

/// Exact evidence of the input relations, profiled from files.
fn profiled(program: &Program, inputs: &Database) -> DatasetEvidence {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!("cq-optimal-order-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir(&root).unwrap();
    for (name, _) in &program.inputs {
        let text: String = inputs[name].iter().map(|row| row.iter().map(i64::to_string).collect::<Vec<_>>().join(",") + "\n").collect();
        std::fs::write(root.join(format!("{name}.csv")), text).unwrap();
    }
    let request = DatasetRequest {
        schema: REQUEST_SCHEMA.into(), name: "optimal-order".into(), date: "2026-11-11".into(), formulation: "cost-test".into(),
        hash_mode: ev::HASH_MODE.into(), distinct: DistinctMode::Exact, metadata: Vec::new(),
        relations: program.inputs.iter().map(|(name, arity)| RelationInput {
            relation: RelationSpec { name: (*name).into(), semantics: RelationSemantics::Set,
                columns: (0..*arity).map(|c| ColumnSpec { name: format!("c{c}"), domain: ScalarDomain::SignedInteger }).collect(), key_sets: Vec::new() },
            path: format!("{name}.csv"), options: SourceOptions::default(), missing_as_empty: false,
        }).collect(),
    };
    let mut evidence = request.collect(&root).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    let unit = |id: &str| Fact {
        id: id.into(), source: Source::Declared, claim: Claim::Equal,
        value: Some(Rational { numerator: "1".into(), denominator: "1".into() }), lower: None, upper: None,
        dataset_hash: evidence.dataset.hash.clone(), unit: Unit::DeclaredRowWork,
        provenance: Provenance { producer: "test reference policy".into(), method: "explicit h=m=1 row work".into(), locator: "optimal_order.rs".into(), run: None, artifact_hash: None },
    };
    // VP-Index: the CPU engine's 16 bindings a group.
    let group = Fact { value: Some(Rational { numerator: "16".into(), denominator: "1".into() }), ..unit("cpu.g") };
    evidence.targets.insert("cpu".into(), TargetEvidence {
        unit: Unit::DeclaredRowWork, configuration_hash: export::sha256(serde_json::to_vec(&config()).unwrap()),
        build_per_row_h: unit("cpu.h"), merge_per_row_m: unit("cpu.m"), index_per_group_g: Some(group),
    });
    evidence.check().unwrap();
    evidence
}

/// The checked compiler record of the program with its inputs bound.
fn planned(program: &Program, evidence: &DatasetEvidence) -> PlannerInput {
    let bindings: String = program.inputs.iter().map(|(name, arity)| format!("{name} = \"{name}\" full [{}];",
        (0..*arity).map(|c| format!("c{c}=\"c{c}\"")).collect::<Vec<_>>().join(","))).collect();
    let json = evidence.canonical_json().unwrap();
    let attachment = format!("declared_evidence {} {{ {bindings} }}\n", quote::quote!(#json));
    export::compile_datalog(&program.source_with(&attachment), config()).unwrap()
}

/// What the run produced, as an observation of the program on the dataset.
fn observed(program: &Program, history: &History, input: &PlannerInput, evidence: &DatasetEvidence) -> estimate::Observed {
    let relations = program.derived.iter().map(|(name, arity)| {
        let rows = &history.database[name];
        let columns = (0..*arity).map(|column| {
            let mut groups = BTreeMap::<i64, u64>::new();
            for row in rows { *groups.entry(row[column]).or_default() += 1; }
            (format!("c{column}"), estimate::ObservedColumn { distinct: Some(groups.len() as u64), max_degree: Some(groups.values().copied().max().unwrap_or(0)) })
        }).collect();
        let rounds = &history.rounds[history.component[name]];
        ((*name).to_string(), estimate::ObservedRelation { rows: rows.len() as u64, rounds: (!rounds.is_empty()).then_some(rounds.len() as u64), columns })
    }).collect();
    estimate::Observed { schema: estimate::OBSERVED_SCHEMA.into(), program_hash: input.identity.program_hash.clone(),
        dataset_hash: evidence.dataset.hash.clone(), relations }
}

/// The exact frames each step of a valuation processes on one state.
/// `delta` holds the rows of its changing read. A read that introduces is
/// charged its output frames; every other step is charged its input frames.
fn step_frames(candidate: &Candidate, full: &Database, delta: Option<&Database>, policy: PartialReads) -> Vec<u128> {
    let mut slots = BTreeMap::<&str, usize>::new();
    let mut prefix: Vec<Vec<i64>> = vec![Vec::new()];
    let mut work = Vec::new();
    for step in &candidate.steps {
        let Some(read) = &step.read else { work.push(prefix.len() as u128); continue; };
        let state = if read.version == Version::Delta { delta.expect("a changing read needs a round") } else { full };
        let rows: &Rows = state.iter().find(|(name, _)| **name == read.logical_relation).map(|(_, rows)| rows).expect("declared relation");
        // Retained columns whose variable the prefix holds, and the others.
        let held: Vec<(usize, usize)> = read.columns.iter().filter_map(|column|
            Some((column.source_column, *slots.get(column.variable.as_deref()?)?))).collect();
        let mut fresh = Vec::<(usize, &str)>::new();
        let mut repeated = Vec::<(usize, usize)>::new();
        for column in &read.columns {
            let Some(variable) = column.variable.as_deref() else { continue; };
            if slots.contains_key(variable) { continue; }
            match fresh.iter().find(|(_, known)| *known == variable) {
                Some(&(first, _)) => repeated.push((first, column.source_column)),
                None => fresh.push((column.source_column, variable)),
            }
        }
        let mut index = HashMap::<Vec<i64>, Vec<Vec<i64>>>::new();
        for row in rows.iter().filter(|row| repeated.iter().all(|&(first, again)| row[first] == row[again])) {
            index.entry(held.iter().map(|&(column, _)| row[column]).collect()).or_default()
                .push(fresh.iter().map(|&(column, _)| row[column]).collect());
        }
        if !step.introduces {
            work.push(prefix.len() as u128);
            prefix.retain(|frame| index.contains_key(&held.iter().map(|&(_, slot)| frame[slot]).collect::<Vec<_>>()));
            continue;
        }
        let mut next = Vec::new();
        for frame in &prefix {
            let Some(matches) = index.get(&held.iter().map(|&(_, slot)| frame[slot]).collect::<Vec<_>>()) else { continue; };
            // Under `distinct` a partial read yields each retained value once.
            let once: BTreeSet<&Vec<i64>>;
            let matches: Vec<&Vec<i64>> = if read.partial && policy == PartialReads::Distinct {
                once = matches.iter().collect(); once.into_iter().collect()
            } else { matches.iter().collect() };
            for values in matches { next.push(frame.iter().chain(values).copied().collect()); }
        }
        for (_, variable) in fresh { let slot = slots.len(); slots.insert(variable, slot); }
        work.push(next.len() as u128);
        prefix = next;
    }
    work
}
fn frames(candidate: &Candidate, full: &Database, delta: Option<&Database>, policy: PartialReads) -> u128 {
    step_frames(candidate, full, delta, policy).into_iter().sum()
}

/// The exact frames of a valuation over the whole run of its hole.
fn exact(origin: &Origin, candidate: &Candidate, history: &History, policy: PartialReads) -> u128 {
    let changing = candidate.steps.iter().filter_map(|step| step.read.as_ref()).find(|read| read.version == Version::Delta);
    match (origin, changing) {
        (Origin::Rule { seed: false, .. }, Some(read)) => {
            let component = history.component.iter().find(|(name, _)| **name == read.logical_relation).map(|(_, at)| *at).unwrap();
            history.rounds[component].iter().map(|Round { delta, full }| frames(candidate, full, Some(delta), policy)).sum()
        }
        _ => frames(candidate, &history.database, None, policy),
    }
}

/// The maintenance policy the compiler recorded for a demand: of the layout
/// with exactly its key, else the uniform policy of its relation and version.
fn policy<'a>(input: &'a PlannerInput, demand: &cost::IndexDemand) -> Option<&'a str> {
    let version = if demand.version == cost::IndexVersion::Full { Version::Full } else { Version::Delta };
    let layouts: Vec<_> = input.layouts.iter().filter(|layout| layout.logical_relation == demand.logical_relation && layout.version == version).collect();
    let key: BTreeSet<usize> = demand.key.iter().copied().collect();
    layouts.iter().find(|layout| layout.columns.iter().take(layout.key).copied().collect::<BTreeSet<_>>() == key)
        .map(|layout| layout.maintenance.as_str())
        .or_else(|| layouts.iter().all(|layout| layout.maintenance == layouts[0].maintenance).then(|| layouts.first().map(|layout| layout.maintenance.as_str())).flatten())
}

/// The exact rows the run writes into the indexes a valuation demands
/// beyond those already provisioned, under each index's recorded policy and
/// unit row costs (the fixture declares h = m = 1): an index kept across
/// rounds receives every row of its relation once; an index rebuilt every
/// round receives the relation as it stands in that round.
fn maintained(input: &PlannerInput, hole: &cost::HoleAdvice, candidate: &Candidate, history: &History) -> u128 {
    let steady = matches!(hole.origin, Origin::Rule { seed: false, .. });
    let rounds: &[Round] = candidate.steps.iter().filter_map(|step| step.read.as_ref()).find(|read| read.version == Version::Delta)
        .filter(|_| steady).map_or(&[], |read| &history.rounds[history.component[read.logical_relation.as_str()]]);
    let rows = |name: &str, state: &Database| state.iter().find(|(held, _)| **held == name).map_or(0, |(_, rows)| rows.len()) as u128;
    candidate.demands.difference(&hole.shared_before).map(|demand| {
        let name = demand.logical_relation.as_str();
        let recursive = history.component.get(name).is_some_and(|&at| !history.rounds[at].is_empty());
        match (policy(input, demand), demand.version, recursive) {
            (Some("replace_each_run"), cost::IndexVersion::Full, true) => rounds.iter().map(|round| rows(name, &round.full)).sum(),
            (Some("replace_each_run"), _, false) if steady => rounds.len() as u128 * rows(name, &history.database),
            (Some("admitted_rows" | "replace_each_run"), _, _) => rows(name, &history.database),
            (other, version, _) => panic!("the model charges no maintenance for {name} {version:?} under {other:?}"),
        }
    }).sum()
}

/// A certificate's value at the synthesized facts and the input rows.
fn value(cost: &cost::LinearCost, report: &cost::AdviceReport, evidence: &DatasetEvidence) -> f64 {
    let number = |value: &Rational| value.numerator.parse::<f64>().unwrap() / value.denominator.parse::<f64>().unwrap();
    let facts = &report.profile.as_ref().unwrap().facts;
    let fact = |name: &str, what: &str| [estimate::ESTIMATE, estimate::STRUCTURAL].iter()
        .find_map(|prefix| facts.get(&format!("{prefix}{name}:{what}"))).copied().unwrap_or_else(|| panic!("no fact {name}:{what}")) as f64;
    number(&cost.constant) + cost.terms.iter().map(|(term, coefficient)| number(coefficient) * match term.split(':').collect::<Vec<_>>()[..] {
        ["final_rows", "inferred", name] | ["rows", "inferred", name] => fact(name, "rows"),
        ["rounds", "inferred", name] => fact(name, "rounds"),
        ["rows", name] => evidence.relations[name].rows.as_ref().unwrap().value.unwrap() as f64,
        _ => panic!("unexpected certificate term {term}"),
    }).sum::<f64>()
}

/// A field-sensitive points-to analysis in miniature.
fn points_to() -> Program {
    Program { inputs: vec![("New", 2), ("Assign", 2), ("Store", 2), ("Load", 2)], derived: vec![("V", 2), ("F", 2)], rules: vec![
        rule(atom("V", &[Var("v"), Var("h")]), &[atom("New", &[Var("v"), Var("h")])]),
        rule(atom("V", &[Var("to"), Var("h")]), &[atom("Assign", &[Var("to"), Var("from")]), atom("V", &[Var("from"), Var("h")])]),
        rule(atom("F", &[Var("h"), Var("g")]), &[atom("Store", &[Var("base"), Var("v")]), atom("V", &[Var("base"), Var("h")]), atom("V", &[Var("v"), Var("g")])]),
        rule(atom("V", &[Var("to"), Var("g")]), &[atom("Load", &[Var("to"), Var("base")]), atom("V", &[Var("base"), Var("h")]), atom("F", &[Var("h"), Var("g")])]),
    ] }
}

struct Shape { seed: u64, variables: u64, heaps: u64, news: usize, assigns: usize, stores: usize, loads: usize }
/// Skewed pseudo-random inputs: low-numbered variables are used most, and
/// only they are allocated, so most of a large program is never reached.
fn generate(shape: &Shape) -> Database {
    let mut state = shape.seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    let mut next = move || { state ^= state << 13; state ^= state >> 7; state ^= state << 17; state };
    let (variables, heaps) = (shape.variables, shape.heaps);
    let skewed = |next: &mut dyn FnMut() -> u64| { let (a, b, c) = (next() % variables, next() % variables, next() % variables); a.min(b).min(c) as i64 };
    let uniform = |next: &mut dyn FnMut() -> u64| (next() % variables) as i64;
    let heap = |next: &mut dyn FnMut() -> u64| (next() % heaps) as i64;
    let mut pairs = |count: usize, second: &dyn Fn(&mut dyn FnMut() -> u64) -> i64| -> Rows {
        (0..count).map(|_| { let first = skewed(&mut next); vec![first, second(&mut next)] }).collect()
    };
    BTreeMap::from([
        ("New", pairs(shape.news, &heap)), ("Assign", pairs(shape.assigns, &uniform)),
        ("Store", pairs(shape.stores, &uniform)), ("Load", pairs(shape.loads, &uniform)),
    ])
}

/// One hole judged by exact work, the model's unit: join frames plus the rows
/// written into the indexes the order demands. The written order, the order
/// advised, the order the estimates rank first before any screen, and the
/// optimum over every valuation of the declared space.
struct Judged { name: String, basis: cost::Basis, incumbent: u128, selected: u128, ranked: u128, optimal: u128, sound: bool }

/// The profile of the program over example datasets, each observed from its
/// own semi-naive run.
fn profile(program: &Program, examples: &[&Database]) -> estimate::Profile {
    let mut profile = None;
    for example in examples {
        let evidence = profiled(program, example);
        let input = planned(program, &evidence);
        let history = program.semi_naive(example);
        let profile = profile.get_or_insert_with(|| estimate::Profile::new(input.identity.program_hash.clone()));
        profile.add(estimate::example(&input, &evidence, &observed(program, &history, &input, &evidence)).unwrap());
    }
    profile.unwrap()
}

/// Plan `inputs` from a profile of `examples`, and judge every hole by exact work.
fn judge(program: &Program, inputs: &Database, examples: &[&Database], space: Space) -> Vec<Judged> {
    let evidence = profiled(program, inputs);
    let input = planned(program, &evidence);
    let history = program.semi_naive(inputs);
    let profile = profile(program, examples);
    let report = cost::analyze_predicted(&input, &evidence, "cpu", space, &profile).unwrap();
    let listings = cost::listed(&input, &evidence, "cpu", space, Some(&profile)).unwrap();
    report.holes.iter().zip(&listings).map(|(hole, listing)| {
        let work = |plan: &Candidate| exact(&hole.origin, plan, &history, space.partial_reads) + maintained(&input, hole, plan, &history);
        let optimal = listing.valuations.iter().map(|valued| work(&valued.plan)).min().unwrap();
        let current = hole.candidates.iter().position(|candidate| candidate.plan.current).unwrap();
        let sound = hole.candidates.iter().all(|candidate|
            candidate.cost.whole_run.as_ref().is_none_or(|bound| value(bound, &report, &evidence) >= work(&candidate.plan) as f64));
        // The order the estimates rank first, before any screen.
        let ranked = hole.candidates.iter().enumerate().filter_map(|(at, candidate)| Some((candidate.estimate?, at)))
            .min().map_or(current, |(_, at)| at);
        let Origin::Rule { source_label, variant, seed, .. } = &hole.origin else { panic!("rule holes only") };
        Judged { name: format!("{}{}#{variant}", source_label.clone().unwrap_or_default(), if *seed { " seed" } else { "" }),
            basis: hole.basis, incumbent: work(&hole.candidates[current].plan),
            selected: work(&hole.candidates[hole.selected].plan), ranked: work(&hole.candidates[ranked].plan),
            optimal, sound }
    }).collect()
}
fn total(judged: &[Judged], pick: fn(&Judged) -> u128) -> u128 { judged.iter().map(pick).sum() }

/// The program, an example dataset and a larger dataset to plan.
fn workload() -> (Program, Database, Database) {
    (points_to(), generate(&Shape { seed: 1, variables: 250, heaps: 12, news: 25, assigns: 500, stores: 100, loads: 100 }),
        generate(&Shape { seed: 2, variables: 500, heaps: 20, news: 40, assigns: 1000, stores: 200, loads: 200 }))
}

#[test]
fn the_semi_naive_reference_reaches_the_naive_fixpoint() {
    let (program, example, _) = workload();
    let history = program.semi_naive(&example);
    assert_eq!(history.database, program.evaluate(&example));
    // The deltas of a relation partition it.
    let rounds = &history.rounds[history.component["V"]];
    assert_eq!(rounds.iter().map(|round| round.delta["V"].len()).sum::<usize>(), history.database["V"].len());
    assert!(rounds.len() > 3);
}

#[test]
fn the_search_finds_the_least_estimate_of_every_listed_valuation() {
    let (program, example, planned_inputs) = workload();
    let evidence = profiled(&program, &planned_inputs);
    let input = planned(&program, &evidence);
    let profile = profile(&program, &[&example]);
    for partial_reads in [PartialReads::Rows, PartialReads::Distinct] {
        for phased in 0..3 {
            let space = Space { partial_reads, phased };
            for (hole, listing) in cost::listed(&input, &evidence, "cpu", space, Some(&profile)).unwrap().into_iter().enumerate() {
                let least = listing.valuations.iter().filter_map(|valued| valued.estimate).fold(f64::INFINITY, f64::min);
                let (found, work) = listing.searched.expect("every hole of the fixture is estimated");
                assert!((work - least).abs() <= 1e-9 * least.max(1.0), "{space:?} hole {hole}: searched {work} against least listed {least}");
                assert!(listing.valuations.iter().any(|valued| valued.plan.steps == found.steps && valued.estimate == Some(work)),
                    "{space:?} hole {hole}: the searched valuation is not listed with its estimate");
            }
        }
    }
}

#[test]
fn certificates_bound_the_exact_frames_when_every_fact_is_exact() {
    // A profile of the planned dataset itself: every synthesized fact is
    // what the run produced, so every certificate must hold.
    let (program, _, planned) = workload();
    for partial_reads in [PartialReads::Rows, PartialReads::Distinct] {
        let judged = judge(&program, &planned, &[&planned], Space { partial_reads, phased: 1 });
        assert_eq!(judged.len(), 6);
        for hole in &judged {
            assert!(hole.sound, "{partial_reads:?} {}: a certificate is below the exact frames", hole.name);
            // Only the seed reads no derived relation.
            assert_eq!(hole.basis == cost::Basis::Predicted, !hole.name.contains("seed"), "{}", hole.name);
        }
    }
}

#[test]
fn the_order_ranked_first_is_near_the_optimal_order() {
    // The profile comes from another dataset; work is exact join frames plus
    // index maintenance. Under `rows` the order with the least certificate
    // is within 2% of the optimal order in every hole. Under `distinct` the
    // optimum binds the delta's keys, which the facts cannot rank (no
    // distinct keys per delta), so the gap is wider.
    let (program, example, planned) = workload();
    for (partial_reads, per_hole, within) in [(PartialReads::Rows, 1.02, 1.01), (PartialReads::Distinct, 1.15, 1.05)] {
        let judged = judge(&program, &planned, &[&example], Space { partial_reads, phased: 1 });
        for hole in &judged {
            assert!(hole.ranked as f64 <= per_hole * hole.optimal as f64, "{partial_reads:?} {}: ranked {} against optimal {}", hole.name, hole.ranked, hole.optimal);
        }
        let (incumbent, ranked, optimal) = (total(&judged, |hole| hole.incumbent), total(&judged, |hole| hole.ranked), total(&judged, |hole| hole.optimal));
        assert!(ranked as f64 <= within * optimal as f64, "{partial_reads:?}: ranked {ranked} against optimal {optimal}");
        assert!((ranked as f64) < 0.9 * incumbent as f64, "{partial_reads:?}: ranked {ranked} against written {incumbent}");
    }
}

#[test]
fn the_advised_orders_are_below_the_written_orders() {
    // What is advised is the order ranked first, unless it undercuts the
    // written order by less than 5%: below the written orders overall and in
    // every hole, and near the optimum.
    let (program, example, planned) = workload();
    let judged = judge(&program, &planned, &[&example], Space::default());
    let (incumbent, selected, optimal) = (total(&judged, |hole| hole.incumbent), total(&judged, |hole| hole.selected), total(&judged, |hole| hole.optimal));
    assert!(selected < incumbent, "{selected} against written {incumbent}");
    assert!(judged.iter().all(|hole| hole.selected <= hole.incumbent), "an advised order is above its written order");
    assert!(selected as f64 <= 1.01 * optimal as f64, "{selected} against optimal {optimal}");
}

#[test]
fn recursive_holes_are_unknown_without_a_profile_and_predicted_with_one() {
    let (program, _, planned_inputs) = workload();
    let evidence = profiled(&program, &planned_inputs);
    let input = planned(&program, &evidence);
    let steady = |report: &cost::AdviceReport| -> Vec<(cost::Decision, cost::Basis)> {
        report.holes.iter().filter(|hole| matches!(hole.origin, Origin::Rule { seed: false, .. })).map(|hole| (hole.decision, hole.basis)).collect()
    };
    let bare = cost::analyze(&input, &evidence, "cpu").unwrap();
    assert!(bare.profile.is_none());
    assert_eq!(steady(&bare), vec![(cost::Decision::Unknown, cost::Basis::Certified); 5]);
    let own = profile(&program, &[&planned_inputs]);
    let predicted = cost::analyze_predicted(&input, &evidence, "cpu", Space::default(), &own).unwrap();
    assert!(steady(&predicted).iter().all(|(decision, basis)| *decision != cost::Decision::Unknown && *basis == cost::Basis::Predicted));
    // With the dataset's own run as the only example, estimates are the run.
    let history = program.semi_naive(&planned_inputs);
    let used = predicted.profile.as_ref().unwrap();
    assert_eq!(used.facts["estimate:V:rows"], history.database["V"].len() as u64);
    assert_eq!(used.facts["estimate:F:rows"], history.database["F"].len() as u64);
    // Rounds belong to the component, named by its first declared relation.
    assert_eq!(used.facts["estimate:V:rounds"], history.rounds[history.component["V"]].len() as u64);
    assert_eq!(used.examples, [evidence.dataset.hash.clone()]);
    // The report round-trips, and a report without a profile omits the fields.
    let reread: cost::AdviceReport = serde_json::from_str(&serde_json::to_string(&predicted).unwrap()).unwrap();
    assert_eq!(reread, predicted);
    let text = serde_json::to_string(&bare).unwrap();
    assert!(!text.contains("\"profile\"") && !text.contains("\"basis\""));
}

#[test]
fn an_estimate_is_the_bound_times_the_median_observed_ratio() {
    let (program, example, planned_inputs) = workload();
    let third = generate(&Shape { seed: 3, variables: 300, heaps: 15, news: 30, assigns: 600, stores: 120, loads: 120 });
    let evidence = profiled(&program, &planned_inputs);
    let input = planned(&program, &evidence);
    let examples = profile(&program, &[&example, &third]);
    assert_eq!(examples.examples.len(), 2);
    let report = cost::analyze_predicted(&input, &evidence, "cpu", Space::default(), &examples).unwrap();
    let bound: u64 = cost::inferred::analyze(&input, &evidence).unwrap().relations.iter()
        .find(|relation| relation.relation == "V").unwrap().rows.as_ref().unwrap().parse().unwrap();
    // Two examples: the median is the mean of their two ratios.
    let ratios: Vec<f64> = examples.examples.iter().map(|example| {
        let pair = &example.relations["V"].rows;
        pair.observed as f64 / pair.bound.parse::<f64>().unwrap()
    }).collect();
    let expected = (bound as f64 * (ratios[0] + ratios[1]) / 2.0).ceil();
    let estimate = report.profile.as_ref().unwrap().facts["estimate:V:rows"];
    assert!((estimate as f64 - expected).abs() <= 1.0, "{estimate} against {expected}");
    assert!(estimate <= bound);
    // A relation the profile never observed keeps its certified bound.
    let mut partial = examples.clone();
    for example in &mut partial.examples { example.relations.remove("F"); }
    let report = cost::analyze_predicted(&input, &evidence, "cpu", Space::default(), &partial).unwrap();
    let facts = &report.profile.as_ref().unwrap().facts;
    assert!(facts.contains_key("structural:F:rows") && !facts.contains_key("estimate:F:rows"));
}

#[test]
fn an_observation_of_another_run_is_rejected() {
    let (program, example, planned_inputs) = workload();
    let evidence = profiled(&program, &example);
    let input = planned(&program, &evidence);
    let history = program.semi_naive(&example);
    let seen = observed(&program, &history, &input, &evidence);
    estimate::example(&input, &evidence, &seen).unwrap();
    // More rows than any database with these inputs can derive.
    let mut inflated = seen.clone();
    inflated.relations.get_mut("V").unwrap().rows = 1 << 40;
    assert!(estimate::example(&input, &evidence, &inflated).unwrap_err().0.contains("exceeds its certified bound"));
    let mut elsewhere = seen.clone();
    elsewhere.dataset_hash = profiled(&program, &planned_inputs).dataset.hash;
    assert!(estimate::example(&input, &evidence, &elsewhere).unwrap_err().0.contains("another dataset"));
    let mut other = seen.clone();
    other.program_hash = export::sha256("another program");
    assert!(estimate::example(&input, &evidence, &other).unwrap_err().0.contains("another program"));
    // A profile of one program does not plan another.
    let mut foreign = estimate::Profile::new(export::sha256("another program"));
    foreign.add(estimate::example(&input, &evidence, &seen).unwrap());
    assert!(cost::analyze_predicted(&input, &evidence, "cpu", Space::default(), &foreign).is_err());
}

/// The evidence with the degree facts the variable planner requests,
/// collected from a run's relations and the round each row appeared in
/// (PLANNING-REDESIGN, PS-Collect).
fn with_degrees(input: &PlannerInput, mut evidence: DatasetEvidence, history: &History) -> DatasetEvidence {
    let requests = cost::variables::Planner::new(input, &evidence).unwrap().requests().unwrap();
    let mut collector = cq_load::stats::DegreeCollector::new(requests);
    for (name, rows) in &history.database {
        let Some(relation) = collector.relation(name) else { continue };
        // The round a derived row was in a delta; 0 for any other row.
        let round: HashMap<&Vec<i64>, u32> = history.component.get(name).map_or(HashMap::new(), |component| history.rounds[*component].iter().enumerate()
            .flat_map(|(round, state)| state.delta[name].iter().map(move |row| (row, round as u32))).collect());
        for row in rows { collector.push(relation, round.get(row).copied().unwrap_or(0), row); }
    }
    let provenance = Provenance { producer: "test reference run".into(), method: "exact degrees of the semi-naive reference run".into(), locator: "optimal_order.rs".into(), run: None, artifact_hash: None };
    evidence.degrees = cq_load::stats::bind_degrees(collector.finish(4).unwrap(), &evidence.dataset.hash, Source::Exact, Claim::Equal, provenance);
    evidence.check().unwrap();
    evidence
}

/// One rule hole planned by variable order: what identifies the variant
/// whatever the written body order, the plan, and its exact frames. With
/// `reference`, also the least and the most exact frames over the atom
/// orders that read the changing occurrence first (VP-Delta; every order of
/// a seed).
struct Variant { rule: String, changing: Option<String>, planned: cost::variables::Planned, frames: u128, atom_orders: Option<(u128, u128)> }
fn variable_plans(program: &Program, inputs: &Database, history: &History, reference: bool) -> Vec<Variant> {
    let evidence = profiled(program, inputs);
    let input = planned(program, &evidence);
    let atoms = Space { partial_reads: PartialReads::Rows, phased: 0 };
    let listings = reference.then(|| cost::listed(&input, &evidence, "cpu", atoms, Some(&profile(program, &[inputs]))).unwrap());
    let evidence = with_degrees(&input, evidence, history);
    let planner = cost::variables::Planner::new(&input, &evidence).unwrap();
    input.holes.iter().enumerate().map(|(at, hole)| {
        let Origin::Rule { source_label, seed, .. } = &hole.origin else { panic!("rule holes only") };
        let first = |plan: &Candidate| plan.steps.iter().find_map(|step| step.read.clone()).unwrap();
        let planned = planner.plan(at).unwrap().expect("every hole of the fixture plans");
        let delta = Some(first(&planned.candidate)).filter(|read| read.version == Version::Delta && !seed);
        let atom_orders = listings.as_ref().map(|listings| {
            let work: Vec<u128> = listings[at].valuations.iter().filter(|valued| delta.as_ref().is_none_or(|delta| first(&valued.plan) == *delta))
                .map(|valued| exact(&hole.origin, &valued.plan, history, atoms.partial_reads)).collect();
            (*work.iter().min().unwrap(), *work.iter().max().unwrap())
        });
        Variant { rule: source_label.clone().unwrap(), frames: exact(&hole.origin, &planned.candidate, history, PartialReads::Distinct), atom_orders,
            changing: delta.map(|read| format!("{}({})", read.logical_relation, read.columns.iter().map(|column| (column.source_column, column.variable.clone().unwrap()))
                .collect::<BTreeMap<_, _>>().into_values().collect::<Vec<_>>().join(","))),
            planned }
    }).collect()
}

/// `points_to` with a type filter on loads, as DOOP's array loads have: the
/// variable loaded into has a type the heap's type is compatible with.
fn typed_points_to() -> Program {
    let mut program = points_to();
    program.inputs.extend([("Kind", 2), ("Fits", 2)]);
    program.rules[3].1.extend([atom("Kind", &[Var("to"), Var("t")]), atom("Fits", &[Var("h"), Var("t")])]);
    program
}
/// Most variables share kind 0 and every heap fits it; the other kinds
/// have one variable each. By the average, a kind has few variables; by
/// the kinds a heap reaches, nearly all of them.
fn typed(shape: &Shape) -> Database {
    let mut inputs = generate(shape);
    let lone = |variable: u64| variable % 5 == 0;
    inputs.insert("Kind", (0..shape.variables).map(|variable| vec![variable as i64, if lone(variable) { 1 + variable as i64 } else { 0 }]).collect());
    inputs.insert("Fits", (0..shape.heaps).flat_map(|heap| [vec![heap as i64, 0], vec![heap as i64, 1 + 5 * heap as i64]]).collect());
    inputs
}

#[test]
fn the_variable_order_is_near_the_best_atom_order() {
    // Exact join frames, no index maintenance.
    let shape = Shape { seed: 3, variables: 300, heaps: 15, news: 30, assigns: 600, stores: 120, loads: 120 };
    for (program, inputs) in [(points_to(), generate(&shape)), (typed_points_to(), typed(&shape))] {
        let variants = variable_plans(&program, &inputs, &program.semi_naive(&inputs), true);
        for variant in &variants {
            let (least, _) = variant.atom_orders.unwrap();
            assert!(variant.frames as f64 <= 1.25 * least as f64, "{} {:?} {:?}: {} frames against the best atom order's {least}",
                variant.rule, variant.changing, variant.planned.order, variant.frames);
        }
        let (frames, least): (u128, u128) = (variants.iter().map(|variant| variant.frames).sum(), variants.iter().map(|variant| variant.atom_orders.unwrap().0).sum());
        assert!(frames as f64 <= 1.05 * least as f64, "{frames} frames against the best atom orders' {least}");
        // The typed load has an order to avoid: from a new field row, the
        // kinds its heap fits reach nearly every variable.
        if let Some(load) = variants.iter().find(|variant| variant.rule == "Rule3" && variant.changing.as_deref() == Some("F(h,g)")).filter(|_| program.inputs.len() == 6) {
            assert!(load.atom_orders.unwrap().1 >= 10 * load.frames, "the worst atom order takes {} frames, the plan {}", load.atom_orders.unwrap().1, load.frames);
        }
    }
}

#[test]
fn a_steady_variant_reads_its_changing_occurrence_in_full_first() {
    let (program, _, inputs) = workload();
    let variants = variable_plans(&program, &inputs, &program.semi_naive(&inputs), false);
    assert_eq!(variants.iter().filter(|variant| variant.changing.is_some()).count(), 5);
    for variant in variants.iter().filter(|variant| variant.changing.is_some()) {
        let first = variant.planned.candidate.steps.iter().find_map(|step| step.read.as_ref()).unwrap();
        assert!(first.version == Version::Delta && !first.partial, "{} {:?} starts at {first:?}", variant.rule, variant.changing);
        let held = first.columns.iter().filter_map(|column| column.variable.as_ref()).collect::<BTreeSet<_>>();
        assert!(variant.planned.order[..held.len()].iter().all(|variable| held.contains(variable)), "{:?} binds another variable before its delta's", variant.planned.order);
    }
}

#[test]
fn any_written_body_order_plans_the_same_variable_order() {
    let shape = Shape { seed: 3, variables: 300, heaps: 15, news: 30, assigns: 600, stores: 120, loads: 120 };
    let inputs = typed(&shape);
    // A body's order does not change what the rules derive: one run serves
    // every permutation (the reference evaluator joins in body order).
    let history = typed_points_to().semi_naive(&inputs);
    // The plan of each variant: the order, and every read of its valuation.
    // The guards a variant carries are the compiler's and follow the written
    // order (of two changing occurrences, which excludes the other), so its
    // frames may differ between permutations; its order and reads may not.
    type Plans = BTreeMap<(String, Option<String>), (Vec<String>, Vec<String>)>;
    let plans = |program: &Program| -> Plans {
        variable_plans(program, &inputs, &history, false).into_iter().map(|variant| {
            let reads = variant.planned.candidate.steps.iter().filter_map(|step| Some((step, step.read.as_ref()?))).map(|(step, read)| format!("{} {:?} {} {} {:?}",
                read.logical_relation, read.version, read.partial, step.introduces, read.columns.iter().map(|column| (column.source_column, column.variable.clone())).collect::<BTreeSet<_>>())).collect();
            ((variant.rule, variant.changing), (variant.planned.order, reads))
        }).collect()
    };
    let written = plans(&typed_points_to());
    assert_eq!(written.len(), 6);
    let mut reversed = typed_points_to();
    for rule in &mut reversed.rules { rule.1.reverse(); }
    let mut rotated = typed_points_to();
    for rule in &mut rotated.rules { rule.1.rotate_left(1); }
    for (name, program) in [("reversed", reversed), ("rotated", rotated)] {
        assert_eq!(plans(&program), written, "{name} bodies plan differently");
    }
}

/// PS-Decide: with the degree facts in the evidence, `analyze` advises the
/// planned variable order of every variant, whatever the written order.
#[test]
fn advice_from_measured_degrees_does_not_depend_on_the_written_body_order() {
    let shape = Shape { seed: 3, variables: 300, heaps: 15, news: 30, assigns: 600, stores: 120, loads: 120 };
    let inputs = typed(&shape);
    let history = typed_points_to().semi_naive(&inputs);
    // Per variant (its rule and changing read): the reads advised, in order.
    type Advised = BTreeMap<(String, Option<String>), Vec<String>>;
    let advised = |program: &Program| -> (Advised, usize) {
        let evidence = profiled(program, &inputs);
        let evidence = with_degrees(&planned(program, &evidence), evidence, &history);
        let input = planned(program, &evidence);
        let report = cost::analyze(&input, &evidence, "cpu").unwrap();
        let reads = report.holes.iter().map(|hole| {
            let Origin::Rule { source_label, seed, .. } = &hole.origin else { panic!("rule holes only") };
            assert_eq!(hole.basis, cost::Basis::Measured, "{}", hole.reason);
            assert_ne!(hole.decision, cost::Decision::Unknown, "{}", hole.reason);
            let reads: Vec<_> = hole.candidates[hole.selected].plan.steps.iter().filter_map(|step| step.read.as_ref()).collect();
            let text = |read: &cost::Read| format!("{} {:?} {:?}", read.logical_relation, read.version,
                read.columns.iter().map(|column| (column.source_column, column.variable.clone())).collect::<BTreeSet<_>>());
            let changing = reads.first().filter(|read| read.version == Version::Delta && !seed).map(|read| text(read));
            ((source_label.clone().unwrap(), changing), reads.iter().map(|read| text(read)).collect())
        }).collect();
        (reads, report.holes.iter().filter(|hole| hole.decision == cost::Decision::Changed).count())
    };
    let (written, _) = advised(&typed_points_to());
    assert_eq!(written.len(), 6);
    let mut reversed = typed_points_to();
    for rule in &mut reversed.rules { rule.1.reverse(); }
    let (reads, changed) = advised(&reversed);
    assert_eq!(reads, written, "reversed bodies are advised differently");
    assert!(changed > 0, "no reversed body was reordered");
}

/// VP-Index: a steady variant whose only order reads by an index of more
/// groups than its work is worth keeps a written order that avoids the
/// index; it is advised where the index has few groups, and where the
/// written order needs the index too.
#[test]
fn a_written_order_that_avoids_a_costly_index_is_kept() {
    let body = [atom("Cast", &[Var("from"), Var("to"), Var("h")]), atom("V", &[Var("from"), Var("h")])];
    let program = |body: &[support::Atom]| Program { inputs: vec![("New", 2), ("Cast", 3)], derived: vec![("V", 2)], rules: vec![
        rule(atom("V", &[Var("v"), Var("h")]), &[atom("New", &[Var("v"), Var("h")])]),
        rule(atom("V", &[Var("to"), Var("h")]), body),
    ] };
    // The one steady variant's decision.
    let decide = |program: &Program, inputs: &Database| -> cost::variables::Decided {
        let history = program.semi_naive(inputs);
        let evidence = profiled(program, inputs);
        let input = planned(program, &evidence);
        let evidence = with_degrees(&input, evidence, &history);
        let decided = cost::variables::Planner::new(&input, &evidence).unwrap().decide("cpu").unwrap();
        let mut steady = input.holes.iter().zip(decided).filter_map(|(hole, decided)| match &hole.origin {
            Origin::Rule { seed, .. } => { let decided = decided.expect("every hole of the fixture plans"); if *seed { assert_eq!(decided.kept, None); None } else { Some(decided) } }
            _ => None,
        });
        let only = steady.next().expect("one steady variant");
        assert!(steady.next().is_none());
        only
    };
    // `news` allocated variables over five heaps; a cast from each of
    // `sources` variables at each heap: `Cast` by (from, h) has a group a row.
    let casts = |news: i64, sources: i64| -> Database { BTreeMap::from([
        ("New", (0..news).flat_map(|v| (0..5).map(move |h| vec![v, h])).collect::<Rows>()),
        ("Cast", (0..sources).flat_map(|from| (0..5).map(move |h| vec![from, (from * 7 + h) % sources.max(news), h])).collect::<Rows>()),
    ]) };
    // 4,000 groups against at most 4,000 changing rows of about two
    // bindings each; the written order scans `Cast` and probes the delta.
    let costly = decide(&program(&body), &casts(20, 800));
    assert_eq!(costly.added, 16.0 * 4000.0);
    assert_eq!(costly.kept, Some(cost::variables::Kept::Index), "work {:?} against {} for the index", costly.work, costly.added);
    // Written delta first, the text reads `Cast` by the same index.
    let delta_first = decide(&program(&[body[1].clone(), body[0].clone()]), &casts(20, 800));
    assert_eq!(delta_first.added, 16.0 * 4000.0);
    assert_eq!((delta_first.kept, delta_first.written), (None, None));
    // A chain of 40 casts, each variable also cast to 20 that go nowhere:
    // 200 groups, and the written order scans all 4,200 casts in each of
    // 41 rounds.
    let chain: Database = BTreeMap::from([
        ("New", (0..5).map(|h| vec![0, h]).collect::<Rows>()),
        ("Cast", (0..40i64).flat_map(|from| (0..5).flat_map(move |h| std::iter::once(vec![from, from + 1, h])
            .chain((0..20).map(move |junk| vec![from, 1000 + from * 20 + junk, h])))).collect::<Rows>()),
    ]);
    let cheap = decide(&program(&body), &chain);
    assert_eq!(cheap.added, 16.0 * 200.0);
    assert_eq!(cheap.kept, None, "work {:?}, index {}, written {:?}", cheap.work, cheap.added, cheap.written);
    assert!(cheap.written.unwrap() > cheap.work.unwrap() + cheap.added);
}

/// VP-Written: a changing row that meets a hub, where the written order
/// reaches the delta from a few rows, keeps the written order.
#[test]
fn a_written_order_that_reads_the_delta_later_for_fewer_bindings_is_kept() {
    // D(x, z) :- Sel(x, y), Big(y, w), D(w, z): every changing row (w, z)
    // meets the 400 `y` of its hub `w`, of which `Sel` holds three.
    let program = Program { inputs: vec![("Base", 2), ("Sel", 2), ("Big", 2)], derived: vec![("D", 2)], rules: vec![
        rule(atom("D", &[Var("w"), Var("z")]), &[atom("Base", &[Var("w"), Var("z")])]),
        rule(atom("D", &[Var("x"), Var("z")]), &[atom("Sel", &[Var("x"), Var("y")]), atom("Big", &[Var("y"), Var("w")]), atom("D", &[Var("w"), Var("z")])]),
    ] };
    let inputs: Database = BTreeMap::from([
        ("Base", (0..5i64).flat_map(|w| (0..50).map(move |z| vec![w, z])).collect::<Rows>()),
        ("Sel", (0..3i64).map(|at| vec![100 + at, at]).collect::<Rows>()),
        ("Big", (0..2000i64).map(|y| vec![y, y % 5]).collect::<Rows>()),
    ]);
    let history = program.semi_naive(&inputs);
    let evidence = profiled(&program, &inputs);
    let input = planned(&program, &evidence);
    let evidence = with_degrees(&input, evidence, &history);
    let decided = cost::variables::Planner::new(&input, &evidence).unwrap().decide("cpu").unwrap();
    let steady: Vec<_> = input.holes.iter().zip(decided).filter(|(hole, _)| matches!(&hole.origin, Origin::Rule { seed: false, .. }))
        .map(|(_, decided)| decided.expect("every hole of the fixture plans")).collect();
    assert_eq!(steady.len(), 1);
    let (work, written) = (steady[0].work.unwrap(), steady[0].written.expect("the written order reads the delta last"));
    // 400 changing rows of about 400 bindings each, against a few rows a round.
    assert!(work > 100_000.0 && written < 2_000.0, "work {work}, written {written}");
    assert_eq!(steady[0].kept, Some(cost::variables::Kept::Bindings));
}
