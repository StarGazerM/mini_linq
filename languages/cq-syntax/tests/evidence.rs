//! Tiny authored fixtures exercise document checking, not workload measurements.
use cq_syntax::evidence::{DatasetEvidence, InputManifest, parse_bytes, parse_json};
use serde_json::{Value, json};

const FIXTURE: &str = include_str!("fixtures/planner-evidence.json");
const HASH: &str = "sha256:1111111111111111111111111111111111111111111111111111111111111111";
fn fixture() -> Value { serde_json::from_str(FIXTURE).unwrap() }
fn accepted(value: &Value) -> DatasetEvidence {
    parse_json(&serde_json::to_string(value).unwrap()).unwrap()
}
fn rejected(value: &Value) {
    assert!(parse_json(&serde_json::to_string(value).unwrap()).is_err(), "accepted invalid evidence: {value}");
}
fn replace(value: &mut Value, path: &str, replacement: Value) {
    *value.pointer_mut(path).unwrap() = replacement;
}
fn restamp(value: &mut Value, hash: &str) {
    match value {
        Value::Object(map) => {
            if let Some(v) = map.get_mut("dataset_hash") { *v = json!(hash); }
            for v in map.values_mut() { restamp(v, hash); }
        }
        Value::Array(a) => for v in a { restamp(v, hash); },
        _ => {}
    }
}
fn rehash(value: &mut Value) {
    let manifest: InputManifest = serde_json::from_value(value["dataset"]["manifest"].clone()).unwrap();
    let hash = manifest.dataset_hash().unwrap();
    value["dataset"]["hash"] = json!(hash);
    value["dataset"]["schema_hash"] = json!(manifest.schema_hash().unwrap());
    restamp(value, &hash);
}
fn count(value: &Value, id: &str, unit: &str, n: u64) -> Value {
    let mut f = value["relations"]["R"]["rows"].clone();
    f["id"] = json!(id); f["unit"] = json!(unit); f["value"] = json!(n);
    f
}
fn rational(value: &Value, id: &str, unit: &str, n: &str, d: &str) -> Value {
    let mut f = count(value, id, unit, 0);
    f["source"] = json!("declared");
    f["value"] = json!({"numerator": n, "denominator": d});
    f
}
fn recursive() -> Value {
    let mut v = fixture();
    let run = json!({"program_hash": HASH, "configuration_hash": HASH, "admission":"insert_only_set", "run_id":"fixture-run", "completed":true});
    v["dataset"]["manifest"]["schema"]["R"]["semantics"] = json!("set");
    v["relations"]["R"]["semantics"] = json!("set");
    v["relations"]["R"]["scope"] = json!("recursive_final");
    fn set_run(v: &mut Value, run: &Value) {
        match v {
            Value::Object(map) => {
                if let Some(p) = map.get_mut("provenance") { p["run"] = run.clone(); }
                for child in map.values_mut() { set_run(child, run); }
            }
            Value::Array(a) => for child in a { set_run(child, run); },
            _ => {}
        }
    }
    set_run(&mut v, &run);
    v["relations"]["R"]["recursive"] = json!({
        "stratum":"s0", "run":run, "round_convention":"executed-steady-rounds-excluding-seed-and-terminal-empty",
        "final_rows":count(&v,"R.final","rows",4), "rounds":count(&v,"s0.rounds","rounds",2),
        "seed_rows":count(&v,"R.seed","rows",1), "unconsumed_final_delta_rows":count(&v,"R.unconsumed","rows",0)
    });
    v["premises"] = json!([{
        "id":"R.partition", "kind":"delta_partition", "relation":"R", "includes_seed_delta":true,
        "insert_only_set":true, "complete_consumption":true, "source":"declared",
        "dataset_hash":v["dataset"]["hash"], "provenance":v["relations"]["R"]["rows"]["provenance"]
    }]);
    v["observations"]["rounds"] = json!([{
        "relation":"R", "stratum":"s0", "run":run, "complete":true,
        "entries":[
            {"round":count(&v,"round.1","rounds",1), "delta_rows":count(&v,"delta.1","rows",1)},
            {"round":count(&v,"round.2","rounds",2), "delta_rows":count(&v,"delta.2","rows",3)}
        ]
    }]);
    rehash(&mut v);
    v
}

#[test]
fn canonical_roundtrip_preserves_every_envelope() {
    let parsed = parse_json(FIXTURE).unwrap();
    parsed.check().unwrap();
    let canonical = parsed.canonical_json().unwrap();
    assert_eq!(parse_json(&canonical).unwrap(), parsed);
    assert_eq!(parse_json(&canonical).unwrap().canonical_json().unwrap(), canonical);
    assert!(canonical.starts_with("{\"dataset\":"));
    assert_eq!(parsed.relations["R"].rows.as_ref().unwrap().value, Some(4));
    let shuffled = serde_json::to_string_pretty(&fixture()).unwrap();
    assert_eq!(parse_json(&shuffled).unwrap().canonical_json().unwrap(), canonical);
}

#[test]
fn semantic_hash_mode_is_mandatory_even_when_the_manifest_digest_matches() {
    for mode in [None, Some("raw-file-sha256"), Some("typed-row-stream-v2")] {
        let mut value = fixture();
        if let Some(mode) = mode {
            value["dataset"]["manifest"]["loader_options"]["hash_mode"] = json!(mode);
        } else {
            value["dataset"]["manifest"]["loader_options"].as_object_mut().unwrap().remove("hash_mode");
        }
        rehash(&mut value);
        let error = parse_json(&value.to_string()).unwrap_err();
        assert!(error.to_string().contains("hash mode"), "{error}");
    }
}

#[test]
fn unit_scalar_domains_reject_unknown_fields_before_semantic_checking() {
    for kind in ["signed_integer", "utf8"] {
        assert!(serde_json::from_value::<cq_syntax::evidence::ScalarDomain>(
            json!({"kind": kind, "unexpected": true})
        ).is_err());
    }
}

#[test]
fn rejects_invalid_json_utf8_duplicates_unknown_fields_and_versions() {
    for invalid in ["", "null", "[]", "{", "{}", "{} {}"] { assert!(parse_json(invalid).is_err()); }
    assert!(parse_bytes(&[0xff, 0xfe]).is_err());
    let compact = serde_json::to_string(&fixture()).unwrap();
    let duplicate = compact.replacen("\"schema\":\"mini-linq.planner-evidence.v1\"", "\"schema\":\"mini-linq.planner-evidence.v1\",\"schema\":\"mini-linq.planner-evidence.v1\"", 1);
    assert!(parse_json(&duplicate).is_err());
    let nested = compact.replacen("\"rows\":{", "\"rows\":null,\"rows\":{", 1);
    assert!(parse_json(&nested).is_err());
    let escaped = compact.replacen("\"producer\":", "\"producer\":\"duplicate\",\"prod\\u0075cer\":", 1);
    assert!(parse_json(&escaped).is_err());
    for path in ["", "/dataset", "/dataset/manifest", "/dataset/manifest/files/R.csv", "/relations/R", "/relations/R/columns/a", "/relations/R/columns/a/domain", "/relations/R/rows", "/relations/R/rows/provenance"] {
        let mut v = fixture(); v.pointer_mut(path).unwrap()["unexpected"] = json!(1); rejected(&v);
    }
    let mut v = fixture(); v["schema"] = json!("mini-linq.planner-evidence.v2"); rejected(&v);
}

#[test]
fn rejects_identity_and_envelope_mutations() {
    let mutations = [
        ("/dataset/hash", json!(HASH)),
        ("/dataset/schema_hash", json!(HASH)),
        ("/dataset/date", json!("2025-02-29")),
        ("/dataset/date", json!("2026-00-01")),
        ("/dataset/name", json!(" ")),
        ("/dataset/formulation", json!("different-query")),
        ("/relations/R/rows/dataset_hash", json!(HASH)),
        ("/relations/R/rows/source", json!("sampled")),
        ("/relations/R/rows/claim", json!("approximate")),
        ("/relations/R/rows/unit", json!("distinct")),
        ("/relations/R/rows/provenance/producer", json!("")),
        ("/relations/R/rows/provenance/method", json!(" \t")),
        ("/relations/R/rows/provenance/locator", json!("")),
        ("/relations/R/rows/provenance/artifact_hash", json!("sha256:ABC")),
        ("/relations/R/rows/id", json!("R.a.distinct")),
        ("/relations/R/rows/id", json!("")),
        ("/relations/R/rows/value", json!(-1)),
        ("/relations/R/rows/value", json!(1.5)),
        ("/relations/R/rows/value", json!("4")),
        ("/relations/R/rows/value", Value::Null),
        ("/relations/R/rows/lower", json!(0)),
        ("/relations/R/rows/claim", json!("interval")),
        ("/relations/R/rows/source", json!("bound")),
        ("/relations/R/rows/source", json!("calibrated")),
        ("/relations/R/rows", json!(4)),
    ];
    for (path, replacement) in mutations {
        let mut v = fixture(); replace(&mut v, path, replacement); rejected(&v);
    }
    let json = serde_json::to_string(&fixture()).unwrap();
    assert!(parse_json(&json.replacen("\"value\":4", "\"value\":18446744073709551616", 1)).is_err());
    assert!(parse_json(&json.replacen("\"value\":4", "\"value\":4.0", 1)).is_err());
    assert!(parse_json(&json.replacen("\"value\":4", "\"value\":4e0", 1)).is_err());
}

#[test]
fn manifest_is_content_bound_and_relative_not_a_path_or_profile_hash() {
    let mut v = fixture();
    v["dataset"]["manifest"]["files"]["R.csv"]["content_hash"] = json!(HASH);
    rejected(&v);
    rehash(&mut v); accepted(&v);
    for name in ["/absolute.csv", "../escape.csv", "a/../b", "./R.csv", "a//b", "C:\\R.csv"] {
        let mut v = fixture();
        let entry = v["dataset"]["manifest"]["files"].as_object_mut().unwrap().remove("R.csv").unwrap();
        v["dataset"]["manifest"]["files"][name] = entry;
        rehash(&mut v); rejected(&v);
    }
    let mut v = fixture();
    v["dataset"]["manifest"]["files"]["R.csv"]["content_hash"] = Value::Null;
    v["dataset"]["manifest"]["files"]["R.csv"]["missing_as_empty"] = json!(true);
    rehash(&mut v); rejected(&v);
    v["dataset"]["manifest"]["loader_options"]["missing_files"] = json!("empty");
    rehash(&mut v); accepted(&v);
    v["dataset"]["manifest"]["files"]["R.csv"]["role"] = json!("metadata");
    rehash(&mut v); rejected(&v);
}

#[test]
fn missing_is_unknown_and_empty_has_no_extrema() {
    let mut unknown = fixture();
    unknown["relations"]["R"]["rows"] = Value::Null;
    for column in ["a", "b"] {
        for field in ["distinct", "max_degree", "min", "max"] { unknown["relations"]["R"]["columns"][column][field] = Value::Null; }
    }
    accepted(&unknown);
    let mut empty = fixture(); empty["relations"]["R"]["rows"]["value"] = json!(0);
    for column in ["a", "b"] {
        empty["relations"]["R"]["columns"][column]["distinct"]["value"] = json!(0);
        empty["relations"]["R"]["columns"][column]["max_degree"]["value"] = json!(0);
        empty["relations"]["R"]["columns"][column]["min"] = Value::Null;
        empty["relations"]["R"]["columns"][column]["max"] = Value::Null;
    }
    accepted(&empty);
    empty["relations"]["R"]["columns"]["b"]["min"] = fixture()["relations"]["R"]["columns"]["b"]["min"].clone();
    rejected(&empty);
}

#[test]
fn cardinality_constraints_use_feasible_intervals_not_loose_upper_endpoints() {
    for (path, n) in [
        ("/relations/R/columns/a/distinct/value", 0),
        ("/relations/R/columns/a/distinct/value", 5),
        ("/relations/R/columns/a/max_degree/value", 0),
        ("/relations/R/columns/a/max_degree/value", 5),
        ("/relations/R/columns/a/max_degree/value", 1),
        ("/relations/R/columns/a/max_degree/value", 4),
    ] {
        let mut v = fixture(); replace(&mut v, path, json!(n)); rejected(&v);
    }
    let mut v = fixture();
    v["relations"]["R"]["rows"]["claim"] = json!("upper");
    v["relations"]["R"]["rows"]["value"] = json!(100);
    accepted(&v);
    v["relations"]["R"]["columns"]["a"]["distinct"]["claim"] = json!("upper");
    v["relations"]["R"]["columns"]["a"]["distinct"]["value"] = json!(90);
    accepted(&v);
    v["relations"]["R"]["rows"]["claim"] = json!("interval");
    v["relations"]["R"]["rows"]["value"] = Value::Null;
    v["relations"]["R"]["rows"]["lower"] = json!(5);
    v["relations"]["R"]["rows"]["upper"] = json!(100);
    rejected(&v); // b has exactly four distinct, degree one.
    v["relations"]["R"]["rows"]["lower"] = json!(101); rejected(&v);
}

#[test]
fn exact_u64_counts_and_products_do_not_wrap_or_round() {
    let mut v = fixture();
    v["relations"]["R"]["rows"]["value"] = json!(u64::MAX);
    for name in ["a", "b"] {
        let c = &mut v["relations"]["R"]["columns"][name];
        c["distinct"]["claim"] = json!("upper"); c["distinct"]["value"] = json!(u64::MAX);
        c["max_degree"]["claim"] = json!("upper"); c["max_degree"]["value"] = json!(u64::MAX);
        c["min"] = Value::Null; c["max"] = Value::Null;
    }
    let parsed = accepted(&v);
    assert_eq!(parsed.relations["R"].rows.as_ref().unwrap().value, Some(u64::MAX));
    assert!(parsed.canonical_json().unwrap().contains("18446744073709551615"));
}

#[test]
fn hll_observations_never_become_bounds_or_keys() {
    let mut v = fixture();
    v["relations"]["R"]["columns"]["a"]["distinct"]["source"] = json!("hll");
    rejected(&v);
    v["relations"]["R"]["columns"]["a"]["distinct"]["claim"] = json!("observation");
    v["relations"]["R"]["columns"]["a"]["distinct"]["value"] = json!(1000);
    accepted(&v); // Observation is not compared to four exact rows.
    v["relations"]["R"]["columns"]["a"]["distinct"]["claim"] = json!("upper"); rejected(&v);
    let mut v = fixture();
    v["relations"]["R"]["keys"] = json!([{"columns":["a"], "source":"hll", "dataset_hash":v["dataset"]["hash"], "provenance":v["relations"]["R"]["rows"]["provenance"]}]);
    rejected(&v);
    v["relations"]["R"]["keys"][0]["source"] = json!("declared"); rejected(&v); // a has degree two.
}

#[test]
fn composite_keys_do_not_make_each_member_unique() {
    let mut v = fixture();
    v["relations"]["R"]["keys"] = json!([{"columns":["a","b"], "source":"declared", "dataset_hash":v["dataset"]["hash"], "provenance":v["relations"]["R"]["rows"]["provenance"]}]);
    v["relations"]["R"]["key_sets"] = json!([{"columns":["a","b"], "distinct":count(&v,"R.ab.distinct","distinct",4), "max_degree":count(&v,"R.ab.degree","degree",1)}]);
    accepted(&v);
    for columns in [json!([]), json!(["a","a"]), json!(["missing"]), json!(["a"])] {
        let mut invalid = v.clone(); invalid["relations"]["R"]["keys"][0]["columns"] = columns; rejected(&invalid);
    }
    let mut invalid = v.clone();
    let key = invalid["relations"]["R"]["keys"][0].clone();
    invalid["relations"]["R"]["keys"].as_array_mut().unwrap().push(key); rejected(&invalid);
    let mut invalid = v.clone(); invalid["relations"]["R"]["key_sets"][0]["max_degree"]["value"] = json!(2); rejected(&invalid);
    let mut invalid = v.clone();
    invalid["relations"]["R"]["key_sets"][0]["columns"] = json!(["a"]);
    rejected(&invalid); // Conflicting singleton restatement.
}

#[test]
fn set_statistics_require_schema_identity_and_dedup_certification() {
    let mut v = fixture(); v["relations"]["R"]["semantics"] = json!("set"); rejected(&v);
    v["dataset"]["manifest"]["schema"]["R"]["semantics"] = json!("set"); rehash(&mut v); rejected(&v);
    v["dataset"]["manifest"]["schema"]["R"]["set_certification"] = json!("full_tuple_deduplicated");
    rehash(&mut v); accepted(&v);
    let mut v = fixture();
    v["relations"]["R"]["semantics"] = json!("set");
    v["dataset"]["manifest"]["schema"]["R"]["semantics"] = json!("set");
    v["relations"]["R"]["rows"]["claim"] = json!("upper");
    for column in ["a", "b"] {
        for field in ["distinct", "max_degree"] { v["relations"]["R"]["columns"][column][field]["claim"] = json!("upper"); }
    }
    rehash(&mut v); accepted(&v);
}

#[test]
fn all_extremum_encodings_are_typed_and_ordered() {
    let domains_and_values = [
        (json!({"kind":"signed_integer"}), json!({"kind":"signed_integer","value":-3}), json!({"kind":"signed_integer","value":4})),
        (json!({"kind":"decimal","scale":2}), json!({"kind":"decimal","unscaled":"-9999999999999999999999999999999999999999","scale":2}), json!({"kind":"decimal","unscaled":"400","scale":2})),
        (json!({"kind":"date","encoding":"days-since-1970"}), json!({"kind":"date","value":-30,"encoding":"days-since-1970"}), json!({"kind":"date","value":400,"encoding":"days-since-1970"})),
        (json!({"kind":"utf8"}), json!({"kind":"utf8","value":"alpha"}), json!({"kind":"utf8","value":"zeta"})),
        (json!({"kind":"symbol","dictionary_hash":HASH,"ordered":true}), json!({"kind":"symbol","code":1,"dictionary_hash":HASH}), json!({"kind":"symbol","code":9,"dictionary_hash":HASH})),
    ];
    for (domain, lo, hi) in domains_and_values {
        let mut v = fixture();
        v["dataset"]["manifest"]["dictionaries"]["symbols"] = json!(HASH);
        v["dataset"]["manifest"]["schema"]["R"]["columns"]["b"] = domain.clone();
        v["relations"]["R"]["columns"]["b"]["domain"] = domain;
        v["relations"]["R"]["columns"]["b"]["min"]["value"] = lo.clone();
        v["relations"]["R"]["columns"]["b"]["max"]["value"] = hi.clone();
        rehash(&mut v); accepted(&v);
        v["relations"]["R"]["columns"]["b"]["min"]["value"] = hi;
        v["relations"]["R"]["columns"]["b"]["max"]["value"] = lo;
        rejected(&v);
    }
    let mut v = fixture();
    v["relations"]["R"]["columns"]["b"]["min"]["value"] = json!({"kind":"decimal","unscaled":"1","scale":2}); rejected(&v);
    v["relations"]["R"]["columns"]["b"]["min"]["value"] = json!({"kind":"signed_integer","value":4}); rejected(&v); // Four distinct values cannot share extrema.
    let mut v = fixture();
    v["relations"]["R"]["columns"]["b"]["distinct"]["value"] = json!(1);
    v["relations"]["R"]["columns"]["b"]["max_degree"]["value"] = json!(4);
    rejected(&v);
    v["relations"]["R"]["columns"]["b"]["max"]["value"]["value"] = json!(1); accepted(&v);
}

#[test]
fn unordered_domains_record_limitations_without_extrema() {
    for domain in [json!({"kind":"unordered","limitation":"NaN has no total order"}), json!({"kind":"symbol","dictionary_hash":HASH,"ordered":false})] {
        let mut v = fixture();
        v["dataset"]["manifest"]["dictionaries"]["symbols"] = json!(HASH);
        v["dataset"]["manifest"]["schema"]["R"]["columns"]["b"] = domain.clone();
        v["relations"]["R"]["columns"]["b"]["domain"] = domain;
        rehash(&mut v); rejected(&v);
        v["relations"]["R"]["columns"]["b"]["min"] = Value::Null;
        v["relations"]["R"]["columns"]["b"]["max"] = Value::Null;
        accepted(&v);
    }
}

#[test]
fn target_constants_use_exact_nonnegative_rationals_without_defaults() {
    let mut v = fixture();
    v["targets"]["cpu"] = json!({"unit":"declared-row-work", "configuration_hash":HASH,
        "build_per_row_h":rational(&v,"cpu.h","declared-row-work","999999999999999999999999999999999999999999999999999999","3"),
        "merge_per_row_m":rational(&v,"cpu.m","declared-row-work","0","1")});
    accepted(&v);
    for (field, invalid) in [("numerator", json!("-1")), ("numerator", json!("1.5")), ("numerator", json!("01")), ("numerator", json!(1)), ("denominator", json!("0")), ("denominator", json!("-1"))] {
        let mut invalid_doc = v.clone(); invalid_doc["targets"]["cpu"]["build_per_row_h"]["value"][field] = invalid; rejected(&invalid_doc);
    }
    let mut invalid = v.clone(); invalid["targets"]["cpu"]["build_per_row_h"]["claim"] = json!("observation"); rejected(&invalid);
    let mut invalid = v.clone(); invalid["targets"]["cpu"]["merge_per_row_m"] = Value::Null; rejected(&invalid);
}

fn relative(value: &Value, id: &str, left: &str, right: &str, n: &str, d: &str) -> Value {
    json!({"id":id, "left":{"relation":left,"measure":"rows"}, "op":"le_scaled", "right":{"relation":right,"measure":"rows"},
        "factor":rational(value,&format!("{id}.factor"),"ratio",n,d), "source":"declared", "dataset_hash":value["dataset"]["hash"], "provenance":value["relations"]["R"]["rows"]["provenance"]})
}
#[test]
fn relative_cycles_are_exact_and_do_not_invent_absolute_facts() {
    let mut v = fixture();
    v["relative"] = json!([relative(&v,"self","R","R","1","1")]); accepted(&v);
    v["relative"][0]["factor"]["value"]["numerator"] = json!("0"); rejected(&v);
    v["relative"][0]["factor"]["value"] = json!({"numerator":"1","denominator":"2"}); rejected(&v);
    v["relations"]["R"]["rows"] = Value::Null;
    for name in ["a","b"] { for field in ["distinct","max_degree","min","max"] { v["relations"]["R"]["columns"][name][field] = Value::Null; } }
    accepted(&v); // x <= x/2 implies zero, but relative-only data is not an estimate.
    v["relative"][0]["right"]["relation"] = json!("missing"); rejected(&v);
    v["relative"][0]["right"]["relation"] = json!("R");
    v["relative"][0]["right"]["measure"] = json!("final_rows"); rejected(&v);
}

#[test]
fn recursive_partitions_require_complete_admission_consumption_and_run_scope() {
    let v = recursive(); accepted(&v);
    let mutations = [
        ("/relations/R/scope", json!("input")),
        ("/relations/R/recursive/round_convention", json!("including-terminal-empty")),
        ("/relations/R/recursive/run/completed", json!(false)),
        ("/relations/R/recursive/run/admission", json!("replacement")),
        ("/relations/R/recursive/rounds/provenance/run/run_id", json!("other")),
        ("/relations/R/recursive/rounds/value", json!(1)),
        ("/relations/R/recursive/final_rows/value", json!(3)),
        ("/relations/R/recursive/seed_rows/value", json!(5)),
        ("/relations/R/recursive/unconsumed_final_delta_rows/value", json!(1)),
        ("/premises/0/includes_seed_delta", json!(false)),
        ("/premises/0/insert_only_set", json!(false)),
        ("/premises/0/complete_consumption", json!(false)),
        ("/premises/0/relation", json!("unknown")),
        ("/premises/0/source", json!("hll")),
        ("/observations/rounds/0/stratum", json!("other")),
        ("/observations/rounds/0/run/run_id", json!("other")),
        ("/observations/rounds/0/entries/1/round/value", json!(1)),
        ("/observations/rounds/0/entries/1/delta_rows/value", json!(4)),
        ("/observations/rounds/0/entries/1/delta_rows/value", json!(2)),
    ];
    for (path, replacement) in mutations { let mut invalid = v.clone(); replace(&mut invalid, path, replacement); rejected(&invalid); }
    let mut subset = v.clone();
    subset["premises"][0] = json!({"id":"R.subset","kind":"delta_subset","relations":["R"],"source":"declared","dataset_hash":subset["dataset"]["hash"],"provenance":subset["relations"]["R"]["rows"]["provenance"]});
    subset["observations"]["rounds"][0]["entries"][1]["delta_rows"]["value"] = json!(2);
    subset["relations"]["R"]["recursive"]["unconsumed_final_delta_rows"]["value"] = json!(1);
    accepted(&subset); // Subset is not partition: an unconsumed last delta is legal.
    subset["premises"][0]["relations"] = json!(["*"]); rejected(&subset);
}

#[test]
fn stop_condition_mutations_never_publish_completed_evidence() {
    for stop in ["cancelled", "panic", "early-stop", "counter-overflow"] {
        let mut v = recursive();
        v["relations"]["R"]["recursive"]["run"]["completed"] = json!(false);
        v["relations"]["R"]["recursive"]["run"]["run_id"] = json!(stop);
        rejected(&v);
    }
    let mut v = recursive();
    v["observations"]["rounds"][0]["entries"][0]["delta_rows"]["value"] = json!(u64::MAX);
    rejected(&v);
}

#[test]
fn observed_steps_check_bound_identity_and_required_premises() {
    let mut v = recursive();
    v["observations"]["steps"] = json!([{
        "relation":"R","hole":"rule.head0","variant":"delta(R@0)","step":"scan.0", "run":v["relations"]["R"]["recursive"]["run"],
        "round":count(&v,"step.round","rounds",1), "rows":count(&v,"step.rows","rows",1),
        "bound":{"fact":"R.final","premises":["R.partition"],"dataset_hash":v["dataset"]["hash"],"program_hash":HASH,"variant":"delta(R@0)"}
    }]);
    accepted(&v);
    for (path, value) in [
        ("/observations/steps/0/bound/fact", json!("missing")),
        ("/observations/steps/0/bound/fact", json!("R.a.distinct")),
        ("/observations/steps/0/bound/fact", json!("step.rows")),
        ("/observations/steps/0/bound/dataset_hash", json!(HASH)),
        ("/observations/steps/0/bound/program_hash", json!("sha256:2222222222222222222222222222222222222222222222222222222222222222")),
        ("/observations/steps/0/bound/variant", json!("delta(R@1)")),
        ("/observations/steps/0/bound/premises", json!([])),
        ("/observations/steps/0/bound/premises", json!(["missing"])),
        ("/observations/steps/0/rows/value", json!(5)),
        ("/observations/steps/0/round/value", json!(0)),
    ] { let mut invalid = v.clone(); replace(&mut invalid, path, value); rejected(&invalid); }
}

#[test]
fn shared_recursive_fixture_and_machine_schema_are_versioned() {
    let evidence = parse_json(include_str!("fixtures/planner-recursive-evidence.json")).unwrap();
    assert_eq!(evidence.relations["R"].recursive.as_ref().unwrap().rounds.value, Some(2));
    let schema: Value = serde_json::from_str(include_str!("../../../doc/planner-evidence.schema.json")).unwrap();
    assert_eq!(schema["properties"]["schema"]["const"], cq_syntax::evidence::SCHEMA);
    assert_eq!(schema["additionalProperties"], false);
    assert!(schema["$defs"]["CountFact"]["allOf"].is_array());
}

fn add_second_relation(v: &mut Value) {
    fn rename_ids(value: &mut Value) {
        match value {
            Value::Object(map) => {
                if let Some(id) = map.get_mut("id") { *id = json!(format!("S.{}", id.as_str().unwrap())); }
                for value in map.values_mut() { rename_ids(value); }
            }
            Value::Array(values) => for value in values { rename_ids(value); },
            _ => {}
        }
    }
    let mut second = v["relations"]["R"].clone();
    rename_ids(&mut second);
    v["relations"]["S"] = second;
    v["dataset"]["manifest"]["schema"]["S"] = v["dataset"]["manifest"]["schema"]["R"].clone();
    rehash(v);
}

#[test]
fn multiple_relation_relative_paths_reject_cycles_and_incompatible_units() {
    let mut v = fixture();
    add_second_relation(&mut v);
    v["relative"] = json!([
        relative(&v, "R.le.S", "R", "S", "1", "1"),
        relative(&v, "S.le.R", "S", "R", "2", "1")
    ]);
    accepted(&v);
    v["relative"][1]["factor"]["value"] = json!({"numerator":"1","denominator":"2"});
    rejected(&v);
    v["relative"][1]["factor"]["value"] = json!({"numerator":"2","denominator":"1"});
    v["relative"][0]["factor"]["unit"] = json!("declared-row-work");
    rejected(&v);
    v["relative"][0]["factor"]["unit"] = json!("ratio");
    v["relative"][0]["factor"]["claim"] = json!("observation");
    rejected(&v);
    v["relative"][0]["factor"]["claim"] = json!("equal");
    v["relations"]["S"]["semantics"] = json!("set");
    v["dataset"]["manifest"]["schema"]["S"]["semantics"] = json!("set");
    v["dataset"]["manifest"]["schema"]["S"]["set_certification"] = json!("input_unique");
    rehash(&mut v);
    rejected(&v);
}

#[test]
fn stratum_round_and_input_run_scopes_cannot_conflict() {
    let mut v = recursive();
    add_second_relation(&mut v);
    accepted(&v);
    v["relations"]["S"]["recursive"]["rounds"]["value"] = json!(3);
    rejected(&v);
    v["relations"]["S"]["recursive"]["rounds"]["value"] = json!(2);
    v["relations"]["S"]["recursive"]["run"]["run_id"] = json!("other-run");
    rejected(&v);

    let mut v = fixture();
    let run = recursive()["relations"]["R"]["recursive"]["run"].clone();
    v["relations"]["R"]["rows"]["provenance"]["run"] = run.clone();
    v["relations"]["R"]["columns"]["a"]["distinct"]["provenance"]["run"] = run;
    accepted(&v);
    v["relations"]["R"]["columns"]["a"]["distinct"]["provenance"]["run"]["run_id"] = json!("other-run");
    rejected(&v);
}

#[test]
fn extrema_bounds_and_dictionary_mismatches_are_not_coerced() {
    let mut v = fixture();
    v["relations"]["R"]["columns"]["b"]["min"]["claim"] = json!("interval");
    v["relations"]["R"]["columns"]["b"]["min"]["value"] = Value::Null;
    v["relations"]["R"]["columns"]["b"]["min"]["lower"] = json!({"kind":"signed_integer","value":0});
    v["relations"]["R"]["columns"]["b"]["min"]["upper"] = json!({"kind":"signed_integer","value":2});
    accepted(&v);
    v["relations"]["R"]["columns"]["b"]["min"]["lower"]["value"] = json!(3);
    rejected(&v);

    let mut v = fixture();
    let domain = json!({"kind":"symbol","dictionary_hash":HASH,"ordered":true});
    v["dataset"]["manifest"]["schema"]["R"]["columns"]["b"] = domain.clone();
    v["relations"]["R"]["columns"]["b"]["domain"] = domain;
    v["relations"]["R"]["columns"]["b"]["min"]["value"] = json!({"kind":"symbol","code":1,"dictionary_hash":HASH});
    v["relations"]["R"]["columns"]["b"]["max"]["value"] = json!({"kind":"symbol","code":9,"dictionary_hash":HASH});
    rehash(&mut v); rejected(&v); // Unknown dictionary.
    v["dataset"]["manifest"]["dictionaries"]["symbols"] = json!(HASH);
    rehash(&mut v); accepted(&v);
    v["relations"]["R"]["columns"]["b"]["min"]["value"]["dictionary_hash"] = json!("sha256:2222222222222222222222222222222222222222222222222222222222222222");
    rejected(&v);
}

#[test]
fn checker_runs_after_direct_construction_and_canonical_serialization() {
    let mut document = parse_json(FIXTURE).unwrap();
    document.relations.get_mut("R").unwrap().rows.as_mut().unwrap().value = Some(0);
    assert!(document.check().is_err());
    assert!(document.canonical_json().is_err());
}

#[test]
fn nonrecursive_steps_are_run_scoped_without_steady_rounds() {
    let mut v = fixture();
    let run = recursive()["relations"]["R"]["recursive"]["run"].clone();
    let mut measured = count(&v, "cq.step.rows", "rows", 2);
    measured["source"] = json!("calibrated");
    measured["provenance"]["run"] = run.clone();
    v["observations"]["steps"] = json!([{
        "relation":"R", "hole":"query", "variant":"default", "step":"scan.0",
        "run":run, "round":null, "rows":measured,
        "bound":{"fact":"R.rows","premises":[],"dataset_hash":v["dataset"]["hash"],"program_hash":HASH,"variant":"default"}
    }]);
    accepted(&v);
    v["observations"]["steps"][0]["run"]["completed"] = json!(false);
    rejected(&v);
}

fn physical() -> Value {
    let mut v = recursive();
    let mut computed = rational(&v, "physical.computed", "rows", "1", "1");
    computed["source"] = json!("calibrated");
    computed["claim"] = json!("observation");
    v["observations"]["physical_steps"] = json!([{
        "hole":"hole:0", "variant":"variant:1/steady", "instance":"program:0:operator:2",
        "stratum":"s0", "trace_hash":HASH,
        "physical_steps":["r0","r1"], "vo_steps":["0","1"], "fused":true,
        "run":v["relations"]["R"]["recursive"]["run"],
        "round":count(&v,"physical.round","rounds",1),
        "rows":count(&v,"physical.rows","rows",2),
        "comparison": {
            "computed":computed, "fact_citations":["R.final","R.partition"],
            "compiler_citations":["compiler:recorded-VO-coverage"], "within_bound":false
        },
        "unknown":[]
    }]);
    v
}

#[test]
fn falsified_physical_comparisons_remain_observations_not_certificates() {
    let v = physical();
    let checked = accepted(&v);
    assert!(!checked.observations.physical_steps[0].comparison.as_ref().unwrap().within_bound);
    let facts = checked.planning_facts();
    facts.check().unwrap();
    assert_eq!(facts.dataset, checked.dataset);
    assert_eq!(facts.relations, checked.relations);
    assert_eq!(facts.observations.rounds, checked.observations.rounds);
    assert!(facts.observations.physical_steps.is_empty());
    for (path, value) in [
        ("/observations/physical_steps/0/comparison/within_bound", json!(true)),
        ("/observations/physical_steps/0/comparison/computed/source", json!("bound")),
        ("/observations/physical_steps/0/comparison/computed/claim", json!("upper")),
        ("/observations/physical_steps/0/comparison/fact_citations", json!(["physical.rows"])),
        ("/observations/physical_steps/0/comparison/fact_citations", json!(["delta.1"])),
        ("/observations/physical_steps/0/comparison/fact_citations", json!(["missing"])),
        ("/observations/physical_steps/0/comparison/computed/dataset_hash", json!(HASH)),
        ("/observations/physical_steps/0/comparison/computed/provenance/run/run_id", json!("other")),
        ("/observations/physical_steps/0/round/value", json!(0)),
        ("/observations/physical_steps/0/stratum", json!("other")),
        ("/observations/physical_steps/0/vo_steps", json!(["0","0"])),
        ("/observations/physical_steps/0/physical_steps", json!([])),
        ("/observations/physical_steps/0/fused", json!(false)),
        ("/observations/physical_steps/0/unknown", json!(["partial proof"])),
        ("/observations/physical_steps/0/comparison", Value::Null),
    ] {
        let mut invalid = v.clone(); replace(&mut invalid, path, value); rejected(&invalid);
    }
    let mut unknown = v;
    unknown["observations"]["physical_steps"][0]["comparison"] = Value::Null;
    unknown["observations"]["physical_steps"][0]["unknown"] = json!(["no finite nested-scope bound"]);
    accepted(&unknown);
}
