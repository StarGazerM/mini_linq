#![cfg(feature = "planner-calibration")]
#[path = "calibration_erasure.rs"]
mod fixture;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use cq_cost::{calibration, export};
use cq_load::stats::{ColumnSpec, DistinctMode, RelationSpec};
use cq_load::stats_manifest::{DatasetRequest, MetadataInput, RelationInput, REQUEST_SCHEMA};
use cq_load::stats_source::{SourceFormat, SourceOptions};
use cq_syntax::{declared_evidence::{DeclaredEvidence, RelationBinding, ColumnBinding, EvidenceView}, evidence as ev};
use quote::ToTokens;

const BUILD: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
struct Case {
    root: PathBuf,
    request: DatasetRequest,
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
fn case(edges: &[(i32,i32)], seeds: &[(i32,)]) -> Case {
    case_for(fixture::SOURCE, edges, seeds)
}
fn case_for(source_text: &str, edges: &[(i32,i32)], seeds: &[(i32,)]) -> Case {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!("cq-calibration-workflow-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("Edge.csv"), edges.iter().map(|(x,y)| format!("{x},{y}\n")).collect::<String>()).unwrap();
    std::fs::write(root.join("Seed.csv"), seeds.iter().map(|(x,)| format!("{x}\n")).collect::<String>()).unwrap();
    std::fs::write(root.join("meta.json"), b"{ \"fixture\": \"canonical-stream\" }\n").unwrap();
    let relation = |name: &str, columns: &[&str]| RelationInput {
        relation: RelationSpec { name: name.into(), columns: columns.iter().map(|name| ColumnSpec { name: (*name).into(), domain: ev::ScalarDomain::SignedInteger }).collect(),
            semantics: ev::RelationSemantics::Set, key_sets: Vec::new() },
        path: format!("{name}.csv"), options: SourceOptions { format: SourceFormat::Csv, delimiter: b',', headers: false, trailing_delimiter: false, batch_size: 2 },
        missing_as_empty: false,
    };
    let request = DatasetRequest { schema: REQUEST_SCHEMA.into(), name: "reach-fixture".into(), date: "2026-11-05".into(), formulation: "set reachability".into(),
        hash_mode: ev::HASH_MODE.into(), distinct: DistinctMode::Exact,
        relations: vec![relation("Edge", &["x","y"]), relation("Seed", &["x"])], metadata: vec![MetadataInput { path: "meta.json".into(), role: ev::FileRole::Metadata }] };
    let configuration = export::Configuration { compiler_hash: BUILD.into(), target: export::Target::Cpu, invocation: "CPU default route; explicit reference row-work policy".into() };
    let unbound = export::compile_datalog(source_text, configuration.clone()).unwrap();
    let mut evidence = request.collect(&root).unwrap();
    evidence.targets.insert("cpu".into(), ev::TargetEvidence {
        unit: ev::Unit::DeclaredRowWork, build_per_row_h: weight("policy.h", &evidence.dataset.hash), merge_per_row_m: weight("policy.m", &evidence.dataset.hash),
        configuration_hash: unbound.identity.configuration_hash,
    });
    let mut source: cq_datalog::source::Program = syn::parse_str(source_text).unwrap();
    source.declared_evidence = Some(DeclaredEvidence::from_json(&evidence.canonical_json().unwrap(), request.relations.iter().map(|input| RelationBinding {
        relation: syn::parse_str(&input.relation.name).unwrap(), source: input.relation.name.clone(), view: EvidenceView::Full,
        columns: input.relation.columns.iter().map(|column| ColumnBinding { column: syn::parse_str(&column.name).unwrap(), source: column.name.clone() }).collect(),
    }).collect()).unwrap());
    let plans = export::compile_datalog(&source.to_token_stream().to_string(), configuration).unwrap();
    Case { root, request, evidence, plans }
}
fn metadata(case: &Case) {
    let bytes = std::fs::read(case.root.join("meta.json")).unwrap();
    cq_storage::counters::calibration::metadata("meta.json", &bytes);
}
fn run(case: &Case, edges: &[(i32,i32)], seeds: &[(i32,)]) -> calibration::Result<calibration::CalibrationResult<BTreeSet<(i32,)>>> {
    calibration::measure(&case.plans, &case.evidence, &case.request, "completed-fixture", || {
        metadata(case);
        fixture::reach(edges, seeds)
    })
}

#[test]
fn complete_run_emits_exact_input_round_physical_and_recosted_evidence() {
    let edges = [(0,1),(0,1),(1,2),(2,3)]; let seeds = [(0,),(0,)];
    let case = case(&edges, &seeds);
    let result = run(&case, &edges, &seeds).unwrap();
    assert_eq!(result.value, BTreeSet::from([(0,),(1,),(2,),(3,)]));
    result.evidence.check().unwrap(); result.plans.check().unwrap();
    assert_eq!(result.evidence.relations["Edge"].rows.as_ref().unwrap().value, Some(3));
    assert_eq!(result.evidence.relations["Seed"].rows.as_ref().unwrap().value, Some(1));
    let recursive = result.evidence.relations.values().find_map(|r| r.recursive.as_ref()).unwrap();
    assert_eq!(recursive.final_rows.value, Some(4));
    assert_eq!(recursive.rounds.value, Some(4));
    assert_eq!(recursive.seed_rows.as_ref().unwrap().value, Some(1));
    assert_eq!(recursive.unconsumed_final_delta_rows.as_ref().unwrap().value, Some(0));
    assert_eq!(result.evidence.observations.rounds[0].entries.iter().map(|r| r.delta_rows.value.unwrap()).collect::<Vec<_>>(), [1,1,1,1]);
    assert!(!result.report.physical_steps.is_empty());
    assert!(result.report.physical_steps.iter().any(|step| step.measured.fused));
    assert!(result.report.physical_steps.iter().all(|step| step.measured.comparison.as_ref().is_none_or(|bound| bound.within_bound)));
    assert_eq!(result.plans.identity.program_hash, case.plans.identity.program_hash);
    assert!(result.evidence.observations.physical_steps.len() == result.report.physical_steps.len());
    assert!(result.planning_evidence().observations.physical_steps.is_empty());
    assert_eq!(result.advice.evidence_hash, export::sha256(result.evidence.canonical_json().unwrap()));
    let report = result.report.render(&result.evidence);
    assert!(report.contains("fused; counted once") && report.contains("No runtime advice adoption"));
    // Review an alternative without executing it: changed prefix/key scopes
    // must not acquire incumbent row counts or invented fan-out maxima.
    let mut alternative = result.advice.clone();
    let hole = alternative.holes.iter_mut().find(|hole| hole.candidates.len() > 1).unwrap();
    hole.selected = hole.candidates.iter().position(|candidate| !candidate.plan.current).unwrap();
    hole.decision = cq_cost::cost::Decision::Changed;
    hole.noise_screen = None;
    let reviews = calibration::assess_bounds(&result.report, &alternative, &result.plans, &result.evidence).unwrap();
    assert!(!reviews.is_empty());
    assert!(reviews.iter().all(|review| review.calibrated_fanout_recost.is_none()));
    assert!(reviews.iter().flat_map(|review| &review.proposed_steps).any(|step| !step.same_executed_prefix));
    assert!(reviews.iter().flat_map(|review| &review.proposed_steps)
        .filter(|step| !step.same_executed_prefix).all(|step| step.comparison.is_none()));
    alternative.input_hash = export::sha256("other program");
    assert!(calibration::assess_bounds(&result.report, &alternative, &result.plans, &result.evidence).is_err());
    let output = case.root.join("output"); result.write_artifacts(&output).unwrap();
    let restored = ev::parse_json(&std::fs::read_to_string(output.join("calibration.evidence.json")).unwrap()).unwrap();
    assert_eq!(restored, result.evidence);
    assert!(result.write_artifacts(&output).is_err(), "never overwrite existing artifacts");
}

#[test]
fn empty_fixpoint_has_zero_executed_rounds_without_invented_delta_entries() {
    let case = case(&[], &[]);
    let result = run(&case, &[], &[]).unwrap();
    assert!(result.value.is_empty());
    let recursive = result.evidence.relations.values().find_map(|r| r.recursive.as_ref()).unwrap();
    assert_eq!(recursive.final_rows.value, Some(0)); assert_eq!(recursive.rounds.value, Some(0));
    assert!(result.evidence.observations.rounds[0].entries.is_empty());
    assert!(!result.report.unexecuted_holes.is_empty());
    result.evidence.check().unwrap();
}

#[test]
fn incomplete_wrong_dataset_and_metadata_runs_never_publish_facts() {
    let edges = [(0,1)]; let seeds = [(0,)]; let case = case(&edges, &seeds);
    assert!(calibration::measure(&case.plans, &case.evidence, &case.request, "no-program", || metadata(&case)).is_err());
    assert!(run(&case, &[(0,2)], &seeds).is_err(), "decoded row identity mismatch");
    assert!(calibration::measure(&case.plans, &case.evidence, &case.request, "no-meta", || fixture::reach(&edges, &seeds)).is_err());
    assert!(calibration::measure(&case.plans, &case.evidence, &case.request, "bad-meta", || {
        cq_storage::counters::calibration::metadata("meta.json", b"{}"); fixture::reach(&edges, &seeds)
    }).is_err());
    assert!(calibration::measure(&case.plans, &case.evidence, &case.request, "two-programs", || {
        metadata(&case); fixture::reach(&edges, &seeds); fixture::reach(&edges, &seeds)
    }).is_err());
    let error = calibration::try_measure::<()>(&case.plans, &case.evidence, &case.request, "loader-error", || Err(export::ExportError("original loader error".into()))).err().unwrap();
    assert_eq!(error.to_string(), "original loader error");
}

fn parallel_case(edges: &[(i32,i32)], seeds: &[(i32,)]) -> Case {
    make_parallel(case(edges, seeds))
}
fn make_parallel(mut case: Case) -> Case {
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
fn parallel_push_instances_preserve_complete_rounds_and_evidence_bindings() {
    let edges = [(0,1),(1,2),(2,3),(10,11)];
    let seeds = [(0,),(10,),(10,)];
    let case = parallel_case(&edges, &seeds);
    let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
    let result = pool.install(|| calibration::measure(&case.plans, &case.evidence, &case.request, "parallel-fixture", || {
        metadata(&case); parallel_reach(&edges, &seeds)
    })).unwrap();
    assert_eq!(result.value, BTreeSet::from([(0,),(1,),(2,),(3,),(10,)]));
    result.evidence.check().unwrap(); result.plans.check().unwrap();
    let recursive = result.evidence.relations.values().find_map(|r| r.recursive.as_ref()).unwrap();
    assert_eq!(recursive.final_rows.value, Some(5));
    assert_eq!(recursive.rounds.value, Some(4));
    assert_eq!(result.evidence.observations.rounds[0].entries.iter().map(|e| e.delta_rows.value.unwrap()).collect::<Vec<_>>(), [2,1,1,1]);
    assert!(result.report.physical_steps.iter().any(|step| step.measured.fused));
    assert!(result.report.physical_steps.iter().all(|step| step.measured.comparison.as_ref().is_none_or(|bound| bound.within_bound)));
    assert!(result.report.traces.values().all(|trace| !trace.host_aliases.is_empty()));
    assert!(result.plans.holes.iter().any(|hole| hole.evidence.as_ref().is_some_and(|e|
        e.bindings.iter().any(|b| b.view == export::EvidenceView::Delta))));
}

const CARTESIAN: &str = stringify! {
    struct CartesianCalibration;
    relation Edge(x:i32,y:i32) = (edges);
    relation Seed(x:i32) = (seeds);
    relation Out(x:i32,y:i32,z:i32);
    output Out;
    Product: Out(*x,*y,*z) :- Edge(x,y), Seed(z);
};
fn cartesian(edges: &[(i32,i32)], seeds: &[(i32,)]) -> BTreeSet<(i32,i32,i32)> {
    cq_datalog::datalog! {
        struct CartesianCalibration;
        relation Edge(x:i32,y:i32) = (edges);
        relation Seed(x:i32) = (seeds);
        relation Out(x:i32,y:i32,z:i32);
        output Out;
        Product: Out(*x,*y,*z) :- Edge(x,y), Seed(z);
    }.into_iter().collect()
}
fn parallel_cartesian(edges: &[(i32,i32)], seeds: &[(i32,)]) -> BTreeSet<(i32,i32,i32)> {
    cq_datalog::datalog! {
        pipeline {
            storage_rewrites = [cq_datalog_parallel::shard_storage(layouts(sharded))];
            round = [cq_datalog_parallel::partition(threshold=0, grain=1)];
            print = cq_datalog_parallel::print_rounds;
        }
        struct CartesianCalibration;
        relation Edge(x:i32,y:i32) = (edges);
        relation Seed(x:i32) = (seeds);
        relation Out(x:i32,y:i32,z:i32);
        output Out;
        Product: Out(*x,*y,*z) :- Edge(x,y), Seed(z);
    }.into_iter().collect()
}

/// M6: record a stream-work correspondence certificate for repeated inputs
/// (`features/planners/cost/src/calibration/report.rs::evaluation_scopes`).
/// Sparse lowering Steps are not that certificate; report unknown bounds.
#[test]
fn keyless_rhs_replay_reports_unknown_without_stream_work_certificate() {
    let edges = [(0,1),(1,2),(2,3),(3,4)]; let seeds = [(10,),(20,)];
    let expected = edges.iter().flat_map(|&(x,y)| seeds.iter().map(move |&(z,)| (x,y,z))).collect::<BTreeSet<_>>();
    let pool = rayon::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
    for parallel in [false, true] {
        let case = case_for(CARTESIAN, &edges, &seeds);
        let case = if parallel { make_parallel(case) } else { case };
        let result = pool.install(|| calibration::measure(&case.plans, &case.evidence, &case.request, "cartesian", || {
            metadata(&case);
            if parallel { parallel_cartesian(&edges, &seeds) } else { cartesian(&edges, &seeds) }
        })).unwrap();
        assert_eq!(result.value, expected);
        let replayed = result.report.physical_steps.iter().filter(|step|
            step.bound_role == "unmodeled repeated-input evaluation scope"
        ).collect::<Vec<_>>();
        assert!(!replayed.is_empty(), "the repeated operand scope must be explicit");
        for step in &replayed {
            assert!(step.planned_comparison.is_none());
            assert!(step.measured.comparison.is_none());
            assert!(!step.planned_unknown.is_empty());
            assert!(!step.measured.unknown.is_empty());
        }
        // Without the stream-work certificate, these diagnostic scopes cannot
        // be summed as one join-output counter. Exact output is checked above.
        assert!(result.report.physical_steps.iter().all(|step| step.measured.comparison.as_ref().is_none_or(|bound| bound.within_bound)));
        result.evidence.check().unwrap();
    }
}

#[test]
fn unobserved_operators_are_not_published_as_measured_zero() {
    let edges = [(10,11)]; let seeds = [(10,)];
    let case = case(&edges, &seeds);
    let result = run(&case, &edges, &seeds).unwrap();
    assert_eq!(result.value, BTreeSet::from([(10,)]));
    assert!(!result.report.unobserved_physical_steps.is_empty());
    for absent in &result.report.unobserved_physical_steps {
        assert!(!result.report.physical_steps.iter().any(|step|
            step.measured.hole == format!("hole:{}", absent.hole)
                && step.measured.instance.starts_with(&format!("program:{}:", absent.program))
                && step.measured.physical_steps.contains(&absent.physical_step)));
    }
    assert!(result.report.physical_steps.iter().any(|step| step.measured.rows.value == Some(0)),
        "a genuinely observed empty output remains an exact zero");
}
