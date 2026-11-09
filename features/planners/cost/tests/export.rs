use cq_cost::export::{self, CheckedIr, Configuration, DatasetIdentity, Origin, PlannerInput, Target, Version};
use cq_lang_datalog_round as round;
use quote::ToTokens;

const CQ: &str = r#"
    struct Query;
    relation R(x: i32, y: i32) = (r);
    relation S(y: i32, z: i32) = (s);
    Query(x,z) :- R(x,y), S(y,z), if opaque!(x,z);
"#;
const DL: &str = r#"
    struct Closure;
    relation Edge(x: i32, y: i32) = (edges);
    relation Reach(x: i32, y: i32);
    output Reach;
    Seed: Reach(*x,*y) :- Edge(x,y);
    Transitive: Reach(*x,*z) :- Reach(x,y), Reach(y,z);
"#;

fn config() -> Configuration {
    Configuration {
        compiler_hash: export::sha256("test compiler build identity"),
        target: Target::Cpu,
        invocation: "test production CPU prefix route".into(),
    }
}
fn cq() -> PlannerInput {
    export::compile_cq(CQ, &Default::default(), config()).unwrap()
}
fn compile_round(source: &str) -> round::Program {
    let source = syn::parse_str(source).unwrap();
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile_with_driver(
        &strata, cq_datalog_incremental::DriverPolicy::DeltaFirst,
    ).unwrap();
    let storage = cq_datalog_storage::compile(&incremental).unwrap();
    cq_datalog_fork::compile(&storage).unwrap()
}
fn hole_plans(program: &round::Program) -> Vec<(cq_lang_cq::Program, cq_lang_vo::Program)> {
    program.holes.iter().map(|hole| {
        let pipeline = hole.invocation_pipeline().unwrap();
        let options = syn::parse2(pipeline.passes[hole.plan.unwrap()].arguments.clone()).unwrap();
        cq_planning::prefix_valuation(&hole.cq, &options).unwrap()
    }).collect()
}

fn recorded_plan(hole: &export::Hole) -> (cq_lang_cq::Program, cq_lang_vo::Program) {
    let normal = syn::parse_str(&hole.normalized_cq).unwrap();
    let valuation = syn::parse::Parser::parse_str(
        |input: syn::parse::ParseStream| cq_lang_vo::Program::parse(input, &normal), &hole.checked_vo,
    ).unwrap();
    (normal, valuation)
}

#[test]
fn checked_cq_roundtrip_is_deterministic_and_complete() {
    let record = cq();
    let json = record.canonical_json().unwrap();
    assert_eq!(export::parse_json(&json).unwrap(), record);
    assert_eq!(cq().canonical_json().unwrap(), json);
    assert_eq!(record.identity.source_hash, export::sha256(CQ));
    assert_eq!(record.identity.dataset, DatasetIdentity::Unbound {});
    assert!(record.layouts.is_empty(), "VO boundary must not invent physical providers");
    let hole = &record.holes[0];
    assert_eq!(hole.candidate.positive_clauses.len(), 2);
    assert_eq!(hole.occurrences.len(), 2);
    assert!(hole.candidate.clauses.iter().any(|c| c.opaque.as_ref().is_some_and(|o| o.conservative_environment)));
    let guard = hole.candidate.steps.iter().find(|s| s.ir.contains("opaque")).unwrap();
    assert!(!guard.required.is_empty(), "macro environment must survive as supplied binding facts");
    assert!(!guard.atom);
    let (normal, plan) = recorded_plan(hole);
    cq_lang_vo::contract::check(&normal, &plan).unwrap();
    assert_eq!(plan.to_token_stream().to_string(), hole.checked_vo);
    assert!(hole.checked_vo.starts_with("valuation"));
    assert!(!hole.checked_vo.contains("struct Query"));
    record.check_against(&record.identity).unwrap();
}

#[test]
fn wrong_missing_and_duplicate_identity_are_rejected() {
    let record = cq();
    let mut changed = record.clone();
    changed.identity.source_hash = export::sha256("different source");
    assert!(changed.check().is_err());
    let mut changed = record.clone();
    changed.identity.program_hash = export::sha256("different program");
    assert!(changed.check().is_err());
    let mut changed = record.clone();
    changed.identity.configuration_hash = export::sha256("different configuration");
    assert!(changed.check().is_err());
    let mut value = serde_json::to_value(&record).unwrap();
    value["identity"].as_object_mut().unwrap().remove("source_hash");
    assert!(export::parse_json(&value.to_string()).is_err());
    let mut value = serde_json::to_value(&record).unwrap();
    value["identity"].as_object_mut().unwrap().remove("dataset");
    assert!(export::parse_json(&value.to_string()).is_err());
    let json = record.canonical_json().unwrap();
    let duplicated = json.replacen("\"source_hash\":", &format!("\"source_hash\":{:?},\"source_hash\":", record.identity.source_hash), 1);
    assert!(export::parse_json(&duplicated).is_err());
    let mut value = serde_json::to_value(&record).unwrap();
    value["identity"]["unknown"] = true.into();
    assert!(export::parse_json(&value.to_string()).is_err());
    for path in ["/identity/dataset", "/checked_ir", "/holes/0/origin"] {
        let mut value = serde_json::to_value(&record).unwrap();
        value.pointer_mut(path).unwrap()["unexpected"] = true.into();
        assert!(export::parse_json(&value.to_string()).is_err(), "accepted unknown field in {path}");
    }
    let mut changed = record.clone();
    changed.schema = "mini-linq.planner-input.v999".into();
    assert!(changed.check().is_err());
}

#[test]
fn external_identity_rejects_self_consistent_wrong_build() {
    let original = cq();
    let mut other = config();
    other.compiler_hash = export::sha256("another compiler");
    let changed = export::compile_cq(CQ, &Default::default(), other).unwrap();
    changed.check().unwrap();
    assert!(changed.check_against(&original.identity).is_err());
    let mut malformed = config();
    malformed.compiler_hash = "unknown".into();
    assert!(export::compile_cq(CQ, &Default::default(), malformed).is_err());
}

#[test]
fn candidate_and_occurrence_mutants_are_not_partial_search_spaces() {
    let record = cq();
    let mut changed = record.clone();
    changed.holes[0].occurrences.pop();
    assert!(changed.check().is_err());
    let mut changed = record.clone();
    changed.holes[0].occurrences[0].normalized_path = vec![99];
    assert!(changed.check().is_err());
    let mut changed = record.clone();
    changed.holes[0].occurrences[0].columns.swap(0, 1);
    assert!(changed.check().is_err());
    let mut changed = record.clone();
    changed.holes[0].occurrences[0].vo_steps.clear();
    assert!(changed.check().is_err());
    let mut changed = record.clone();
    changed.holes[0].candidate.clauses.pop();
    assert!(changed.check().is_err());
    let mut changed = record.clone();
    changed.holes[0].candidate.steps.iter_mut().find(|s| !s.required.is_empty()).unwrap().required.clear();
    assert!(changed.check().is_err());
    let mut changed = record.clone();
    changed.source.push(' ');
    assert!(changed.check().is_err(), "exact source bytes participate in identity");
    let mut value = serde_json::to_value(&record).unwrap();
    value["holes"][0]["occurrences"][0].as_object_mut().unwrap().remove("columns");
    assert!(export::parse_json(&value.to_string()).is_err());
}

#[test]
fn unrelated_checked_vo_is_not_accepted_as_current_plan() {
    let record = cq();
    let (normal, plan) = recorded_plan(&record.holes[0]);
    let other_source = CQ.replace("opaque!(x,z)", "other!(x,z)");
    assert!(export::export_cq(&other_source, &normal, &plan, config()).is_err());
}

#[test]
fn nested_scopes_keep_full_candidate_interfaces_and_step_paths() {
    let source = r#"
        struct Branches;
        relation R(x:i32,y:i32) = (r);
        relation S(y:i32,z:i32) = (s);
        relation T(y:i32,z:i32) = (t);
        Branches(x,z) :- R(x,y), { S(y,z) } | { T(y,z) };
    "#;
    let record = export::compile_cq(source, &Default::default(), config()).unwrap();
    assert_eq!(export::parse_json(&record.canonical_json().unwrap()).unwrap(), record);
    let block = record.holes[0].candidate.steps.iter().find(|s| !s.branches.is_empty()).unwrap();
    assert_eq!(block.branches.len(), 2);
    assert_eq!(block.block_kind.as_deref(), Some("union"));
    assert!(record.holes[0].occurrences.iter().any(|o| o.normalized_path.len() == 3));
    assert!(record.holes[0].occurrences.iter().filter(|o| o.normalized_path.len() == 3).all(|o| o.vo_steps.iter().all(|path| path.len() == 3)));
}

#[test]
fn datalog_consumes_recorded_variants_and_layouts_without_rescheduling() {
    let program = compile_round(DL);
    let plans = hole_plans(&program);
    let record = export::export_datalog(DL, &program, &plans, config()).unwrap();
    assert_eq!(record.holes.len(), program.holes.len());
    let CheckedIr::Datalog { round: ir } = &record.checked_ir else { panic!("Datalog record"); };
    let parsed: round::Program = syn::parse_str(ir).unwrap();
    assert_eq!(program, parsed);
    assert_eq!(export::parse_json(&record.canonical_json().unwrap()).unwrap(), record);
    let steady: Vec<_> = record.holes.iter().filter(|hole| matches!(hole.origin, Origin::Rule { seed: false, .. })).collect();
    assert_eq!(steady.len(), 2, "both repeated changing occurrences must be present");
    for hole in steady {
        let Origin::Rule { original_delta: Some(delta), permutation, source_label, driver, .. } = &hole.origin else { panic!("changing rule"); };
        assert_eq!(driver, "DeltaFirst");
        assert_eq!(source_label.as_deref(), Some("Transitive"));
        assert_eq!(permutation[0], *delta);
        assert_eq!(hole.occurrences.len(), 2);
        assert_eq!(hole.occurrences.iter().filter(|o| o.version == Version::Delta).count(), 1);
        assert!(hole.occurrences.iter().all(|o| o.logical_relation == "Reach"));
        assert!(hole.occurrences.iter().all(|o| o.demand.is_some()));
        assert_ne!(hole.occurrences[0].source_path, hole.occurrences[1].source_path);
    }
    assert!(record.layouts.iter().all(|layout| !layout.provider_ir.is_empty() && !layout.maintenance.is_empty()));
    let mut wrong = record.clone();
    let changing = wrong.holes.iter_mut().find(|h| matches!(h.origin, Origin::Rule { seed: false, .. })).unwrap();
    changing.occurrences[0].version = Version::Full;
    assert!(wrong.check().is_err());
    let mut missing = record.clone();
    missing.holes.pop();
    assert!(missing.check().is_err());
    assert!(export::export_datalog(DL, &program, &plans[..plans.len()-1], config()).is_err());
}

#[test]
fn default_datalog_route_includes_negative_key_holes() {
    let source = r#"
        struct Negative;
        relation Edge(x:i32,y:i32) = (edges);
        relation Blocked(x:i32) = (blocked);
        relation Result(x:i32);
        output Result;
        Keep: Result(*x) :- Edge(x,y), !Blocked(x);
    "#;
    let record = export::compile_datalog(source, config()).unwrap();
    assert!(record.holes.iter().any(|h| matches!(h.origin, Origin::NegativeKey { .. })));
    assert!(record.holes.iter().any(|h| matches!(h.origin, Origin::Rule { seed: true, .. })));
    assert_eq!(export::parse_json(&record.canonical_json().unwrap()).unwrap(), record);
}

#[test]
fn export_command_publishes_one_checked_input() {
    let directory = std::env::temp_dir().join(format!("cq-cost-export-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Remove(std::path::PathBuf);
    impl Drop for Remove {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }
    let _remove = Remove(directory.clone());
    let source = directory.join("query.cq");
    let output = directory.join("plans.json");
    std::fs::write(&source, CQ).unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_cq-cost"))
        .args(["export", "--language", "cq", "--source"])
        .arg(&source)
        .args(["--compiler-hash", &config().compiler_hash, "--out"])
        .arg(&output)
        .output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let written = std::fs::read_to_string(&output).unwrap();
    let record = export::parse_json(&written).unwrap();
    assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), record.input_hash);
    assert_eq!(record.identity.source_hash, export::sha256(CQ));
    let checked = std::process::Command::new(env!("CARGO_BIN_EXE_cq-cost"))
        .args(["check", "--plans"]).arg(&output).output().unwrap();
    assert!(checked.status.success(), "{}", String::from_utf8_lossy(&checked.stderr));
    assert_eq!(String::from_utf8(checked.stdout).unwrap().trim(), record.input_hash);
    let normal = directory.join("normalized.cq");
    let valuation = directory.join("checked.vo");
    let consumed = directory.join("consumed.json");
    std::fs::write(&normal, &record.holes[0].normalized_cq).unwrap();
    std::fs::write(&valuation, &record.holes[0].checked_vo).unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_cq-cost"))
        .args(["export", "--language", "cq", "--source"]).arg(&source)
        .args(["--compiler-hash", &config().compiler_hash, "--out"]).arg(&consumed)
        .arg("--cq").arg(&normal).arg("--vo").arg(&valuation)
        .output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let consumed = export::parse_json(&std::fs::read_to_string(&consumed).unwrap()).unwrap();
    assert_eq!(consumed.holes, record.holes);
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_cq-cost"))
        .args(["export", "--language", "cq", "--source"]).arg(&source)
        .args(["--compiler-hash", &config().compiler_hash, "--out"]).arg(&output)
        .arg("--vo").arg(&valuation)
        .output().unwrap();
    assert!(!result.status.success(), "a standalone valuation requires its recorded CQ");
    let malformed = directory.join("invalid.json");
    std::fs::write(&malformed, "{}").unwrap();
    assert!(!std::process::Command::new(env!("CARGO_BIN_EXE_cq-cost"))
        .args(["check", "--plans"]).arg(&malformed).status().unwrap().success());

    let result = std::process::Command::new(env!("CARGO_BIN_EXE_cq-cost"))
        .args(["export", "--language", "cq", "--source"])
        .arg(&source)
        .args(["--compiler-hash", "invalid", "--out"])
        .arg(&output)
        .output().unwrap();
    assert!(!result.status.success());
    assert_eq!(std::fs::read_to_string(output).unwrap(), written,
        "a rejected input must not replace a previous complete export");
}

#[test]
fn declared_dataset_identity_and_aliases_survive_authored_order() {
    // Local fixture mirrors languages/cq-syntax/tests/fixtures for external consumers.
    let json = include_str!("fixtures/planner-evidence.json");
    let document = cq_syntax::evidence::parse_json(json).unwrap();
    let attachment = quote::quote! {
        declared_evidence #json { R = "R" full [x = "a", y = "b"]; }
    };
    let source = CQ.replace("struct Query;", &format!("struct Query; {attachment}"));
    let options = syn::parse_str("order(R,S)").unwrap();
    let record = export::compile_cq(&source, &options, config()).unwrap();
    let unattached = export::compile_cq(CQ, &options, config()).unwrap();
    assert_eq!(record.identity.program_hash, unattached.identity.program_hash);
    assert_ne!(record.identity.source_hash, unattached.identity.source_hash);
    assert_ne!(record.identity.dataset, unattached.identity.dataset);
    assert_ne!(record.identity.checked_records_hash, unattached.identity.checked_records_hash);
    assert_ne!(record.input_hash, unattached.input_hash);
    let DatasetIdentity::Bound { dataset_hash, schema_hash, evidence_hash } = &record.identity.dataset else {
        panic!("supplied evidence must be bound, not silently ignored");
    };
    assert_eq!(dataset_hash, &document.dataset.hash);
    assert_eq!(schema_hash, &document.dataset.schema_hash);
    assert_eq!(evidence_hash, &export::sha256(document.canonical_json().unwrap()));
    let projected = record.holes[0].evidence.as_ref().unwrap();
    assert_eq!(projected.bindings[0].input_relation, "R");
    assert_eq!(projected.bindings[0].columns[0].input_column, "x");
    assert_eq!(projected.bindings[0].columns[0].dataset_column, "a");
    let (_, plan) = recorded_plan(&record.holes[0]);
    assert!(plan.valuation.evidence.is_none(), "declared evidence must not select legacy root planning");
    assert_eq!(export::parse_json(&record.canonical_json().unwrap()).unwrap(), record);

    let mut wrong = record.clone();
    if let DatasetIdentity::Bound { dataset_hash, .. } = &mut wrong.identity.dataset {
        *dataset_hash = export::sha256("wrong dataset");
    }
    assert!(wrong.check().is_err());
    let mut wrong = record.clone();
    wrong.holes[0].evidence.as_mut().unwrap().bindings[0].columns[0].dataset_column = "b".into();
    assert!(wrong.check().is_err());
    let mut absent = record.clone();
    absent.holes[0].evidence = None;
    assert!(absent.check().is_err());
}

#[test]
fn identity_binds_the_consumed_plan_not_only_the_source() {
    let left = export::compile_cq(CQ, &syn::parse_str("order(R,S)").unwrap(), config()).unwrap();
    let right = export::compile_cq(CQ, &syn::parse_str("order(S,R)").unwrap(), config()).unwrap();
    assert_eq!(left.identity.program_hash, right.identity.program_hash);
    assert_eq!(left.identity.configuration_hash, right.identity.configuration_hash);
    assert_ne!(left.identity.checked_records_hash, right.identity.checked_records_hash);
    assert!(right.check_against(&left.identity).is_err());
}

#[test]
fn candidate_reference_keeps_predicates_preprocessed_by_the_incumbent() {
    let source = r#"
        struct Filtered;
        relation R(x:i32,y:i32) = (r);
        Filtered(x) :- R(x,y), where y > 2;
    "#;
    let record = export::compile_cq(source, &Default::default(), config()).unwrap();
    let hole = &record.holes[0];
    assert!(hole.candidate.steps.iter().any(|step| step.preprocessing));
    assert_eq!(hole.candidate_reference.steps.len(), hole.candidate_reference.clauses.len());
    let predicate = hole.candidate_reference.steps.iter().find(|step| step.ir.starts_with("where")).unwrap();
    assert!(!predicate.preprocessing);
    assert!(predicate.required.contains(&"y".to_owned()));
    assert!(predicate.provides.is_empty());
    assert_eq!(export::parse_json(&record.canonical_json().unwrap()).unwrap(), record);
    let mut incomplete = record.clone();
    incomplete.holes[0].candidate_reference.steps.pop();
    assert!(incomplete.check().is_err());
}

#[test]
fn candidate_reference_counts_variables_not_fixed_literal_arguments() {
    let source = "struct Fixed; relation R(a:i32,b:i32); answer(x) :- R(7,x);";
    let record = export::compile_cq(source, &Default::default(), config()).unwrap();
    assert_eq!(export::parse_json(&record.canonical_json().unwrap()).unwrap(), record);
    let atom = record.holes[0].candidate_reference.steps.iter().find(|step| step.atom).unwrap();
    assert_eq!(atom.provides, ["x"]);
}

#[test]
fn consumed_targets_are_recorded_without_inventing_a_backend() {
    let record = cq();
    let (normal, plan) = recorded_plan(&record.holes[0]);
    for target in [Target::CpuPar, Target::Gpu] {
        let mut configuration = config();
        configuration.target = target;
        let exported = export::export_cq(CQ, &normal, &plan, configuration.clone()).unwrap();
        exported.check().unwrap();
        assert_eq!(exported.configuration.target, target);
        assert!(exported.check_against(&record.identity).is_err());
        assert!(export::compile_cq(CQ, &Default::default(), configuration.clone()).is_err());
        assert!(export::compile_datalog(DL, configuration).is_err());
    }
    let program = compile_round(DL);
    let plans = hole_plans(&program);
    for target in [Target::CpuPar, Target::Gpu] {
        let mut configuration = config();
        configuration.target = target;
        assert!(export::export_datalog(DL, &program, &plans, configuration).is_err());
    }
}

#[test]
fn parallel_record_export_preserves_real_partitioned_occurrences() {
    let source = syn::parse_str(DL).unwrap();
    let strata = cq_datalog_stratify::compile(&source).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let storage = cq_datalog_storage::compile(&incremental).unwrap();
    let storage = cq_datalog_partition::compile_storage(&storage).unwrap();
    let original = cq_datalog_fork::compile(&storage).unwrap();
    let partitioned = cq_datalog_partition::compile_sharded(&original).unwrap();
    let program = &partitioned.round;
    let plans = hole_plans(program);
    let mut configuration = config();
    configuration.target = Target::CpuPar;
    // The caller's full target invocation can retain wrapper-specific options.
    configuration.invocation = partitioned.to_token_stream().to_string();
    let record = export::export_datalog(DL, program, &plans, configuration).unwrap();
    assert_eq!(export::parse_json(&record.canonical_json().unwrap()).unwrap(), record);
    assert!(!program.execution.is_empty());
    assert!(export::export_datalog(DL, program, &plans, config()).is_err());
}

#[test]
fn parallel_target_without_eligible_scans_does_not_invent_work() {
    let source = r#"
        struct NoScan;
        relation Edge(x:i32,y:i32) = (edges);
        relation Reach(x:i32,y:i32);
        output Reach;
        Seed: Reach(*x,*y) :- let limit: i32 = 0, Edge(x,y), if *x >= limit;
    "#;
    let program = compile_round(source);
    assert!(program.context.strata.iter().flat_map(|s| s.seeds.iter().chain(&s.steps))
        .all(|rule| rule.scan.is_none()));
    let plans = hole_plans(&program);
    let mut configuration = config();
    configuration.target = Target::CpuPar;
    let record = export::export_datalog(source, &program, &plans, configuration).unwrap();
    record.check().unwrap();
}
