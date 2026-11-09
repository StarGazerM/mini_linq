//! Isolated mathematical tests use explicit exported projections. Compiler
//! legality/export roundtrips belong to the export/enumeration integration tests.
//! Every evidence document starts with the checked typed-row-stream fixture.
use std::collections::{BTreeMap, BTreeSet};

use cq_cost::cost::{bounds::{self, BoundModel}, Candidate, Comparison, CostStep,
    IndexDemand, IndexVersion, LinearCost, Read, ReadColumn};
use cq_cost::export::{self, CheckedIr, ClauseKind, Configuration, DatasetIdentity,
    Evidence, EvidenceBinding, EvidenceColumn, EvidenceView, Hole, Identity,
    Layout, Origin, PlannerInput, Scope, Target, Version};
use cq_syntax::evidence::{self as ev, Claim, DatasetEvidence, Rational, Source};
use num_bigint::BigInt;
use num_rational::BigRational;
use serde_json::{Value, json};

// Local fixture mirrors languages/cq-syntax/tests/fixtures for external consumers.
const FIXTURE: &str = include_str!("fixtures/planner-evidence.json");
const HASH: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
fn rat(n: impl ToString) -> Rational { Rational { numerator: n.to_string(), denominator: "1".into() } }
fn number(n: u64) -> BigRational { BigRational::from_integer(BigInt::from(n)) }
fn count(base: &Value, id: &str, unit: &str, n: u64) -> Value {
    let mut fact = base.clone();
    fact["id"] = json!(id); fact["unit"] = json!(unit); fact["value"] = json!(n);
    fact
}
fn stamp(value: &mut Value, hash: &str) {
    match value {
        Value::Object(map) => {
            if let Some(v) = map.get_mut("dataset_hash") { *v = json!(hash); }
            for child in map.values_mut() { stamp(child, hash); }
        }
        Value::Array(array) => for child in array { stamp(child, hash); },
        _ => {}
    }
}
fn checked(mut value: Value) -> DatasetEvidence {
    let manifest: ev::InputManifest = serde_json::from_value(value["dataset"]["manifest"].clone()).unwrap();
    let hash = manifest.dataset_hash().unwrap();
    value["dataset"]["hash"] = json!(&hash);
    value["dataset"]["schema_hash"] = json!(manifest.schema_hash().unwrap());
    stamp(&mut value, &hash);
    ev::parse_json(&value.to_string()).unwrap()
}
fn reseal(evidence: &mut DatasetEvidence) { *evidence = checked(serde_json::to_value(&*evidence).unwrap()); }
fn catalog(data: &[(&str, Vec<Vec<i64>>)]) -> DatasetEvidence {
    let mut value: Value = serde_json::from_str(FIXTURE).unwrap();
    let base_relation = value["relations"]["R"].clone();
    let base_schema = value["dataset"]["manifest"]["schema"]["R"].clone();
    let base_count = base_relation["rows"].clone();
    value["relations"] = json!({});
    value["dataset"]["manifest"]["schema"] = json!({});
    for (name, rows) in data {
        let mut relation = base_relation.clone();
        let mut schema = base_schema.clone();
        relation["columns"] = json!({}); schema["columns"] = json!({});
        for column in 0..rows.first().map_or(2, Vec::len) {
            let name_column = format!("c{column}");
            let mut counts = BTreeMap::<i64, u64>::new();
            for row in rows { *counts.entry(row[column]).or_default() += 1; }
            let mut c = base_relation["columns"]["a"].clone();
            c["distinct"] = count(&base_count, &format!("{name}.{name_column}.distinct"), "distinct", counts.len() as u64);
            c["max_degree"] = count(&base_count, &format!("{name}.{name_column}.degree"), "degree", counts.values().copied().max().unwrap_or(0));
            c["min"] = Value::Null; c["max"] = Value::Null;
            relation["columns"][&name_column] = c;
            schema["columns"][&name_column] = json!({"kind":"signed_integer"});
        }
        relation["rows"] = count(&base_count, &format!("{name}.rows"), "rows", rows.len() as u64);
        value["relations"][*name] = relation;
        value["dataset"]["manifest"]["schema"][*name] = schema;
    }
    let mut h = count(&base_count, "cpu.h", "declared-row-work", 1);
    h["source"] = json!("declared"); h["value"] = json!({"numerator":"1", "denominator":"1"});
    let mut m = h.clone(); m["id"] = json!("cpu.m");
    value["targets"] = json!({"cpu": {"unit":"declared-row-work", "configuration_hash":HASH, "build_per_row_h":h, "merge_per_row_m":m}});
    checked(value)
}
fn recursive(evidence: &mut DatasetEvidence, name: &str, rounds: u64, partition: bool) {
    let run = ev::RunIdentity { program_hash: HASH.into(), configuration_hash: HASH.into(),
        admission: ev::Admission::InsertOnlySet, run_id: "tiny-completed-run".into(), completed: true };
    let relation = evidence.relations.get_mut(name).unwrap();
    relation.semantics = ev::RelationSemantics::Set;
    relation.scope = ev::RelationScope::RecursiveFinal;
    let fact = relation.rows.as_ref().unwrap().clone();
    let mut final_rows = fact.clone(); final_rows.id = format!("{name}.final"); final_rows.provenance.run = Some(run.clone());
    let mut n = final_rows.clone(); n.id = format!("{name}.rounds"); n.unit = ev::Unit::Rounds; n.value = Some(rounds);
    relation.recursive = Some(ev::RecursiveEvidence { stratum: "s0".into(), final_rows, rounds: n,
        round_convention: ev::ROUND_CONVENTION.into(), run: run.clone(), seed_rows: None, unconsumed_final_delta_rows: None });
    relation.rows.as_mut().unwrap().provenance.run = Some(run.clone());
    for column in relation.columns.values_mut() {
        for f in [&mut column.distinct, &mut column.max_degree].into_iter().flatten() { f.provenance.run = Some(run.clone()); }
    }
    evidence.dataset.manifest.schema.get_mut(name).unwrap().semantics = ev::RelationSemantics::Set;
    let mut provenance = fact.provenance; provenance.run = Some(run);
    evidence.premises.push(if partition {
        ev::Premise::DeltaPartition { id: format!("{name}.partition"), relation: name.into(), includes_seed_delta: true,
            insert_only_set: true, complete_consumption: true, source: Source::Declared,
            dataset_hash: evidence.dataset.hash.clone(), provenance }
    } else {
        ev::Premise::DeltaSubset { id: format!("{name}.subset"), relations: vec![name.into()], source: Source::Declared,
            dataset_hash: evidence.dataset.hash.clone(), provenance }
    });
    reseal(evidence);
}
fn rule(seed: bool) -> Origin {
    Origin::Rule { stratum: 0, seed, variant: 0, source_rule: 0, source_label: Some("Test".into()),
        head_relation: 0, original_delta: (!seed).then_some(0), driver: "recorded".into(), permutation: vec![],
        authored: false, admission: "recorded insert-only-set".into(), fanout: vec![] }
}
fn fixture(evidence: &DatasetEvidence, reads: &[(&str, &[&str], Version)], origin: Origin) -> (PlannerInput, Candidate) {
    let mut bound = BTreeSet::new();
    let mut bindings = Vec::new();
    let mut steps = Vec::new();
    for (index, (relation, vars, version)) in reads.iter().enumerate() {
        let input_relation = format!("explicit_physical_alias_{index}");
        let columns: Vec<_> = vars.iter().enumerate().map(|(c, var)| ReadColumn { source_column: c,
            input_column: format!("physical_column_{c}"), variable: Some((*var).into()) }).collect();
        bindings.push(EvidenceBinding { input_relation: input_relation.clone(), dataset_relation: (*relation).into(),
            view: if *version == Version::Delta { EvidenceView::Delta } else { EvidenceView::Full },
            columns: columns.iter().map(|c| EvidenceColumn { input_column: c.input_column.clone(), dataset_column: format!("c{}", c.source_column) }).collect() });
        let provides: Vec<_> = vars.iter().filter(|v| !bound.contains(**v)).map(|v| (*v).to_string()).collect();
        steps.push(CostStep { clause: index, kind: ClauseKind::Atom,
            read: Some(Read { source_occurrence: index, normalized_clause: index,
                logical_relation: format!("source_{relation}"), input_relation, version: *version, columns }),
            required: vars.iter().filter(|v| bound.contains(**v)).map(|v| (*v).into()).collect(),
            provides: provides.clone(), bound_before: bound.iter().cloned().collect(), introduces: !provides.is_empty() });
        bound.extend(provides);
    }
    let scope = Scope { clauses: vec![], positive_clauses: vec![], steps: vec![] };
    let canonical = evidence.canonical_json().unwrap();
    let hole = Hole { id: 0, origin, source_cq: "isolated projected bound test".into(), normalized_cq: String::new(),
        checked_vo: String::new(), vo_hash: HASH.into(), captures_ir: String::new(), ordering_ir: None,
        evidence: Some(Evidence { canonical_json: canonical.clone(), attachment_ir: String::new(), bindings }),
        candidate: scope.clone(), candidate_reference: scope, occurrences: vec![] };
    let input = PlannerInput { schema: export::SCHEMA.into(), identity: Identity { source_hash: HASH.into(),
        program_hash: HASH.into(), compiler_hash: HASH.into(), configuration_hash: HASH.into(), checked_records_hash: HASH.into(),
        dataset: DatasetIdentity::Bound { dataset_hash: evidence.dataset.hash.clone(), schema_hash: evidence.dataset.schema_hash.clone(), evidence_hash: export::sha256(canonical) } },
        configuration: Configuration { compiler_hash: HASH.into(), target: Target::Cpu, invocation: "isolated mathematical fixture".into() },
        source: String::new(), checked_ir: CheckedIr::Cq {}, holes: vec![hole], layouts: vec![], input_hash: HASH.into() };
    (input, Candidate { order: (0..reads.len()).collect(), steps, demands: BTreeSet::new(), current: true, unbounded: vec![] })
}
fn cost(input: &PlannerInput, candidate: &Candidate, evidence: &DatasetEvidence) -> cq_cost::cost::CandidateCost {
    bounds::cost_candidate(input, &input.holes[0], candidate, evidence, "cpu", &BTreeSet::new()).unwrap()
}
fn whole(input: &PlannerInput, candidate: &Candidate, evidence: &DatasetEvidence) -> BigRational {
    bounds::upper(&cost(input, candidate, evidence).whole_run.unwrap(), evidence).unwrap().unwrap().exact().unwrap()
}
fn constant(n: u64) -> LinearCost { LinearCost { constant: rat(n), terms: BTreeMap::new() } }
fn linear(terms: &[(&str, u64)], n: u64) -> LinearCost {
    LinearCost { constant: rat(n), terms: terms.iter().map(|(name, value)| ((*name).into(), rat(*value))).collect() }
}
fn layout(name: &str, version: Version, key: &[usize], policy: &str) -> Layout {
    let mut columns = key.to_vec(); for c in 0..2 { if !columns.contains(&c) { columns.push(c); } }
    Layout { version, index: 0, logical_relation: format!("source_{name}"), input_relation: "recorded_layout".into(), columns,
        key: key.len(), provider_ir: "recorded_provider".into(), hasher_ir: "recorded_hasher".into(), maintenance: policy.into(), protocol_ir: "recorded_protocol".into() }
}
fn demand(name: &str, version: IndexVersion, key: &[usize]) -> IndexDemand {
    IndexDemand { logical_relation: format!("source_{name}"), version, key: key.to_vec() }
}

#[test]
fn skewed_and_correlated_tiny_joins_never_underestimate_exact_prefixes() {
    // Enumerate bags over a correlated two-column support, including duplicates
    // and empty inputs. No probabilistic/independent column assumption enters.
    for mask_r in 0..16_u32 { for mask_s in 0..16_u32 {
        let support = [vec![0, 0], vec![0, 0], vec![0, 1], vec![1, 1]];
        let r: Vec<_> = support.iter().enumerate().filter(|(i, _)| mask_r & (1 << i) != 0).map(|(_, r)| r.clone()).collect();
        let s: Vec<_> = support.iter().enumerate().filter(|(i, _)| mask_s & (1 << i) != 0).map(|(_, r)| r.clone()).collect();
        let e = catalog(&[("UnrelatedCaseR", r.clone()), ("DatasetS", s.clone())]);
        let (input, candidate) = fixture(&e, &[("UnrelatedCaseR", &["x", "y"], Version::Full), ("DatasetS", &["x", "z"], Version::Full)], Origin::Cq {});
        let c = cost(&input, &candidate, &e);
        let actual = r.iter().flat_map(|r| s.iter().filter(move |s| r[0] == s[0])).count() as u64;
        let bound = c.steps[1].output.as_ref().unwrap().constant.exact().unwrap();
        assert!(bound >= number(actual), "{r:?} join {s:?}: {bound} < {actual}");
        assert_eq!(c.steps[0].work.as_ref().unwrap().constant, rat(0));
    } }
}

#[test]
fn composite_degree_uses_min_single_column_and_explicit_composite() {
    let mut e = catalog(&[("R", vec![vec![0, 0], vec![0, 1], vec![1, 0], vec![1, 1]]),
        ("S", vec![vec![0, 0, 10], vec![0, 1, 20], vec![1, 0, 30], vec![1, 1, 40]])]);
    // Individual degrees are two, not their product four.
    let reads = [("R", &["x", "y"][..], Version::Full), ("S", &["x", "y", "z"][..], Version::Full)];
    let (input, candidate) = fixture(&e, &reads, Origin::Cq {});
    let c = cost(&input, &candidate, &e);
    assert_eq!(c.steps[1].output.as_ref().unwrap().constant, rat(8));
    let mut degree = e.relations["S"].columns["c0"].max_degree.clone().unwrap();
    degree.id = "S.c0c1.degree".into(); degree.value = Some(1);
    e.relations.get_mut("S").unwrap().key_sets.push(ev::KeySetEvidence { columns: vec!["c0".into(), "c1".into()], distinct: None, max_degree: Some(degree) });
    reseal(&mut e);
    let (input, candidate) = fixture(&e, &reads, Origin::Cq {});
    let c = cost(&input, &candidate, &e);
    assert_eq!(c.steps[1].output.as_ref().unwrap().constant, rat(4));
    assert!(c.steps[1].citations.contains("S.c0c1.degree"));
}

#[test]
fn a_base_key_duplicated_by_a_previous_join_is_not_a_prefix_key() {
    let mut e = catalog(&[("R", vec![vec![1, 7]]),
        ("S", (0..8).map(|z| vec![7, z]).collect()), ("T", vec![vec![1, 9]])]);
    let provenance = e.relations["R"].rows.as_ref().unwrap().provenance.clone();
    e.relations.get_mut("R").unwrap().keys.push(ev::KeyDeclaration { columns: vec!["c0".into()], source: Source::Declared,
        dataset_hash: e.dataset.hash.clone(), provenance });
    reseal(&mut e);
    let (input, candidate) = fixture(&e, &[("R", &["x", "y"], Version::Full), ("S", &["y", "z"], Version::Full), ("T", &["x", "w"], Version::Full)], Origin::Cq {});
    let c = cost(&input, &candidate, &e);
    assert_eq!(c.steps[1].output.as_ref().unwrap().constant, rat(8));
    assert_eq!(c.steps[2].output.as_ref().unwrap().constant, rat(8));
    assert_eq!(whole(&input, &candidate, &e), number(16));
}

#[test]
fn unknown_hll_and_zero_are_distinct_and_bindings_are_not_guessed() {
    let mut e = catalog(&[("R", vec![vec![0, 0]]), ("S", vec![vec![0, 1]])]);
    let r = e.relations.get_mut("S").unwrap(); r.rows = None;
    for column in r.columns.values_mut() {
        column.max_degree = None;
        let distinct = column.distinct.as_mut().unwrap(); distinct.source = Source::Hll; distinct.claim = Claim::Observation;
    }
    reseal(&mut e);
    let reads = [("R", &["x", "y"][..], Version::Full), ("S", &["x", "z"][..], Version::Full)];
    let (mut input, candidate) = fixture(&e, &reads, Origin::Cq {});
    assert!(cost(&input, &candidate, &e).whole_run.is_none());
    assert!(!cost(&input, &candidate, &e).citations.iter().any(|id| id.starts_with("S.")));
    input.holes[0].evidence.as_mut().unwrap().bindings.clear();
    assert!(cost(&input, &candidate, &e).whole_run.is_none());
    let mut empty = catalog(&[("R", vec![]), ("S", vec![vec![0, 1]])]);
    empty.relations.insert("S".into(), e.relations["S"].clone()); reseal(&mut empty);
    let (input, candidate) = fixture(&empty, &reads, Origin::Cq {});
    assert_eq!(whole(&input, &candidate, &empty), number(0));
}

#[test]
fn declared_keys_derive_degree_one_without_numeric_degree_facts() {
    let mut e = catalog(&[("R", vec![vec![0, 1], vec![0, 2]]), ("S", vec![vec![0, 5]])]);
    let relation = e.relations.get_mut("S").unwrap();
    relation.columns.get_mut("c0").unwrap().max_degree = None;
    relation.keys.push(ev::KeyDeclaration { columns: vec!["c0".into()], source: Source::Declared,
        dataset_hash: e.dataset.hash.clone(), provenance: relation.rows.as_ref().unwrap().provenance.clone() });
    relation.rows = None; reseal(&mut e);
    let (input, candidate) = fixture(&e, &[("R", &["x", "y"], Version::Full), ("S", &["x", "z"], Version::Full)], Origin::Cq {});
    let c = cost(&input, &candidate, &e);
    assert_eq!(c.steps[1].output.as_ref().unwrap().constant, rat(2));
    assert!(c.steps[1].inequality.contains("declared key S[c0]"));
}

#[test]
fn filters_probe_their_input_scalar_lets_preserve_rows_and_for_is_unknown() {
    let e = catalog(&[("R", vec![vec![0, 1], vec![1, 2]])]);
    let (input, mut candidate) = fixture(&e, &[("R", &["x", "y"], Version::Full)], Origin::Cq {});
    let step = CostStep { clause: 1, kind: ClauseKind::Constraint, read: None, required: vec!["x".into()],
        provides: vec![], bound_before: vec!["x".into(), "y".into()], introduces: false };
    candidate.steps.push(step.clone());
    candidate.steps.push(CostStep { clause: 2, kind: ClauseKind::OpaqueLet, provides: vec!["z".into()], introduces: true, ..step.clone() });
    let c = cost(&input, &candidate, &e);
    for step in &c.steps[1..] {
        assert_eq!(step.work.as_ref().unwrap().constant, rat(2));
        assert_eq!(step.output.as_ref().unwrap().constant, rat(2));
    }
    candidate.steps.push(CostStep { clause: 3, kind: ClauseKind::OpaqueFor, ..step });
    assert!(cost(&input, &candidate, &e).whole_run.is_none());
}

#[test]
fn seed_executes_once_delta_partition_amortizes_and_subset_does_not_claim_equality() {
    let mut e = catalog(&[("R", vec![vec![0, 1], vec![1, 2], vec![2, 3]])]);
    recursive(&mut e, "R", 7, true);
    let (input, candidate) = fixture(&e, &[("R", &["x", "y"], Version::Full)], rule(true));
    assert_eq!(whole(&input, &candidate, &e), number(3));
    let (input, candidate) = fixture(&e, &[("R", &["x", "y"], Version::Delta)], rule(false));
    let c = cost(&input, &candidate, &e);
    assert_eq!(c.per_round.as_ref().unwrap().coefficient, rat(1));
    assert_eq!(c.whole_run.as_ref().unwrap().terms["final_rows:R"], rat(1));
    assert!(!c.whole_run.as_ref().unwrap().terms.contains_key("rounds:s0"));
    assert_eq!(whole(&input, &candidate, &e), number(3));
    assert!(c.citations.contains("R.partition"));
    let p = e.premises.pop().unwrap();
    let ev::Premise::DeltaPartition { provenance, .. } = p else { unreachable!() };
    e.premises.push(ev::Premise::DeltaSubset { id: "R.subset".into(), relations: vec!["R".into()],
        source: Source::Declared, dataset_hash: e.dataset.hash.clone(), provenance });
    reseal(&mut e);
    let (input, candidate) = fixture(&e, &[("R", &["x", "y"], Version::Delta)], rule(false));
    let c = cost(&input, &candidate, &e);
    assert_eq!(whole(&input, &candidate, &e), number(3));
    assert!(c.steps[0].inequality.contains("consumed delta <= F"));
    assert!(!c.steps[0].inequality.contains("consumed delta = F"));
    e.premises.clear(); reseal(&mut e);
    let (input, candidate) = fixture(&e, &[("R", &["x", "y"], Version::Delta)], rule(false));
    assert!(cost(&input, &candidate, &e).whole_run.is_none());
}

#[test]
fn constant_work_is_charged_per_round_and_delta_indexes_per_consumed_row() {
    let mut e = catalog(&[("R", vec![vec![0, 1], vec![1, 2], vec![2, 3]]), ("S", vec![vec![0, 4], vec![1, 5]])]);
    recursive(&mut e, "R", 7, true);
    let (mut input, mut candidate) = fixture(&e, &[("S", &["x", "z"], Version::Full), ("R", &["x", "y"], Version::Delta)], rule(false));
    candidate.demands.insert(demand("R", IndexVersion::Delta, &[0]));
    input.layouts.push(layout("R", Version::Delta, &[0], "replace_each_run"));
    let c = cost(&input, &candidate, &e);
    let per_round = c.per_round.as_ref().unwrap();
    // Work certificate may choose a constant or delta-proportional join bound,
    // but it is a single valid affine and its complete sum includes h*delta.
    assert!(per_round.coefficient.exact().unwrap() >= number(1));
    let whole = c.whole_run.as_ref().unwrap();
    let expected = per_round.coefficient.exact().unwrap() * number(3) + per_round.constant.exact().unwrap() * number(7);
    assert_eq!(bounds::upper(whole, &e).unwrap().unwrap().exact().unwrap(), expected);
    assert!(whole.terms.contains_key("rounds:s0"));
}

#[test]
fn persistent_and_input_maintenance_is_once_and_shared_demands_are_marginal() {
    let mut e = catalog(&[("R", vec![vec![0, 1], vec![1, 2], vec![2, 3]]), ("S", vec![vec![0, 4], vec![1, 5]])]);
    recursive(&mut e, "R", 7, true);
    let (mut input, mut candidate) = fixture(&e, &[("R", &["x", "y"], Version::Full), ("S", &["x", "z"], Version::Full)], rule(true));
    input.layouts.push(layout("R", Version::Full, &[], "admitted_rows"));
    input.layouts.push(layout("S", Version::Full, &[0], "admitted_rows"));
    // New key inherits the recorded same-relation uniform maintenance policy.
    candidate.demands = BTreeSet::from([demand("R", IndexVersion::Full, &[0]), demand("S", IndexVersion::Full, &[0])]);
    let standalone = cost(&input, &candidate, &e);
    let shared = bounds::cost_candidate(&input, &input.holes[0], &candidate, &e, "cpu", &candidate.demands).unwrap();
    let a = bounds::upper(standalone.whole_run.as_ref().unwrap(), &e).unwrap().unwrap().exact().unwrap();
    let b = bounds::upper(shared.whole_run.as_ref().unwrap(), &e).unwrap().unwrap().exact().unwrap();
    assert_eq!(a - b, number(5)); // m*3 + h*2, not multiplied by seven rounds.
    input.layouts[0].maintenance = "unknown_policy".into();
    assert!(cost(&input, &candidate, &e).whole_run.is_none());
    input.layouts[0].maintenance = "replace_each_run".into();
    let rebuilt = cost(&input, &candidate, &e);
    assert_eq!(rebuilt.whole_run.as_ref().unwrap().terms["rounds:s0"], rat(3));
}

#[test]
fn huge_integer_products_are_exact_without_float_or_machine_overflow() {
    let mut e = catalog(&[("R", vec![vec![0, 0]]), ("S", vec![vec![0, 0]])]);
    for relation in e.relations.values_mut() {
        relation.rows.as_mut().unwrap().value = Some(u64::MAX);
        for column in relation.columns.values_mut() {
            column.distinct.as_mut().unwrap().value = Some(1);
            column.max_degree.as_mut().unwrap().value = Some(u64::MAX);
        }
    }
    reseal(&mut e);
    let (input, candidate) = fixture(&e, &[("R", &["x", "y"], Version::Full), ("S", &["x", "z"], Version::Full)], Origin::Cq {});
    let expected = number(u64::MAX) * number(u64::MAX);
    assert_eq!(whole(&input, &candidate, &e), expected);
}

#[test]
fn exact_interval_dominance_crossing_equality_and_different_final_relations() {
    let mut e = catalog(&[("R", vec![vec![0, 1], vec![1, 2]]), ("S", vec![vec![0, 1], vec![1, 2], vec![2, 3]])]);
    recursive(&mut e, "R", 2, true); recursive(&mut e, "S", 2, true);
    let r = linear(&[("final_rows:R", 1)], 0);
    let s = linear(&[("final_rows:S", 1)], 0);
    assert_eq!(bounds::compare(&r, &s, &e).unwrap(), Comparison::LeftDominates);
    assert_eq!(bounds::compare(&r, &constant(2), &e).unwrap(), Comparison::Equal);
    for relation in e.relations.values_mut() {
        relation.rows = None;
        for column in relation.columns.values_mut() { column.distinct = None; column.max_degree = None; }
        let f = &mut relation.recursive.as_mut().unwrap().final_rows;
        f.claim = Claim::Interval; f.value = None; f.lower = Some(0); f.upper = Some(10);
    }
    reseal(&mut e);
    assert_eq!(bounds::compare(&r, &constant(5), &e).unwrap(), Comparison::Crossing);
    assert_eq!(bounds::compare(&r, &s, &e).unwrap(), Comparison::Crossing);
    assert_eq!(bounds::compare(&r, &r, &e).unwrap(), Comparison::Equal);
    assert_eq!(bounds::compare(&r, &constant(10), &e).unwrap(), Comparison::LeftDominates);
    assert_eq!(bounds::compare(&constant(10), &r, &e).unwrap(), Comparison::RightDominates);
    assert_eq!(bounds::upper(&r, &e).unwrap(), Some(rat(10)));
    assert_eq!(bounds::upper(&linear(&[("final_rows:absent", 1)], 0), &e).unwrap(), None);
}

#[test]
fn relative_constraints_are_compared_on_the_feasible_domain_not_a_rectangle() {
    let mut e = catalog(&[("R", vec![vec![0, 1]]), ("S", vec![vec![0, 1]])]);
    for relation in e.relations.values_mut() {
        let f = relation.rows.as_mut().unwrap(); f.claim = Claim::Interval; f.value = None; f.lower = Some(0); f.upper = Some(100);
        for c in relation.columns.values_mut() { c.distinct = None; c.max_degree = None; }
    }
    let mut factor = e.targets["cpu"].build_per_row_h.clone(); factor.id = "relative.factor".into(); factor.unit = ev::Unit::Ratio;
    factor.value = Some(Rational { numerator: "1".into(), denominator: "3".into() });
    e.relative.push(ev::RelativeFact { id: "R.le.S.third".into(),
        left: ev::FactSubject { relation: "R".into(), measure: ev::RowMeasure::Rows }, op: ev::RelativeOp::LeScaled,
        right: ev::FactSubject { relation: "S".into(), measure: ev::RowMeasure::Rows }, factor,
        source: Source::Declared, dataset_hash: e.dataset.hash.clone(), provenance: e.relations["R"].rows.as_ref().unwrap().provenance.clone() });
    reseal(&mut e);
    assert_eq!(bounds::compare(&linear(&[("rows:R", 3)], 0), &linear(&[("rows:S", 1)], 0), &e).unwrap(), Comparison::LeftDominates);
    assert_eq!(bounds::upper(&linear(&[("rows:R", 3)], 0), &e).unwrap(), Some(rat(100)));
    // Relative-only cycles are not an absolute anchor.
    for relation in e.relations.values_mut() { relation.rows = None; }
    reseal(&mut e);
    assert_eq!(bounds::upper(&linear(&[("rows:R", 1)], 0), &e).unwrap(), None);
    assert_eq!(bounds::compare(&linear(&[("rows:R", 3)], 0), &linear(&[("rows:S", 1)], 0), &e).unwrap(), Comparison::LeftDominates);
}

#[test]
fn invalid_target_profile_scope_and_supplied_facts_are_errors_not_unknown() {
    let e = catalog(&[("R", vec![vec![0, 1]])]);
    let (input, candidate) = fixture(&e, &[("R", &["x", "y"], Version::Full)], Origin::Cq {});
    let model = BoundModel::new(&input, &e, "cpu").unwrap();
    assert_eq!(model.cost(&input.holes[0], &candidate, &BTreeSet::new()).unwrap(), cost(&input, &candidate, &e));
    assert_eq!(model.compare(&constant(1), &constant(1)).unwrap(), Comparison::Equal);
    assert_eq!(model.upper(&constant(1)).unwrap(), Some(rat(1)));
    assert!(BoundModel::new(&input, &e, "gpu").is_err());
    let mut invalid = e.clone(); invalid.targets.clear();
    assert!(BoundModel::new(&input, &invalid, "cpu").is_err());
    let mut invalid = e.clone(); invalid.targets.get_mut("cpu").unwrap().configuration_hash = export::sha256("different");
    assert!(BoundModel::new(&input, &invalid, "cpu").is_err());
    let mut invalid = e.clone(); invalid.targets.get_mut("cpu").unwrap().build_per_row_h.value.as_mut().unwrap().denominator = "0".into();
    assert!(BoundModel::new(&input, &invalid, "cpu").is_err());
    let mut invalid = e.clone(); invalid.relations.get_mut("R").unwrap().columns.get_mut("c0").unwrap().max_degree.as_mut().unwrap().value = Some(0);
    assert!(bounds::upper(&constant(0), &invalid).is_err());
    let mut recursive_e = e.clone(); recursive(&mut recursive_e, "R", 1, true);
    let (mut recursive_input, _) = fixture(&recursive_e, &[("R", &["x", "y"], Version::Delta)], rule(false));
    recursive_input.identity.program_hash = export::sha256("different program");
    assert!(BoundModel::new(&recursive_input, &recursive_e, "cpu").is_err());
}

#[test]
fn input_only_evidence_preserves_structural_delta_affine_but_not_whole_run() {
    let e = catalog(&[("EDB", (0..157).map(|x| vec![0, x]).collect())]);
    let (mut input, mut candidate) = fixture(&e, &[("Changing", &["x", "y"], Version::Delta),
        ("EDB", &["x", "z"], Version::Full), ("EDB", &["z", "w"], Version::Full)], rule(false));
    // No fabricated final relation or name-based alias: only EDB is attached.
    input.holes[0].evidence.as_mut().unwrap().bindings.remove(0);
    let c = cost(&input, &candidate, &e);
    assert!(c.per_round.is_some());
    assert!(c.whole_run.is_none());
    assert_eq!(c.steps[0].output.as_ref().unwrap().coefficient, rat(1));
    assert!(c.citations.contains("EDB.c0.degree"));
    let a = c.per_round.as_ref().unwrap().coefficient.exact().unwrap();
    candidate.demands.insert(demand("Changing", IndexVersion::Delta, &[0]));
    input.layouts.push(layout("Changing", Version::Delta, &[0], "replace_each_run"));
    let indexed = cost(&input, &candidate, &e);
    assert_eq!(indexed.per_round.as_ref().unwrap().coefficient.exact().unwrap(), a + number(1));
    assert!(indexed.whole_run.is_none());
}

#[test]
fn certified_disjoint_extrema_prove_zero_but_overlapping_extrema_do_not_filter() {
    let mut e = catalog(&[("R", vec![vec![0, 1], vec![1, 2]]), ("S", vec![vec![3, 0], vec![4, 1]])]);
    for (name, low, high) in [("R", 0, 1), ("S", 3, 4)] {
        let base = e.relations[name].rows.as_ref().unwrap();
        let scalar = |suffix: &str, value| ev::Fact {
            id: format!("{name}.c0.{suffix}"), source: Source::Exact, claim: Claim::Equal,
            value: Some(ev::Scalar::SignedInteger { value }), lower: None, upper: None,
            dataset_hash: e.dataset.hash.clone(), provenance: base.provenance.clone(), unit: ev::Unit::Scalar,
        };
        let min = scalar("min", low); let max = scalar("max", high);
        let column = e.relations.get_mut(name).unwrap().columns.get_mut("c0").unwrap();
        column.min = Some(min); column.max = Some(max);
    }
    reseal(&mut e);
    let reads = [("R", &["x", "y"][..], Version::Full), ("S", &["x", "z"][..], Version::Full)];
    let (input, candidate) = fixture(&e, &reads, Origin::Cq {});
    let c = cost(&input, &candidate, &e);
    assert_eq!(c.steps[1].output.as_ref().unwrap().constant, rat(0));
    assert!(c.steps[1].citations.contains("R.c0.max"));
    assert!(c.steps[1].citations.contains("S.c0.min"));
    let column = e.relations.get_mut("S").unwrap().columns.get_mut("c0").unwrap();
    column.min.as_mut().unwrap().value = Some(ev::Scalar::SignedInteger { value: 0 });
    reseal(&mut e);
    let (input, candidate) = fixture(&e, &reads, Origin::Cq {});
    assert_eq!(cost(&input, &candidate, &e).steps[1].output.as_ref().unwrap().constant, rat(2));
}

#[test]
fn selected_affine_is_valid_for_every_delta_size_not_a_coefficientwise_minimum() {
    let mut e = catalog(&[("R", (0..10).map(|x| vec![0, x]).collect()),
        ("S", vec![vec![0, 100], vec![0, 200]])]);
    recursive(&mut e, "R", 5, true);
    let (input, candidate) = fixture(&e, &[("S", &["x", "z"], Version::Full), ("R", &["x", "y"], Version::Delta)], rule(false));
    let c = cost(&input, &candidate, &e);
    let affine = c.per_round.as_ref().unwrap();
    for delta in 0..=10 {
        let declared = affine.coefficient.exact().unwrap() * number(delta) + affine.constant.exact().unwrap();
        let exact_work = number(2 + 2 * delta); // S root scan plus the exact join.
        assert!(declared >= exact_work);
    }
}

#[test]
fn declared_bound_and_completed_calibrated_upper_claims_are_safe_not_observations() {
    let mut e = catalog(&[("R", vec![vec![0, 1], vec![0, 2]]), ("S", vec![vec![0, 3], vec![0, 4]])]);
    let rows = e.relations.get_mut("S").unwrap().rows.as_mut().unwrap();
    rows.source = Source::Bound; rows.claim = Claim::Upper; rows.value = Some(100);
    for source in [Source::Bound, Source::Declared, Source::Calibrated] {
        let degree = e.relations.get_mut("S").unwrap().columns.get_mut("c0").unwrap().max_degree.as_mut().unwrap();
        degree.source = source; degree.claim = Claim::Upper; degree.value = Some(3);
        degree.provenance.run = (source == Source::Calibrated).then(|| ev::RunIdentity {
            program_hash: HASH.into(), configuration_hash: HASH.into(), admission: ev::Admission::Bag,
            run_id: "completed-bag-count".into(), completed: true,
        });
        reseal(&mut e);
        let (input, candidate) = fixture(&e, &[("R", &["x", "y"], Version::Full), ("S", &["x", "z"], Version::Full)], Origin::Cq {});
        let c = cost(&input, &candidate, &e);
        assert_eq!(c.steps[1].output.as_ref().unwrap().constant, rat(6));
        assert!(c.steps[1].citations.contains("S.c0.degree"));
    }
}

#[test]
fn support_distinct_cap_is_a_multiplicity_bound_not_division_by_distinct() {
    let rows = vec![vec![0, 0], vec![0, 1], vec![1, 0], vec![1, 1]];
    let mut e = catalog(&[("R", rows.clone()), ("S", rows)]);
    for relation in e.relations.values_mut() {
        let rows = relation.rows.as_mut().unwrap();
        rows.source = Source::Bound; rows.claim = Claim::Upper; rows.value = Some(100);
        for column in relation.columns.values_mut() {
            let degree = column.max_degree.as_mut().unwrap();
            degree.source = Source::Bound; degree.claim = Claim::Upper; degree.value = Some(3);
        }
    }
    reseal(&mut e);
    let (input, candidate) = fixture(&e, &[("R", &["x", "y"], Version::Full), ("S", &["x", "z"], Version::Full)], Origin::Cq {});
    let c = cost(&input, &candidate, &e);
    assert_eq!(c.steps[1].output.as_ref().unwrap().constant, rat(18));
    assert!(c.steps[1].inequality.contains("support-distinct cap"));
    assert!(c.steps[1].citations.iter().any(|id| id.ends_with(".distinct")));
}

#[test]
fn absent_binding_is_unknown_but_invalid_explicit_binding_is_an_error() {
    let e = catalog(&[("R", vec![vec![0, 1]])]);
    let (mut input, candidate) = fixture(&e, &[("R", &["x", "y"], Version::Full)], Origin::Cq {});
    input.holes[0].evidence.as_mut().unwrap().bindings[0].dataset_relation = "r".into();
    assert!(bounds::cost_candidate(&input, &input.holes[0], &candidate, &e, "cpu", &BTreeSet::new()).is_err());
    input.holes[0].evidence.as_mut().unwrap().bindings[0].dataset_relation = "R".into();
    input.holes[0].evidence.as_mut().unwrap().bindings[0].columns[0].dataset_column = "C0".into();
    assert!(bounds::cost_candidate(&input, &input.holes[0], &candidate, &e, "cpu", &BTreeSet::new()).is_err());
}
