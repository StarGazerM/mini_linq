#![cfg(feature = "planner-calibration")]
//! PS-After: a completed run yields the degree facts of the relations it
//! leaves, and advice from them; nothing else is observed.
#[path = "calibration_erasure.rs"]
mod fixture;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use cq_cost::{calibration, export};
use cq_load::stats::{ColumnSpec, DistinctMode, RelationSpec};
use cq_load::stats_manifest::{DatasetRequest, RelationInput, REQUEST_SCHEMA};
use cq_load::stats_source::{SourceFormat, SourceOptions};
use cq_syntax::declared_evidence::{DeclaredEvidence, RelationBinding, ColumnBinding, EvidenceView};
use cq_evidence as ev;
use quote::ToTokens;

const BUILD: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
struct Case {
    root: PathBuf,
    evidence: ev::DatasetEvidence,
    plans: export::PlannerInput,
}
impl Drop for Case { fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); } }
fn weight(id: &str, hash: &str) -> ev::RationalFact {
    ev::Fact {
        id: id.into(), source: ev::Source::Declared, claim: ev::Claim::Equal,
        value: Some(ev::Rational { numerator: "1".into(), denominator: "1".into() }), lower: None, upper: None,
        dataset_hash: hash.into(), unit: ev::Unit::DeclaredRowWork,
        provenance: ev::Provenance { producer: "test reference policy".into(), method: "explicit h=m=1; not measured hardware cost".into(), locator: "calibration_workflow.rs".into(), run: None, artifact_hash: None },
    }
}
/// The plans of the fixture program with the exact evidence of its input files attached.
fn case(edges: &[(i32,i32)], seeds: &[(i32,)]) -> Case {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!("cq-calibration-workflow-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("Edge.csv"), edges.iter().map(|(x,y)| format!("{x},{y}\n")).collect::<String>()).unwrap();
    std::fs::write(root.join("Seed.csv"), seeds.iter().map(|(x,)| format!("{x}\n")).collect::<String>()).unwrap();
    let relation = |name: &str, columns: &[&str]| RelationInput {
        relation: RelationSpec { name: name.into(), columns: columns.iter().map(|name| ColumnSpec { name: (*name).into(), domain: ev::ScalarDomain::SignedInteger }).collect(),
            semantics: ev::RelationSemantics::Set, key_sets: Vec::new() },
        path: format!("{name}.csv"), options: SourceOptions { format: SourceFormat::Csv, delimiter: b',', headers: false, trailing_delimiter: false, batch_size: 2 },
        missing_as_empty: false,
    };
    let request = DatasetRequest { schema: REQUEST_SCHEMA.into(), name: "reach-fixture".into(), date: "2026-11-05".into(), formulation: "set reachability".into(),
        hash_mode: ev::HASH_MODE.into(), distinct: DistinctMode::Exact,
        relations: vec![relation("Edge", &["x","y"]), relation("Seed", &["x"])], metadata: Vec::new() };
    let configuration = export::Configuration { compiler_hash: BUILD.into(), target: export::Target::Cpu, invocation: "CPU default route; explicit reference row-work policy".into() };
    let unbound = export::compile_datalog(fixture::SOURCE, configuration.clone()).unwrap();
    let mut evidence = request.collect(&root).unwrap();
    evidence.targets.insert("cpu".into(), ev::TargetEvidence {
        unit: ev::Unit::DeclaredRowWork, build_per_row_h: weight("policy.h", &evidence.dataset.hash), merge_per_row_m: weight("policy.m", &evidence.dataset.hash),
        configuration_hash: unbound.identity.configuration_hash,
    });
    let mut source: cq_datalog::source::Program = syn::parse_str(fixture::SOURCE).unwrap();
    source.declared_evidence = Some(DeclaredEvidence::from_json(&evidence.canonical_json().unwrap(), request.relations.iter().map(|input| RelationBinding {
        relation: syn::parse_str(&input.relation.name).unwrap(), source: input.relation.name.clone(), view: EvidenceView::Full,
        columns: input.relation.columns.iter().map(|column| ColumnBinding { column: syn::parse_str(&column.name).unwrap(), source: column.name.clone() }).collect(),
    }).collect()).unwrap());
    let plans = export::compile_datalog(&source.to_token_stream().to_string(), configuration).unwrap();
    Case { root, evidence, plans }
}
fn run(case: &Case, edges: &[(i32,i32)], seeds: &[(i32,)]) -> calibration::Result<calibration::CalibrationResult<BTreeSet<(i32,)>>> {
    calibration::measure(&case.plans, &case.evidence, "completed-fixture", || fixture::reach(edges, seeds))
}
fn degrees(evidence: &ev::DatasetEvidence) -> BTreeMap<String, (u64, u64)> {
    evidence.degrees.iter().map(|degree| {
        assert_eq!(degree.value.source, ev::Source::Calibrated);
        let value = degree.value.value.as_ref().unwrap();
        (degree.value.id.clone(), (value.numerator.parse().unwrap(), value.denominator.parse().unwrap()))
    }).collect()
}

#[test]
fn a_completed_run_yields_the_degree_facts_of_its_relations_and_advice_from_them() {
    let edges = [(0,1),(1,2),(2,3)]; let seeds = [(0,)];
    let case = case(&edges, &seeds);
    let result = run(&case, &edges, &seeds).unwrap();
    assert_eq!(result.value, BTreeSet::from([(0,),(1,),(2,),(3,)]));
    result.evidence.check().unwrap();
    // The facts are added to the supplied evidence; nothing else of it changes.
    let mut without = result.evidence.clone();
    without.degrees.clear();
    assert_eq!(without, case.evidence);
    // Reach = {0,1,2,3} and Edge maps 0, 1 and 2 on: three targets over
    // four reached sources.
    let facts = degrees(&result.evidence);
    assert_eq!(facts["degree:joined:Reach[0]~Edge[0]>1"], (3, 4));
    assert_eq!(facts["degree:distinct:Edge[1]"], (3, 1));
    assert_eq!(facts["degree:distinct:Seed[0]"], (1, 1));
    // PS-Read: the planner plans every rule from that evidence alone.
    let planner = cq_cost::cost::variables::Planner::new(&case.plans, &result.evidence).unwrap();
    let planned: Vec<_> = (0..case.plans.holes.len()).filter_map(|hole| planner.plan(hole).unwrap()).collect();
    assert_eq!(planned.len(), case.plans.holes.iter().filter(|hole| matches!(hole.origin, export::Origin::Rule { .. })).count());
    assert!(planned.iter().any(|plan| plan.order == ["x", "y"]), "{:?}", planned.iter().map(|plan| &plan.order).collect::<Vec<_>>());
    // PS-Decide: the advice of the calibrated run is decided from them.
    assert_eq!(result.advice.evidence_hash, export::sha256(result.evidence.canonical_json().unwrap()));
    for hole in result.advice.holes.iter().filter(|hole| matches!(hole.origin, export::Origin::Rule { .. })) {
        assert_eq!(hole.basis, cq_cost::cost::Basis::Measured, "{}", hole.reason);
    }
    let output = case.root.join("output"); result.write_artifacts(&output).unwrap();
    let restored = ev::parse_json(&std::fs::read_to_string(output.join("calibration.facts.json")).unwrap()).unwrap();
    assert_eq!(restored, result.evidence);
    assert_eq!(std::fs::read_to_string(output.join("calibration.orders.dl")).unwrap().trim_end(), result.advice.advice.trim_end());
    let mut written: Vec<_> = std::fs::read_dir(&output).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect();
    written.sort();
    assert_eq!(written, ["calibration.costs.json", "calibration.facts.json", "calibration.orders.dl"]);
    assert!(result.write_artifacts(&output).is_err(), "never overwrite existing artifacts");
}

#[test]
fn an_empty_fixpoint_has_facts_of_empty_relations() {
    let case = case(&[], &[]);
    let result = run(&case, &[], &[]).unwrap();
    assert!(result.value.is_empty());
    result.evidence.check().unwrap();
    assert!(!result.evidence.degrees.is_empty());
    assert!(degrees(&result.evidence).values().all(|&(numerator, _)| numerator == 0));
}

#[test]
fn a_run_that_is_not_the_planned_one_publishes_nothing() {
    let edges = [(0,1)]; let seeds = [(0,)]; let case = case(&edges, &seeds);
    assert!(calibration::measure(&case.plans, &case.evidence, "no-program", || ()).is_err());
    assert!(calibration::measure(&case.plans, &case.evidence, "two-programs", || {
        fixture::reach(&edges, &seeds); fixture::reach(&edges, &seeds)
    }).is_err());
    assert!(calibration::measure(&case.plans, &case.evidence, " ", || fixture::reach(&edges, &seeds)).is_err(), "a run needs an identity");
    let error = calibration::try_measure::<()>(&case.plans, &case.evidence, "loader-error", || Err(export::ExportError("original loader error".into()))).err().unwrap();
    assert_eq!(error.to_string(), "original loader error");
    // The run is bound to its inputs by their rows: the evidence states one edge.
    let error = run(&case, &[(0,1),(1,2)], &seeds).err().unwrap();
    assert!(error.to_string().contains("holds 2 rows; its evidence states 1"), "{error}");
    // Outside a calibration nothing is observed.
    assert_eq!(fixture::reach(&edges, &seeds), BTreeSet::from([(0,),(1,)]));
}

/// A derived row is passed with the round that derived it, for a relation
/// whose rounds the observer needs: only its deltas are kept, and passed
/// in order. Another relation keeps nothing: its rows come without rounds.
#[test]
fn a_relation_whose_rounds_are_needed_is_passed_delta_by_delta() {
    use cq_storage::statistics::collection::{with_observer, Observer, RelationRows, ScalarValue};
    struct Rounds { wanted: bool, asked: std::sync::Mutex<Vec<usize>>, rows: std::sync::Mutex<BTreeMap<usize, Vec<(usize, i64)>>> }
    impl Observer for Rounds {
        fn needs_rounds(&self, relation: usize) -> bool {
            self.asked.lock().unwrap().push(relation);
            self.wanted
        }
        fn relation(&self, relation: usize, _rows: Option<usize>, pass: RelationRows<'_>) {
            let mut rows = Vec::new();
            pass(&mut |age, row| {
                let ScalarValue::Signed(value) = row.cell(0) else { panic!("an integer column") };
                // Rounds are synchronous here: a row sees what its own round published.
                assert_eq!(age.published, age.horizon);
                rows.push((age.published, value));
            });
            self.rows.lock().unwrap().insert(relation, rows);
        }
    }
    // 0 is the seed; 1 and 3 follow it; 2 follows 1 and 4 follows 3. 3 is
    // derived again through 2, a round later: only its first round counts.
    let edges = [(0,1),(1,2),(2,3),(0,3),(3,4)]; let seeds = [(0,)];
    let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
    for (parallel, wanted) in [(false, true), (true, true), (false, false), (true, false)] {
        let seen = std::sync::Arc::new(Rounds { wanted, asked: Default::default(), rows: Default::default() });
        let reached = pool.install(|| with_observer(seen.clone(), || if parallel { parallel_reach(&edges, &seeds) } else { fixture::reach(&edges, &seeds) }));
        assert_eq!(reached, BTreeSet::from([(0,),(1,),(2,),(3,),(4,)]));
        // Relations in declaration order: Edge, Seed, Reach. Only the
        // derived one is asked about; inputs are of round 0.
        assert_eq!(*seen.asked.lock().unwrap(), [2]);
        let seen = seen.rows.lock().unwrap();
        assert!(seen[&0].iter().all(|(round, _)| *round == 0) && seen[&0].len() == 5, "{:?}", seen[&0]);
        assert_eq!(seen[&1], [(0, 0)]);
        let rounds: BTreeMap<i64, usize> = seen[&2].iter().map(|&(round, value)| (value, round)).collect();
        assert_eq!((rounds.len(), seen[&2].len()), (5, 5), "every reached row once: {:?}", seen[&2]);
        if !wanted {
            assert!(rounds.values().all(|round| *round == 0), "an unwanted relation has no rounds: {rounds:?}");
            continue;
        }
        assert!(seen[&2].is_sorted_by_key(|(round, _)| *round), "rounds ascend along the pass: {:?}", seen[&2]);
        assert!(rounds[&0] < rounds[&1] && rounds[&1] < rounds[&2], "parallel {parallel}: {rounds:?}");
        assert!(rounds[&1] == rounds[&3] && rounds[&2] == rounds[&4], "parallel {parallel}: {rounds:?}");
    }
}

fn parallel_case(edges: &[(i32,i32)], seeds: &[(i32,)]) -> Case {
    let mut case = case(edges, seeds);
    let configuration = export::Configuration {
        compiler_hash: BUILD.into(), target: export::Target::CpuPar,
        invocation: "CPU-par sharded layouts; threshold=0, grain=1; Rayon threads=2; explicit reference row-work policy".into(),
    };
    let mut profile = case.evidence.targets.remove("cpu").unwrap();
    profile.configuration_hash = export::sha256(serde_json::to_vec(&configuration).unwrap());
    case.evidence.targets.insert("cpu-par".into(), profile);
    let mut source: cq_datalog::source::Program = syn::parse_str(&case.plans.source).unwrap();
    source.declared_evidence.as_mut().unwrap().document = std::sync::Arc::new(case.evidence.clone());
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let storage = cq_datalog_storage::compile(&incremental).unwrap();
    let storage = cq_datalog_partition::compile_storage_with(&storage, quote::quote!(layouts(sharded))).unwrap();
    let round = cq_datalog_fork::compile(&storage).unwrap();
    let parallel = cq_datalog_partition::compile_sharded_with(&round, quote::quote!(threshold=0, grain=1)).unwrap();
    let valuations = parallel.round.holes.iter().map(|hole| {
        let pipeline = hole.invocation_pipeline().unwrap();
        let options = syn::parse2(pipeline.passes[hole.plan.unwrap()].arguments.clone()).unwrap();
        cq_planning::prefix_valuation(&hole.cq, &options).unwrap()
    }).collect::<Vec<_>>();
    case.plans = export::export_datalog(&source.to_token_stream().to_string(), &parallel.round, &valuations, configuration).unwrap();
    case
}

fn parallel_reach(edges: &[(i32,i32)], seeds: &[(i32,)]) -> BTreeSet<(i32,)> {
    cq_datalog::datalog! {
        pipeline {
            storage_rewrites = [cq_datalog_parallel::shard_storage(layouts(sharded))];
            round = [cq_datalog_parallel::partition(threshold=0, grain=1)];
            print = cq_datalog_parallel::print_rounds;
        }
        struct CalibrationReach;
        relation Edge(x: i32, y: i32) = (edges);
        relation Seed(x: i32) = (seeds);
        relation Reach(x: i32);
        output Reach;
        Init: Reach(*x) :- Seed(x);
        Step: Reach(*y) :- Reach(x), Edge(x,y), where x >= 0, where x < 10;
    }.into_iter().collect()
}

#[test]
fn a_parallel_run_leaves_the_same_degree_facts() {
    let edges = [(0,1),(1,2),(2,3),(10,11)];
    let seeds = [(0,),(10,)];
    let serial_case = case(&edges, &seeds);
    let serial = run(&serial_case, &edges, &seeds).unwrap();
    let case = parallel_case(&edges, &seeds);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
    let result = pool.install(|| calibration::measure(&case.plans, &case.evidence, "parallel-fixture", || parallel_reach(&edges, &seeds))).unwrap();
    assert_eq!(result.value, BTreeSet::from([(0,),(1,),(2,),(3,),(10,)]));
    assert_eq!(result.value, serial.value);
    result.evidence.check().unwrap();
    // The relations a run leaves do not depend on how it evaluated them.
    assert_eq!(degrees(&result.evidence), degrees(&serial.evidence));
}
