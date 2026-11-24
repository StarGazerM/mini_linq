use cq_lang_datalog_round::{self as round, incremental::strata::source, storage};
use cq_syntax::declared_evidence::EvidenceView;
use quote::{ToTokens, quote};

// Local fixtures mirror runtime/cq-evidence/tests/fixtures for external consumers.
const INPUT: &str = include_str!("fixtures/planner-evidence.json");
const RECURSIVE: &str = include_str!("fixtures/planner-recursive-evidence.json");

fn source(evidence: Option<&str>) -> source::Program {
    let attachment = evidence.map(|json| quote!(
        declared_evidence #json { R = "R" full [a="a",b="b"]; }
    ));
    syn::parse2(quote! {
        struct Reach;
        relation E(a:i32,b:i32) = (edges);
        relation R(a:i32,b:i32);
        #attachment
        output R;
        Seed: R(*a,*b) :- E(a,b);
        Step: R(*a,*c) :- R(a,b), E(b,c);
    }).unwrap()
}
fn compile(program: &source::Program) -> (storage::Program, round::Program) {
    let strata = cq_datalog_stratify::compile(program).unwrap();
    let incremental = cq_datalog_incremental::compile(&strata).unwrap();
    let storage = cq_datalog_storage::compile(&incremental).unwrap();
    let round = cq_datalog_fork::compile(&storage).unwrap();
    cq_datalog_fork::check(&storage, &round).unwrap();
    (storage, round)
}

#[test]
fn source_scope_is_checked_without_promoting_input_counts_to_final_counts() {
    assert!(source::contract::check(&source(Some(INPUT))).is_err());
    source::contract::check(&source(Some(RECURSIVE))).unwrap();
}

#[test]
fn dataset_metadata_reaches_checked_delta_views_without_changing_decisions() {
    let (storage, attached) = compile(&source(Some(RECURSIVE)));
    let (_, plain) = compile(&source(None));
    assert_eq!(attached.context, plain.context);
    assert_eq!(attached.holes.len(), plain.holes.len());
    let mut delta_seen = false;
    for (with, without) in attached.holes.iter().zip(&plain.holes) {
        assert_eq!(with.cq.query, without.cq.query);
        assert_eq!(with.cq.declarations, without.cq.declarations);
        assert_eq!(with.planning, without.planning);
        let facts = with.cq.declared_evidence.as_ref().unwrap();
        facts.check(&with.cq.declarations).unwrap();
        delta_seen |= facts.bindings.iter().any(|binding| binding.view == EvidenceView::Delta);
    }
    assert!(delta_seen);
    let restored: round::Program = syn::parse2(attached.to_token_stream()).unwrap();
    assert_eq!(restored, attached);
    cq_datalog_fork::check(&storage, &restored).unwrap();
}

#[test]
fn fork_rejects_lost_or_relabelled_evidence() {
    let (storage, target) = compile(&source(Some(RECURSIVE)));
    let index = target.holes.iter().position(|hole| {
        hole.cq.declared_evidence.as_ref().unwrap().bindings.iter()
            .any(|binding| binding.view == EvidenceView::Delta)
    }).unwrap();
    let mut lost = target.clone();
    lost.holes[index].cq.declared_evidence = None;
    assert!(cq_datalog_fork::check(&storage, &lost).is_err());
    let mut relabelled = target.clone();
    for binding in &mut relabelled.holes[index].cq.declared_evidence.as_mut().unwrap().bindings {
        if binding.view == EvidenceView::Delta { binding.view = EvidenceView::Full; }
    }
    assert!(cq_datalog_fork::check(&storage, &relabelled).is_err());
    let mut wrong_column = target.clone();
    wrong_column.holes[index].cq.declared_evidence.as_mut().unwrap().bindings[0].columns[0].source = "missing".into();
    assert!(cq_datalog_fork::check(&storage, &wrong_column).is_err());
}

#[test]
fn source_rewrite_cannot_discard_the_recorded_input() {
    let before = source(Some(RECURSIVE));
    let mut after = before.clone();
    after.declared_evidence = None;
    assert!(source::check_rewrite(&before, &after).is_err());
}

fn coalesced_source(json: &str) -> source::Program {
    syn::parse2(quote! {
        struct Coalesced;
        relation E(a:i32,b:i32) = (edges);
        relation R(a:i32,b:i32);
        relation S(a:i32,b:i32);
        declared_evidence #json { S = "R" full [a="a",b="b"]; }
        output S;
        Seed: R(*a,*b) :- E(a,b);
        Step: R(*a,*c) :- R(a,b), E(b,c);
        Copy: S(*a,*b) :- R(a,b);
    }).unwrap()
}
fn round_result(program: &source::Program, merge: bool) -> syn::Result<round::Program> {
    let mut strata = cq_datalog_stratify::compile(program)?;
    if merge { strata.components = vec![vec![syn::parse_quote!(R), syn::parse_quote!(S)]]; }
    let incremental = cq_datalog_incremental::compile(&strata)?;
    let storage = cq_datalog_storage::compile(&incremental)?;
    let result = cq_datalog_fork::compile(&storage)?;
    round::contract::check(&result)?;
    Ok(result)
}

#[test]
fn evidence_iteration_scope_is_owned_by_the_actual_round_component() {
    let program = coalesced_source(RECURSIVE);
    // S is not intrinsically recursive: Source cannot decide its later grouping.
    source::contract::check(&program).unwrap();
    round_result(&program, true).unwrap();
    assert!(round_result(&program, false).is_err(), "seed-only S must be Frozen");

    let mut document: serde_json::Value = serde_json::from_str(RECURSIVE).unwrap();
    document["relations"]["R"]["recursive"]["stratum"] = serde_json::json!("1");
    for series in document["observations"]["rounds"].as_array_mut().unwrap() {
        series["stratum"] = serde_json::json!("1");
    }
    let json = serde_json::to_string(&document).unwrap();
    assert!(round_result(&coalesced_source(&json), true).is_err(), "file cannot invent its component index");

    document["relations"]["R"]["scope"] = serde_json::json!("frozen");
    document["relations"]["R"]["recursive"] = serde_json::Value::Null;
    document["premises"] = serde_json::json!([]);
    document["observations"]["rounds"] = serde_json::json!([]);
    document["dataset"]["manifest"]["schema"]["R"]["set_certification"] = serde_json::json!("full_tuple_deduplicated");
    let manifest: cq_evidence::InputManifest = serde_json::from_value(document["dataset"]["manifest"].clone()).unwrap();
    let hash = manifest.dataset_hash().unwrap();
    document["dataset"]["hash"] = serde_json::json!(hash);
    document["dataset"]["schema_hash"] = serde_json::json!(manifest.schema_hash().unwrap());
    fn restamp(value: &mut serde_json::Value, hash: &str) {
        match value {
            serde_json::Value::Object(object) => {
                if let Some(value) = object.get_mut("dataset_hash") { *value = serde_json::json!(hash); }
                for value in object.values_mut() { restamp(value, hash); }
            }
            serde_json::Value::Array(array) => for value in array { restamp(value, hash); },
            _ => {}
        }
    }
    restamp(&mut document, &hash);
    let json = serde_json::to_string(&document).unwrap();
    let frozen = coalesced_source(&json);
    source::contract::check(&frozen).unwrap();
    round_result(&frozen, false).unwrap();
    assert!(round_result(&frozen, true).is_err(), "coalesced changing S cannot claim Frozen");
    assert!(source::contract::check(&source(Some(&json))).is_err(), "intrinsically recursive R cannot claim Frozen");

    let mut edb = source(Some(RECURSIVE));
    edb.declared_evidence.as_mut().unwrap().bindings[0].relation = syn::parse_quote!(E);
    assert!(source::contract::check(&edb).is_err(), "EDB cannot claim RecursiveFinal");
}
