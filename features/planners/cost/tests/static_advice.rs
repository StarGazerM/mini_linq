use cq_cost::{cost::{self, Decision, advice::{AdviceList, compile_datalog_advice}}, export::{self, Configuration, Target}};
use cq_load::{stats::{ColumnSpec, DistinctMode, RelationSpec}, stats_manifest::{DatasetRequest, RelationInput, REQUEST_SCHEMA}, stats_source::SourceOptions};
use cq_syntax::evidence::{self as ev, Claim, DatasetEvidence, Fact, Provenance, Rational, RelationSemantics, ScalarDomain, Source, TargetEvidence, Unit};
use quote::{ToTokens, quote};
use std::{path::PathBuf, sync::atomic::{AtomicUsize, Ordering}};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!("cq-static-advice-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("R.csv"), (0..20).map(|x| format!("{x},0\n")).collect::<String>()).unwrap();
        std::fs::write(path.join("S.csv"), "0,7\n").unwrap();
        Self(path)
    }
}
impl Drop for Fixture { fn drop(&mut self) { std::fs::remove_dir_all(&self.0).unwrap(); } }
fn config() -> Configuration {
    Configuration { compiler_hash: export::sha256("explicit test compiler"), target: Target::Cpu, invocation: "static advice fixture".into() }
}
fn evidence(files: &Fixture) -> DatasetEvidence {
    let request = DatasetRequest {
        schema: REQUEST_SCHEMA.into(), name: "cost-fixture".into(), date: "2026-11-05".into(), formulation: "cost-test".into(),
        hash_mode: ev::HASH_MODE.into(), distinct: DistinctMode::Exact, metadata: Vec::new(),
        relations: ["R", "S"].into_iter().map(|name| RelationInput {
            relation: RelationSpec { name: name.into(), semantics: RelationSemantics::Set,
                columns: ["c0", "c1"].into_iter().map(|name| ColumnSpec { name: name.into(), domain: ScalarDomain::SignedInteger }).collect(), key_sets: Vec::new() },
            path: format!("{name}.csv"), options: SourceOptions::default(), missing_as_empty: false,
        }).collect(),
    };
    let mut evidence = request.collect(&files.0).unwrap();
    let unit = |id: &str| Fact {
        id: id.into(), source: Source::Declared, claim: Claim::Equal,
        value: Some(Rational { numerator: "1".into(), denominator: "1".into() }), lower: None, upper: None,
        dataset_hash: evidence.dataset.hash.clone(), unit: Unit::DeclaredRowWork,
        provenance: Provenance { producer: "test reference policy".into(), method: "explicit h=m=1 normalized row-work policy, not hardware calibration".into(), locator: "static_advice.rs".into(), run: None, artifact_hash: None },
    };
    evidence.targets.insert("cpu".into(), TargetEvidence {
        unit: Unit::DeclaredRowWork, configuration_hash: export::sha256(serde_json::to_vec(&config()).unwrap()),
        build_per_row_h: unit("cpu.h"), merge_per_row_m: unit("cpu.m"),
    });
    evidence.check().unwrap();
    evidence
}
fn program(evidence: &DatasetEvidence) -> export::PlannerInput {
    let json = evidence.canonical_json().unwrap();
    let source = quote! {
        struct Join;
        relation R(c0:i64,c1:i64) = (r);
        relation S(c0:i64,c1:i64) = (s);
        relation Out(c0:i64,c1:i64);
        declared_evidence #json {
            R = "R" full [c0="c0",c1="c1"];
            S = "S" full [c0="c0",c1="c1"];
        }
        output Out;
        Seed: Out(*x,*z) :- R(x,y), S(y,z);
    }.to_string();
    export::compile_datalog(&source, config()).unwrap()
}

#[test]
fn static_choice_is_checked_before_storage_and_records_its_degree_citations() {
    let files = Fixture::new(); let evidence = evidence(&files); let input = program(&evidence);
    let report = cost::analyze(&input, &evidence, "cpu").unwrap();
    let choice = &report.holes[0];
    assert_eq!(choice.decision, Decision::Changed);
    assert_eq!(choice.candidates[choice.selected].plan.order, [1,0]);
    assert!(choice.citations.iter().any(|id| id.ends_with(":rows")));
    assert_eq!(report.input_hash, input.input_hash);
    assert!(report.whole_run.is_some());
    let parsed: AdviceList = syn::parse_str(&report.advice).unwrap();
    assert_eq!(parsed.0.len(), 1); assert_eq!(parsed.0[0].rule, "Seed");
    assert_eq!(syn::parse2::<AdviceList>(parsed.to_token_stream()).unwrap().0, parsed.0);
    let advised = compile_datalog_advice(&input, &parsed.0).unwrap();
    let source = &advised.storage.incremental.strata.source;
    assert!(advised.context.layouts.iter().any(|layout| source.declarations[layout.relation].name == "R"
        && layout.columns[..layout.key] == [1]), "provider demand must follow the advised root, before Storage");
    // The exact source join has twenty distinct (x,7) answers in either order;
    // the compiler contract checked the emitted plan rather than a private VO.
    let expected: std::collections::BTreeSet<_> = (0..20).map(|x| (x,7)).collect();
    let r: Vec<_> = (0..20).map(|x| (x,0)).collect(); let s = [(0,7)];
    let original = r.iter().flat_map(|&(x,y)| s.iter().filter(move |&&(v,_)| y == v).map(move |&(_,z)| (x,z))).collect::<std::collections::BTreeSet<_>>();
    let reordered = s.iter().flat_map(|&(y,z)| r.iter().filter(move |&&(_,v)| y == v).map(move |&(x,_)| (x,z))).collect::<std::collections::BTreeSet<_>>();
    assert_eq!(original, expected); assert_eq!(reordered, expected);
}

#[test]
fn sub_five_percent_bound_changes_are_recorded_but_never_advised_or_shared() {
    let files = Fixture::new();
    std::fs::write(files.0.join("R.csv"), (0..101).map(|x| format!("{x},0\n")).collect::<String>()).unwrap();
    std::fs::write(files.0.join("S.csv"), (0..100).map(|z| format!("0,{z}\n")).collect::<String>()).unwrap();
    let evidence = evidence(&files);
    let input = program(&evidence);
    let report = cost::analyze(&input, &evidence, "cpu").unwrap();
    let hole = &report.holes[0];
    assert_eq!(hole.decision, Decision::Retained);
    assert!(hole.candidates[hole.selected].plan.current);
    let screen = hole.noise_screen.as_ref().expect("a strictly cheaper loose bound is screened");
    assert_eq!(screen.minimum_ratio, Rational { numerator: "21".into(), denominator: "20".into() });
    assert_eq!(hole.candidates[screen.candidate].plan.order, [1, 0]);
    assert!(!screen.provenance.is_empty());
    assert!(hole.admission.is_none());
    assert!(syn::parse_str::<AdviceList>(&report.advice).unwrap().0.is_empty());
    assert_eq!(report.index_union[&0], hole.candidates[hole.selected].plan.demands);
}

#[test]
fn incomplete_facts_retain_structural_order_and_emit_no_fake_default_advice() {
    let files = Fixture::new(); let mut evidence = evidence(&files);
    let r = evidence.relations.get_mut("R").unwrap(); r.rows = None;
    for c in r.columns.values_mut() { c.distinct = None; c.max_degree = None; c.min = None; c.max = None; }
    let input = program(&evidence);
    let report = cost::analyze(&input, &evidence, "cpu").unwrap();
    assert_eq!(report.holes[0].decision, Decision::Unknown);
    assert!(report.holes[0].candidates[report.holes[0].selected].plan.current);
    assert!(syn::parse_str::<AdviceList>(&report.advice).unwrap().0.is_empty());
    assert!(report.whole_run.is_none());
}

#[test]
fn required_cost_profiles_and_record_identity_are_not_guessed() {
    let files = Fixture::new(); let mut evidence = evidence(&files);
    evidence.targets.clear(); let input = program(&evidence);
    assert!(cost::analyze(&input, &evidence, "cpu").is_err());
    let mut evidence = self::evidence(&files); evidence.targets.get_mut("cpu").unwrap().configuration_hash = export::sha256("wrong configuration");
    let input = program(&evidence); assert!(cost::analyze(&input, &evidence, "cpu").is_err());
    let evidence = self::evidence(&files); let mut input = program(&evidence); input.identity.program_hash = export::sha256("wrong program");
    assert!(cost::analyze(&input, &evidence, "cpu").is_err());
}

#[test]
fn provenance_comments_cannot_inject_an_unchecked_plan_statement() {
    let files = Fixture::new(); let mut evidence = evidence(&files);
    let target = evidence.targets.get_mut("cpu").unwrap();
    target.build_per_row_h.id = "cpu.h\nplan(Evil, order(R@0));".into();
    target.build_per_row_h.provenance.method = "policy\r\nplan(Evil, order(R@0));".into();
    let report = cost::analyze(&program(&evidence), &evidence, "cpu").unwrap();
    let parsed: AdviceList = syn::parse_str(&report.advice).unwrap();
    assert_eq!(parsed.0.len(), 1); assert_eq!(parsed.0[0].rule, "Seed");
}

#[test]
fn command_emits_a_single_static_plan_and_complete_report() {
    let files = Fixture::new(); let evidence = evidence(&files); let input = program(&evidence);
    let plans = files.0.join("plans.json"); let facts = files.0.join("evidence.json");
    let output = files.0.join("costs.json"); let advice = files.0.join("orders.dl");
    std::fs::write(&plans, input.canonical_json().unwrap()).unwrap();
    std::fs::write(&facts, evidence.canonical_json().unwrap()).unwrap();
    let result = std::process::Command::new(env!("CARGO_BIN_EXE_cq-cost"))
        .arg("advise").arg("--plans").arg(&plans).arg("--evidence").arg(&facts)
        .args(["--target","cpu","--out"]).arg(&output).arg("--advice").arg(&advice).output().unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let report: cost::AdviceReport = serde_json::from_slice(&std::fs::read(output).unwrap()).unwrap();
    assert_eq!(report.schema, cost::REPORT_SCHEMA); assert_eq!(report.identity, input.identity);
    let text = std::fs::read_to_string(advice).unwrap(); assert_eq!(text.trim_end(), report.advice.trim_end());
    assert_eq!(syn::parse_str::<AdviceList>(&text).unwrap().0.len(), 1);
}

#[test]
fn filtered_negative_key_scans_keep_their_canonical_provider_demand() {
    let files = Fixture::new(); let evidence = evidence(&files); let input = program(&evidence);
    let mut source: cq_lang_datalog_round::incremental::source::Program = syn::parse_str(&input.source).unwrap();
    source.rules[0] = syn::parse_quote!(Out(*x,*y) :- R(x,y), !S(0,_););
    let input = export::compile_datalog(&source.to_token_stream().to_string(), config()).unwrap();
    let report = cost::analyze(&input, &evidence, "cpu").unwrap();
    let key = report.holes.iter().find(|hole| matches!(hole.origin, export::Origin::NegativeKey { .. })).unwrap();
    let selected = &key.candidates[key.selected];
    assert!(selected.plan.demands.iter().any(|d| d.logical_relation == "S" && d.key == [0,1]));
    assert!(!selected.plan.demands.iter().any(|d| d.logical_relation == "S" && d.key == [0]));
    assert!(selected.cost.whole_run.is_some(), "recorded component key construction executes once");
    assert!(selected.cost.citations.contains("compiler:DL-Round:key_scope=component"));
    assert!(selected.cost.citations.contains("compiler:DL-Round:key-build-once"));
    assert!(!selected.cost.preprocessing.is_empty(), "the recorded single-read restriction has a sourced input-work bound");
}

#[test]
fn persistent_input_indexes_are_charged_once_across_strata() {
    let files = Fixture::new(); let evidence = evidence(&files); let input = program(&evidence);
    let mut source: cq_lang_datalog_round::incremental::source::Program = syn::parse_str(&input.source).unwrap();
    let mut output = source.declarations[2].clone(); output.name = syn::parse_quote!(Out2);
    source.declarations.push(output);
    source.outputs.push(syn::parse_quote!(Out2));
    source.models.insert("Out2".into(), syn::parse_quote!(set));
    let mut query = source.rules[0].clone(); query.head.relation = syn::parse_quote!(Out2);
    source.rules.push(query); source.labels.push(Some(syn::parse_quote!(Second)));
    let input = export::compile_datalog(&source.to_token_stream().to_string(), config()).unwrap();
    let report = cost::analyze(&input, &evidence, "cpu").unwrap();
    assert_eq!(report.holes.len(), 2);
    let mut holes = report.holes.iter().collect::<Vec<_>>(); holes.sort_by_key(|h| h.planning_rank);
    assert_ne!(holes[0].origin, holes[1].origin);
    let first = &holes[0].candidates[holes[0].selected];
    let second = &holes[1].candidates[holes[1].selected];
    assert!(first.plan.demands.is_subset(&holes[1].shared_before));
    let upper = |cost: &cost::LinearCost| cost::bounds::upper(cost, &evidence).unwrap().unwrap().exact().unwrap();
    assert!(upper(first.cost.whole_run.as_ref().unwrap()) > upper(second.cost.whole_run.as_ref().unwrap()));
}

#[test]
fn changed_variants_pin_and_check_the_recorded_admit_owner() {
    let files = Fixture::new(); let evidence = evidence(&files); let input = program(&evidence);
    let report = cost::analyze(&input, &evidence, "cpu").unwrap();
    let constraint = report.holes[0].admission.as_ref().unwrap();
    assert!(constraint.held_fixed);
    assert_eq!(constraint.owner, cost::AdmissionOwner::Admit);
    assert_eq!(constraint.hint, cost::AdmissionHint::Admit {});
    cost::advice::check_admission_constraints(&input, &report).unwrap();
    let plans: AdviceList = syn::parse_str(&report.advice).unwrap();
    assert_eq!(plans.0[0].known, Some(cq_datalog_incremental::Known::Admit));
    cost::advice::check_datalog_advice(&input, &report, &plans.0).unwrap();
    for mutation in 0..7 {
        let mut changed = report.clone();
        match mutation {
            0 => changed.holes[0].admission = None,
            1 => changed.holes[0].admission.as_mut().unwrap().owner = cost::AdmissionOwner::InPlan,
            2 => changed.holes[0].admission.as_mut().unwrap().hint = cost::AdmissionHint::At { level: 1 },
            3 => changed.holes[0].admission.as_mut().unwrap().held_fixed = false,
            4 => changed.holes[0].admission.as_mut().unwrap().source_variant = 99,
            5 => changed.holes[0].selected = usize::MAX,
            6 => changed.holes[0].decision = Decision::Retained,
            _ => unreachable!(),
        }
        assert!(cost::advice::check_admission_constraints(&input, &changed).is_err(), "accepted mutation {mutation}");
    }
    let mut removed = plans.0.clone(); removed[0].known = None;
    assert!(cost::advice::check_datalog_advice(&input, &report, &removed).is_err());
}

#[test]
fn automatic_inplan_owner_is_held_fixed_when_the_new_order_would_choose_admit() {
    let files = Fixture::new(); let evidence = evidence(&files); let input = program(&evidence);
    let mut source: cq_lang_datalog_round::incremental::source::Program = syn::parse_str(&input.source).unwrap();
    source.declarations[2].columns.pop();
    source.rules[0] = syn::parse_quote!(Out(*x) :- R(x,y), S(y,z););
    let input = export::compile_datalog(&source.to_token_stream().to_string(), config()).unwrap();
    let report = cost::analyze(&input, &evidence, "cpu").unwrap();
    assert_eq!(report.holes[0].decision, Decision::Changed);
    let constraint = report.holes[0].admission.as_ref().unwrap();
    assert_eq!(constraint.owner, cost::AdmissionOwner::InPlan);
    assert_eq!(constraint.hint, cost::AdmissionHint::At { level: 2 });
    assert!(constraint.alternative_owner.contains("not compared"));
    let plans: AdviceList = syn::parse_str(&report.advice).unwrap();
    let checked = cost::advice::check_datalog_advice(&input, &report, &plans.0).unwrap();
    let owner = |program: &cq_lang_datalog_round::Program| program.context.strata.iter()
        .flat_map(|s| s.seeds.iter().chain(&s.steps)).next().unwrap().known;
    assert_eq!(owner(&checked), cq_lang_datalog_round::Admission::InPlan);
    let mut unpinned = plans.0.clone(); unpinned[0].known = None;
    assert_eq!(owner(&compile_datalog_advice(&input, &unpinned).unwrap()), cq_lang_datalog_round::Admission::Admit);
    assert!(cost::advice::check_datalog_advice(&input, &report, &unpinned).is_err());
}

#[test]
fn joined_restricted_views_remain_unknown_without_materialization_lifetime_facts() {
    let files = Fixture::new();
    let evidence = evidence(&files);
    let original = program(&evidence);
    let mut source: cq_lang_datalog_round::incremental::source::Program = syn::parse_str(&original.source).unwrap();
    source.rules[0] = syn::parse_quote!(Out(*x,*z) :- R(x,y), S(y,z), where y >= 0;);
    let input = export::compile_datalog(&source.to_token_stream().to_string(), config()).unwrap();
    let report = cost::analyze(&input, &evidence, "cpu").unwrap();
    assert_eq!(report.holes[0].decision, Decision::Unknown);
    assert!(report.holes[0].candidates.iter().all(|candidate|
        candidate.cost.whole_run.is_none()
            && candidate.cost.unknown.iter().any(|reason| reason.starts_with("restricted-view-lifetime:"))));
    assert!(syn::parse_str::<AdviceList>(&report.advice).unwrap().0.is_empty());
}
